//! Typed JSON persistence with atomic replacement and an explicit writer lock.
//!
//! Reads never create files. Hold [`StoreLock`] across a read–modify–write
//! operation, or use [`JsonStore::update`], to avoid losing another writer's
//! changes. All cooperating writers must use the same store path.

use serde::{de::DeserializeOwned, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// A JSON file whose location and access policy are chosen by the caller.
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

    /// Rejects files accessible by other users on Unix and creates new parent
    /// directories with mode `0700`. New files use mode `0600` on Unix in
    /// either policy. Existing parent directory permissions are unchanged.
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
                return Err(format!(
                    "Cannot read {} {}: {error}",
                    self.label,
                    self.path.display()
                )
                .into())
            }
        };
        let metadata = file.metadata()?;
        if metadata.is_dir() {
            return Err(format!(
                "Cannot read {} {}: path is a directory",
                self.label,
                self.path.display()
            )
            .into());
        }
        #[cfg(unix)]
        if self.private {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(format!(
                    "{} {} is accessible to other users; run 'chmod 600' on it",
                    self.label,
                    self.path.display()
                )
                .into());
            }
        }
        // Read separately so an I/O failure is not reported as invalid JSON.
        let mut contents = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut contents).map_err(|error| {
            format!(
                "Cannot read {} {}: {error}",
                self.label,
                self.path.display()
            )
        })?;
        serde_json::from_slice(&contents)
            .map(Some)
            .map_err(|_| format!("{} {} is malformed", self.label, self.path.display()).into())
    }

    /// Writes through a private temporary file in the same directory, syncs it,
    /// and atomically replaces the destination. Readers see a complete file.
    ///
    /// # Errors
    ///
    /// Returns directory, serialization or write errors. A failure before
    /// replacement leaves the previous destination untouched. Serialization
    /// errors omit custom messages, which can contain values.
    pub fn write<T: Serialize>(&self, value: &T) -> crate::Result<()> {
        self.create_parent()?;
        let mut file = tempfile::NamedTempFile::new_in(self.parent())?;
        serde_json::to_writer_pretty(&mut file, value)
            .map_err(|_| format!("Cannot serialize {}", self.label))?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(&self.path).map_err(|error| {
            format!(
                "Cannot write {} {}: {}",
                self.label,
                self.path.display(),
                error.error
            )
        })?;
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
        let mut options = fs::OpenOptions::new();
        options.create(true).truncate(false).write(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options
            .open(&path)
            .map_err(|error| format!("Cannot open {}: {error}", path.display()))?;
        file.lock()
            .map_err(|error| format!("Cannot lock {}: {error}", path.display()))?;
        Ok(StoreLock { _file: file })
    }

    /// Reloads the latest data under the writer lock, applies `change`, and
    /// persists it. Missing data starts at `T::default()`.
    ///
    /// # Errors
    ///
    /// Returns lock, read, change or write errors. A failed change is not saved.
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

    fn create_parent(&self) -> crate::Result<()> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        if self.private {
            std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        }
        builder.create(self.parent()).map_err(|error| {
            format!(
                "Cannot create directory {}: {error}",
                self.parent().display()
            )
            .into()
        })
    }
}

/// An exclusive store lock, released when dropped. It contains no parsed data.
#[derive(Debug)]
pub struct StoreLock {
    _file: fs::File,
}
