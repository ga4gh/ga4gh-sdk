//! Typed JSON persistence with atomic replacement and an explicit writer lock.
//!
//! Reads never create files. Hold [`StoreLock`] across a read–modify–write
//! operation, or use [`JsonStore::update`], to avoid losing another writer's
//! changes. All cooperating writers must use the same store path.
//! The store directory and its ancestors must be trusted: another user with
//! directory write access can replace the lock file and bypass cooperation.
//! File modes and advisory locks do not protect against such interference.
//!
//! This interface is public so the separate CLI crate can reuse SDK persistence
//! without making the SDK depend on CLI authentication or application paths.
//! Its current production consumer is the CLI token store; Extensions Manager
//! is a future consumer tracked separately in issue #64. The shared invariant
//! is typed local JSON with atomic replacement and cooperating writer locks.
//! Paths, schemas, missing-file defaults and access policies belong to callers;
//! this does not establish an extension registry format or a database API.

use serde::{de::DeserializeOwned, Serialize};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// A filesystem failure with its original cause and replacement status.
///
/// Inspect this through [`crate::Error::downcast_ref`] when deciding how to
/// handle a failed write or update. Default formatting omits cause text.
pub struct JsonStoreIoError {
    context: String,
    cause: io::Error,
    replacement_completed: bool,
}

impl JsonStoreIoError {
    /// Whether the destination was replaced before the failure occurred.
    ///
    /// When true, do not repeat a non-idempotent update: its change is already
    /// visible. Inspect the saved state and handle the durability failure
    /// separately. False means no successful replacement was recorded by this
    /// operation; it does not undo side effects of a caller's change callback.
    pub fn replacement_completed(&self) -> bool {
        self.replacement_completed
    }
}

impl fmt::Display for JsonStoreIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.context, self.cause.kind())
    }
}

impl fmt::Debug for JsonStoreIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for JsonStoreIoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

fn io_error(context: String, cause: io::Error) -> crate::Error {
    JsonStoreIoError {
        context,
        cause,
        replacement_completed: false,
    }
    .into()
}

/// A JSON file whose location and access policy are chosen by the caller.
///
/// ```no_run
/// use ga4gh_sdk::utils::json_store::JsonStore;
/// # fn main() -> ga4gh_sdk::Result<()> {
/// let store = JsonStore::new("my-app/state.json", "application state");
/// store.update::<Vec<String>, _>(|items| {
///     items.push("new item".into());
///     Ok(())
/// })?;
/// let items = store.read::<Vec<String>>()?;
/// # let _ = items;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct JsonStore {
    path: PathBuf,
    label: &'static str,
    private: bool,
}

impl JsonStore {
    /// Describes a store without reading it or creating files.
    /// `label` identifies its purpose in errors, for example `"token store"`.
    pub fn new(path: impl Into<PathBuf>, label: &'static str) -> Self {
        Self {
            path: path.into(),
            label,
            private: false,
        }
    }

    /// On Unix, reads and writes reject existing data files accessible by other
    /// users, and acquiring a lock rejects broadly accessible lock files.
    /// Creates new parent directories with mode `0700`. New files use mode
    /// `0600` on Unix in either policy. Existing parent directory permissions
    /// are unchanged; callers must ensure the directory and its ancestors
    /// cannot be modified by untrusted users.
    pub fn private(mut self) -> Self {
        self.private = true;
        self
    }

    /// The caller-supplied location of the store.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads typed data, returning `None` for a missing file.
    ///
    /// # Errors
    ///
    /// Returns I/O, permission-policy or malformed-data errors. Parser messages
    /// and values are omitted because they may contain credentials.
    pub fn read<T: DeserializeOwned>(&self) -> crate::Result<Option<T>> {
        let mut file = match fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(io_error(
                    format!("Cannot read {} {}", self.label, self.path.display()),
                    error,
                ))
            }
        };
        let metadata = file.metadata().map_err(|error| {
            io_error(
                format!("Cannot inspect {} {}", self.label, self.path.display()),
                error,
            )
        })?;
        if metadata.is_dir() {
            return Err(io_error(
                format!("Cannot read {} {}", self.label, self.path.display()),
                io::ErrorKind::IsADirectory.into(),
            ));
        }
        #[cfg(unix)]
        self.check_private_permissions(&metadata, &self.path)?;
        // Read separately so an I/O failure is not reported as invalid JSON.
        let mut contents = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut contents).map_err(|error| {
            io_error(
                format!("Cannot read {} {}", self.label, self.path.display()),
                error,
            )
        })?;
        serde_json::from_slice(&contents)
            .map(Some)
            .map_err(|_| format!("{} {} is malformed", self.label, self.path.display()).into())
    }

    /// Writes through a private temporary file in the same directory, syncs it,
    /// and atomically replaces the destination. Readers see a complete file.
    /// On Unix, also syncs the parent directory after replacement. Other
    /// platforms do not sync directories. Power-loss durability still depends
    /// on the filesystem; newly created ancestor directories are not synced.
    ///
    /// # Errors
    ///
    /// Returns directory, serialization or write errors. A failure before
    /// replacement leaves the previous destination untouched. A directory-sync
    /// error happens after replacement, so the new file may already be visible.
    /// [`JsonStoreIoError::replacement_completed`] identifies this case. Do not
    /// blindly retry a non-idempotent update after an error.
    /// I/O causes remain inspectable through [`crate::Error::downcast_ref`].
    /// Serialization errors omit custom messages, which can contain values.
    pub fn write<T: Serialize>(&self, value: &T) -> crate::Result<()> {
        self.write_with_directory_sync(value, fs::File::sync_all)
    }

    fn write_with_directory_sync<T: Serialize>(
        &self,
        value: &T,
        sync_directory: impl FnOnce(&fs::File) -> io::Result<()>,
    ) -> crate::Result<()> {
        self.create_parent()?;
        let writing = || format!("Cannot write {} {}", self.label, self.path.display());
        #[cfg(unix)]
        if self.private {
            match fs::metadata(&self.path) {
                Ok(metadata) if metadata.is_file() => {
                    self.check_private_permissions(&metadata, &self.path)?;
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(io_error(writing(), error)),
            }
        }
        #[cfg(unix)]
        let directory = fs::File::open(self.parent()).map_err(|error| {
            io_error(
                format!("Cannot open directory {}", self.parent().display()),
                error,
            )
        })?;
        let mut file = tempfile::NamedTempFile::new_in(self.parent())
            .map_err(|error| io_error(writing(), error))?;
        serde_json::to_writer_pretty(&mut file, value).map_err(|error| {
            if error.is_io() {
                io_error(writing(), error.into())
            } else {
                format!("Cannot serialize {}", self.label).into()
            }
        })?;
        file.write_all(b"\n")
            .map_err(|error| io_error(writing(), error))?;
        file.as_file()
            .sync_all()
            .map_err(|error| io_error(writing(), error))?;
        file.persist(&self.path)
            .map_err(|error| io_error(writing(), error.error))?;
        #[cfg(unix)]
        sync_directory(&directory).map_err(|cause| {
            crate::Error::from(JsonStoreIoError {
                context: format!(
                    "Cannot sync directory {} after writing {} {}",
                    self.parent().display(),
                    self.label,
                    self.path.display()
                ),
                cause,
                replacement_completed: true,
            })
        })?;
        #[cfg(not(unix))]
        let _ = sync_directory;
        Ok(())
    }

    /// Acquires an exclusive lock on `<path>.lock`, waiting for other writers.
    /// The separate lock file stays in place when the data file is replaced.
    ///
    /// # Errors
    ///
    /// Returns an error if the lock directory or file cannot be created or locked.
    pub fn lock(&self) -> crate::Result<StoreLock> {
        self.create_parent()?;
        let mut name = self.path.as_os_str().to_owned();
        name.push(".lock");
        let path = PathBuf::from(name);
        // Give a recovery hint even if opening a shared read-only lock fails.
        // The descriptor check below remains authoritative after opening.
        #[cfg(unix)]
        if self.private {
            if let Ok(metadata) = fs::metadata(&path) {
                if metadata.is_file() {
                    self.check_private_permissions(&metadata, &path)?;
                }
            }
        }
        let mut options = fs::OpenOptions::new();
        options.create(true).truncate(false).write(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options
            .open(&path)
            .map_err(|error| io_error(format!("Cannot open {}", path.display()), error))?;
        #[cfg(unix)]
        self.check_private_permissions(
            &file
                .metadata()
                .map_err(|error| io_error(format!("Cannot inspect {}", path.display()), error))?,
            &path,
        )?;
        file.lock()
            .map_err(|error| io_error(format!("Cannot lock {}", path.display()), error))?;
        Ok(StoreLock { _file: file })
    }

    /// Reloads the latest data under the writer lock, applies `change`, and
    /// persists it. Missing data starts at `T::default()`.
    /// Convert application errors with `map_err(crate::Error::other)` inside
    /// the callback; the original type remains available through
    /// [`crate::Error::downcast_ref`].
    ///
    /// # Errors
    ///
    /// Returns lock, read, change or write errors. A failed change is not saved.
    /// A directory-sync failure can follow a completed replacement: inspect
    /// [`JsonStoreIoError::replacement_completed`] instead of automatically
    /// applying the change again. Callback side effects are not rolled back.
    pub fn update<T: DeserializeOwned + Serialize + Default, R>(
        &self,
        change: impl FnOnce(&mut T) -> crate::Result<R>,
    ) -> crate::Result<R> {
        let _lock = self.lock()?;
        let mut value = self.read()?.unwrap_or_default();
        let result = change(&mut value)?;
        self.write(&value)?;
        Ok(result)
    }

    fn parent(&self) -> &Path {
        self.path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
    }

    #[cfg(unix)]
    fn check_private_permissions(&self, metadata: &fs::Metadata, path: &Path) -> crate::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        if self.private && metadata.permissions().mode() & 0o077 != 0 {
            return Err(format!(
                "{} {} is accessible to other users; run 'chmod 600' on it",
                self.label,
                path.display()
            )
            .into());
        }
        Ok(())
    }

    fn create_parent(&self) -> crate::Result<()> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        if self.private {
            std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        }
        builder.create(self.parent()).map_err(|error| {
            io_error(
                format!("Cannot create directory {}", self.parent().display()),
                error,
            )
        })
    }
}

/// An exclusive store lock, released when dropped. It contains no parsed data.
#[derive(Debug)]
pub struct StoreLock {
    _file: fs::File,
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_directory_sync_failure_reports_the_completed_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::new(dir.path().join("state.json"), "test state");
        store.write(&vec!["old"]).unwrap();
        let error = store
            .write_with_directory_sync(&vec!["new"], |_| Err(io::Error::other("SENTINEL_IO")))
            .unwrap_err();
        assert_eq!(
            store.read::<Vec<String>>().unwrap(),
            Some(vec!["new".into()])
        );
        assert!(error
            .downcast_ref::<JsonStoreIoError>()
            .unwrap()
            .replacement_completed());
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().to_string(),
            "SENTINEL_IO"
        );
        assert!(!format!("{error} {error:?}").contains("SENTINEL_IO"));
    }
}
