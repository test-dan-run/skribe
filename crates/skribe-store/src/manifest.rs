use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use skribe_domain::LibraryId;

use crate::StoreError;
use crate::fsutil;

/// Version of the library folder layout this build reads and writes.
pub const LIBRARY_FORMAT_VERSION: u32 = 1;

/// `library.json` is a few hundred bytes; anything much larger is not ours.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// The contents of `library.json`: which library a folder is, and what wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryManifest {
    pub library_id: LibraryId,
    pub format_version: u32,
    pub created_by_app_version: String,
}

#[derive(Serialize, Deserialize)]
struct ManifestFile {
    library_id: String,
    format_version: u32,
    created_by_app_version: String,
}

impl LibraryManifest {
    /// Writes the manifest for a new library, then reads it back so a library is only ever
    /// created with an identity that is known to be on disk.
    pub(crate) fn create(path: &Path, app_version: &str) -> Result<Self, StoreError> {
        let manifest = Self {
            library_id: LibraryId::generate(),
            format_version: LIBRARY_FORMAT_VERSION,
            created_by_app_version: app_version.to_owned(),
        };
        let file = ManifestFile {
            library_id: manifest.library_id.to_string(),
            format_version: manifest.format_version,
            created_by_app_version: manifest.created_by_app_version.clone(),
        };
        let mut json = serde_json::to_vec_pretty(&file).map_err(|error| invalid(path, error))?;
        json.push(b'\n');
        fsutil::write_atomic(path, &json)?;

        let written = Self::load(path)?;
        if written != manifest {
            return Err(invalid(
                path,
                "the file read back differs from what was written",
            ));
        }
        Ok(manifest)
    }

    pub(crate) fn load(path: &Path) -> Result<Self, StoreError> {
        let size = fs::metadata(path)
            .map_err(StoreError::io("read", path))?
            .len();
        if size > MAX_MANIFEST_BYTES {
            return Err(invalid(
                path,
                format!("it is {size} bytes, far larger than a manifest"),
            ));
        }
        let bytes = fs::read(path).map_err(StoreError::io("read", path))?;
        let file: ManifestFile =
            serde_json::from_slice(&bytes).map_err(|error| invalid(path, error))?;
        let library_id = file
            .library_id
            .parse::<LibraryId>()
            .map_err(|error| invalid(path, format!("library_id is {error}")))?;
        if file.format_version == 0 {
            return Err(invalid(path, "format_version is 0"));
        }
        if file.format_version > LIBRARY_FORMAT_VERSION {
            return Err(StoreError::LibraryFormatTooNew {
                found: file.format_version,
                supported: LIBRARY_FORMAT_VERSION,
            });
        }
        Ok(Self {
            library_id,
            format_version: file.format_version,
            created_by_app_version: file.created_by_app_version,
        })
    }
}

fn invalid(path: &Path, reason: impl ToString) -> StoreError {
    StoreError::ManifestInvalid {
        path: path.to_owned(),
        reason: reason.to_string(),
    }
}
