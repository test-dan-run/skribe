use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

use crate::StoreError;

/// Exclusive ownership of a library folder for as long as the value lives. A second process
/// (or a second open in this one) is refused rather than left to migrate or write concurrently.
#[derive(Debug)]
pub(crate) struct LibraryLock {
    _file: File,
}

impl LibraryLock {
    pub(crate) fn acquire(path: &Path) -> Result<Self, StoreError> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .map_err(StoreError::io("open", path))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => {
                let library = path.parent().unwrap_or(path).to_owned();
                Err(StoreError::LibraryLocked { path: library })
            }
            Err(TryLockError::Error(source)) => Err(StoreError::Io {
                action: "lock",
                path: path.to_owned(),
                source,
            }),
        }
    }
}
