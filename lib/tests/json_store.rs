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
