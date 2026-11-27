//! Complete updates, concurrent writers, and private persistence policies.

use ga4gh_sdk::utils::json_store::JsonStore;
use serde::{Serialize, Serializer};
use std::fs;

#[test]
fn failed_changes_and_serialization_leave_previous_data_intact() {
    let dir = tempfile::tempdir().unwrap();
    let store = JsonStore::new(dir.path().join("nested/state.json"), "test state");
    assert!(store.read::<Vec<String>>().unwrap().is_none());
    assert!(!store.path().parent().unwrap().exists());
    store.write(&vec!["original"]).unwrap();
    let before = fs::read(store.path()).unwrap();
    let result: ga4gh_sdk::Result<()> = store.update::<Vec<String>, _>(|value| {
        value.push("discarded".into());
        Err("Change rejected".into())
    });
    assert!(result.is_err());
    assert_eq!(fs::read(store.path()).unwrap(), before);

    struct Invalid;
    impl Serialize for Invalid {
        fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("SENTINEL_SECRET"))
        }
    }
    let error = store.write(&Invalid).unwrap_err();
    assert!(!format!("{error} {error:?}").contains("SENTINEL"));
    assert_eq!(fs::read(store.path()).unwrap(), before);
    assert_eq!(
        fs::read_dir(store.path().parent().unwrap())
            .unwrap()
            .count(),
        2
    );
}

#[test]
fn update_holds_an_exclusive_lock_during_the_change() {
    let dir = tempfile::tempdir().unwrap();
    let store = JsonStore::new(dir.path().join("counter.json"), "counter");
    // Create the documented lock file, then open an independent contender.
    drop(store.lock().unwrap());
    let contender = fs::OpenOptions::new()
        .write(true)
        .open(dir.path().join("counter.json.lock"))
        .unwrap();
    store
        .update::<u64, _>(|counter| {
            // This probe must contend even if all worker threads happen to
            // run sequentially. Removing update's lock must fail this test.
            assert!(matches!(
                contender.try_lock(),
                Err(fs::TryLockError::WouldBlock)
            ));
            *counter += 1;
            Ok(())
        })
        .unwrap();
    contender.try_lock().expect("update must release its lock");
    assert_eq!(store.read::<u64>().unwrap(), Some(1));
}

#[test]
fn independent_writers_reload_under_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("counter.json");
    let barrier = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let barrier = &barrier;
            let store = JsonStore::new(&path, "counter");
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..8 {
                    store
                        .update::<u64, _>(|counter| {
                            *counter += 1;
                            Ok(())
                        })
                        .unwrap();
                }
            });
        }
    });
    assert_eq!(
        JsonStore::new(path, "counter").read::<u64>().unwrap(),
        Some(64)
    );
}

#[test]
fn malformed_data_is_not_overwritten_or_echoed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let bytes = br#"{"secret":"SENTINEL_SECRET"}"#;
    fs::write(&path, bytes).unwrap();
    let store = JsonStore::new(&path, "test state");
    let error = store.update::<Vec<String>, _>(|_| Ok(())).unwrap_err();
    assert!(error.to_string().contains("state.json"));
    assert!(!format!("{error} {error:?}").contains("SENTINEL"));
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn io_errors_keep_the_original_cause_and_operation_context() {
    use std::error::Error;
    use std::io;

    let dir = tempfile::tempdir().unwrap();
    let store = JsonStore::new(dir.path().join("invalid\0.json"), "test state");
    for (error, operation) in [
        (store.read::<u64>().unwrap_err(), "Cannot read test state"),
        (store.write(&1).unwrap_err(), "Cannot write test state"),
        (store.lock().unwrap_err(), "Cannot open"),
    ] {
        assert!(error.to_string().contains(operation), "{error}");
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            error
                .source()
                .unwrap()
                .downcast_ref::<io::Error>()
                .unwrap()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    let blocked = dir.path().join("blocked");
    fs::write(&blocked, "existing file").unwrap();
    let expected = fs::create_dir_all(&blocked).unwrap_err().kind();
    let error = JsonStore::new(blocked.join("state.json"), "test state")
        .write(&1)
        .unwrap_err();
    assert!(error.to_string().starts_with("Cannot create directory"));
    assert_eq!(error.downcast_ref::<io::Error>().unwrap().kind(), expected);
    assert_eq!(fs::read_to_string(blocked).unwrap(), "existing file");
}

#[cfg(unix)]
#[test]
fn private_stores_reject_an_existing_shared_lock_file() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let store = JsonStore::new(dir.path().join("state.json"), "test state").private();
    store.write(&7).unwrap();
    let path = dir.path().join("state.json.lock");
    fs::write(&path, "existing lock file").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

    let error = store.lock().unwrap_err();
    assert!(error.to_string().contains("state.json.lock"));
    assert!(error.to_string().contains("chmod 600"));
    let result = store.update::<u64, ()>(|_| panic!("insecure lock must reject the update"));
    assert!(result.is_err());
    assert_eq!(store.read::<u64>().unwrap(), Some(7));
    assert_eq!(fs::read_to_string(&path).unwrap(), "existing lock file");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644
    );

    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    store
        .update::<u64, _>(|value| {
            *value += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(store.read::<u64>().unwrap(), Some(8));
}
