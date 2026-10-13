use std::fs;
use std::path::{Path, PathBuf};

use crate::StoreError;
use crate::fsutil;

/// The three storage roots: meeting library, model packages and scratch space.
///
/// They are kept apart so that meeting data, models and temporary files can be backed up,
/// cleared or removed independently. See `docs/storage.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageRoots {
    library: PathBuf,
    models: PathBuf,
    temp: PathBuf,
}

impl StorageRoots {
    /// The default layout: three sibling folders under the app's local data folder.
    pub fn under(base: &Path) -> Result<Self, StoreError> {
        Self::new(base.join("library"), base.join("models"), base.join("temp"))
    }

    pub fn new(library: PathBuf, models: PathBuf, temp: PathBuf) -> Result<Self, StoreError> {
        let named = [
            ("library", &library),
            ("models", &models),
            ("temporary", &temp),
        ];
        for (name, path) in named {
            if !path.is_absolute() {
                return Err(invalid(format!(
                    "the {name} folder {} is not absolute",
                    path.display()
                )));
            }
            // SQLite takes paths as UTF-8 text.
            if path.to_str().is_none() {
                return Err(invalid(format!(
                    "the {name} folder {} is not valid Unicode",
                    path.display()
                )));
            }
        }
        for (index, (name, path)) in named.iter().enumerate() {
            for (other_name, other) in &named[index + 1..] {
                if path.starts_with(other) || other.starts_with(path) {
                    return Err(invalid(format!(
                        "the {name} folder {} and the {other_name} folder {} overlap",
                        path.display(),
                        other.display()
                    )));
                }
            }
        }
        Ok(Self {
            library,
            models,
            temp,
        })
    }

    /// Rejects roots inside the install folder, which an update or uninstall may replace.
    pub fn ensure_outside(&self, install_dir: &Path) -> Result<(), StoreError> {
        for (name, path) in [
            ("library", &self.library),
            ("models", &self.models),
            ("temporary", &self.temp),
        ] {
            if path.starts_with(install_dir) {
                return Err(invalid(format!(
                    "the {name} folder {} is inside the install folder {}",
                    path.display(),
                    install_dir.display()
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn create(&self) -> Result<(), StoreError> {
        let folders = [
            self.library.clone(),
            self.db_dir(),
            self.audio_dir(),
            self.derived_dir(),
            self.backups_dir(),
            self.trash_dir(),
            self.models.clone(),
            self.temp.clone(),
        ];
        for folder in folders {
            fs::create_dir_all(&folder).map_err(StoreError::io("create", &folder))?;
        }
        Ok(())
    }

    /// Whether the library holds anything a fresh database would hide.
    pub(crate) fn library_has_content(&self) -> Result<bool, StoreError> {
        for folder in [
            self.audio_dir(),
            self.derived_dir(),
            self.backups_dir(),
            self.trash_dir(),
        ] {
            if fsutil::has_entries(&folder)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn library(&self) -> &Path {
        &self.library
    }

    pub fn models(&self) -> &Path {
        &self.models
    }

    pub fn temp(&self) -> &Path {
        &self.temp
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.library.join("backups")
    }

    pub(crate) fn manifest_path(&self) -> PathBuf {
        self.library.join("library.json")
    }

    pub(crate) fn lock_path(&self) -> PathBuf {
        self.library.join("library.lock")
    }

    pub(crate) fn db_dir(&self) -> PathBuf {
        self.library.join("db")
    }

    pub(crate) fn db_path(&self) -> PathBuf {
        self.db_dir().join("skribe.sqlite")
    }

    pub(crate) fn migration_marker_path(&self) -> PathBuf {
        self.db_dir().join("migration-in-progress.json")
    }

    pub(crate) fn audio_dir(&self) -> PathBuf {
        self.library.join("audio")
    }

    pub(crate) fn derived_dir(&self) -> PathBuf {
        self.library.join("derived")
    }

    pub(crate) fn trash_dir(&self) -> PathBuf {
        self.library.join("trash")
    }
}

fn invalid(reason: String) -> StoreError {
    StoreError::InvalidRoots { reason }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> PathBuf {
        std::env::temp_dir().join("skribe-roots-test")
    }

    #[test]
    fn default_layout_is_three_siblings() {
        let roots = StorageRoots::under(&base()).expect("valid roots");
        assert_eq!(roots.library(), base().join("library"));
        assert_eq!(roots.models(), base().join("models"));
        assert_eq!(roots.temp(), base().join("temp"));
    }

    #[test]
    fn rejects_relative_roots() {
        let result = StorageRoots::under(Path::new("relative"));
        assert!(matches!(result, Err(StoreError::InvalidRoots { .. })));
    }

    #[test]
    fn rejects_nested_or_shared_roots() {
        let library = base().join("library");
        for (models, temp) in [
            (library.join("models"), base().join("temp")),
            (base().join("models"), library.join("temp")),
            (base().join("models"), base().join("models")),
            (base(), base().join("temp")),
        ] {
            let result = StorageRoots::new(library.clone(), models, temp);
            assert!(matches!(result, Err(StoreError::InvalidRoots { .. })));
        }
    }

    #[test]
    fn rejects_roots_inside_the_install_folder() {
        let roots = StorageRoots::under(&base().join("app").join("data")).expect("valid roots");
        assert!(roots.ensure_outside(&base().join("app")).is_err());
        assert!(roots.ensure_outside(&base().join("elsewhere")).is_ok());
    }
}
