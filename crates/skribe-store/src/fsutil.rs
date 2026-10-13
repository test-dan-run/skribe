use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::StoreError;

pub(crate) fn now_unix_ms() -> u64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

pub(crate) fn exists(path: &Path) -> Result<bool, StoreError> {
    path.try_exists().map_err(StoreError::io("check", path))
}

pub(crate) fn has_entries(dir: &Path) -> Result<bool, StoreError> {
    let mut entries = fs::read_dir(dir).map_err(StoreError::io("list", dir))?;
    Ok(entries.next().is_some())
}

/// `path` with `suffix` appended to its file name, as SQLite names its side files.
pub(crate) fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Flushes a file's contents to the device.
pub(crate) fn sync_file(path: &Path) -> Result<(), StoreError> {
    // Windows only flushes handles opened for writing.
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(StoreError::io("open", path))?;
    file.sync_all().map_err(StoreError::io("flush", path))
}

/// Flushes a directory entry change (create, rename, remove). Windows has no equivalent for
/// directory handles and persists renames with the file system journal.
pub(crate) fn sync_dir(dir: &Path) -> Result<(), StoreError> {
    if cfg!(unix) {
        let handle = File::open(dir).map_err(StoreError::io("open", dir))?;
        handle.sync_all().map_err(StoreError::io("flush", dir))?;
    }
    Ok(())
}

/// Replaces `path` with `contents` so a crash leaves either the old file or the new one.
pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), StoreError> {
    let staged = with_suffix(path, ".tmp");
    let mut file = File::create(&staged).map_err(StoreError::io("create", &staged))?;
    file.write_all(contents)
        .map_err(StoreError::io("write", &staged))?;
    file.sync_all().map_err(StoreError::io("flush", &staged))?;
    drop(file);
    fs::rename(&staged, path).map_err(StoreError::io("replace", path))?;
    match path.parent() {
        Some(parent) => sync_dir(parent),
        None => Ok(()),
    }
}
