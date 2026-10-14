//! Settings commands. The only setting so far is the section shortcut.

use serde::{Deserialize, Serialize};
use skribe_domain::{Shortcut, ShortcutError};
use skribe_store::{LibraryService, Stored};

use crate::{CommandError, open_library};

/// Longest shortcut text accepted from the frontend; a real one is under 30 characters.
const MAX_SHORTCUT_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// The shortcut that advances to the next section while recording, such as
    /// `Ctrl+Shift+F9`.
    pub section_shortcut: String,
    pub default_section_shortcut: String,
    /// Quote this when changing the shortcut. Zero until it is first saved.
    pub section_shortcut_revision: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetSectionShortcutArgs {
    pub shortcut: String,
    pub expected_revision: u32,
}

fn settings(shortcut: Stored<Shortcut>) -> Settings {
    Settings {
        section_shortcut: shortcut.value.to_string(),
        default_section_shortcut: Shortcut::DEFAULT.to_string(),
        section_shortcut_revision: shortcut.revision,
    }
}

fn shortcut(text: &str) -> Result<Shortcut, CommandError> {
    if text.len() > MAX_SHORTCUT_LEN {
        return Err(CommandError::invalid_input("That is not a shortcut."));
    }
    text.parse().map_err(|error| {
        CommandError::invalid_input(&match error {
            ShortcutError::Empty => "Choose a shortcut.".to_owned(),
            ShortcutError::NoKey => {
                "Add a letter, digit or function key to the shortcut.".to_owned()
            }
            ShortcutError::NeedsModifier => {
                "The shortcut works while other apps have focus, so it must include Ctrl, Alt \
                 or the Windows key."
                    .to_owned()
            }
            ShortcutError::UnknownKey(key) => format!(
                "{key} cannot be used in the shortcut. Use Ctrl, Alt, Shift or the Windows key \
                 with a letter, a digit or F1 to F24."
            ),
            ShortcutError::Repeated(key) => {
                format!("{key} appears more than once in the shortcut.")
            }
        })
    })
}

/// `settings_get`
pub fn get(service: &LibraryService) -> Result<Settings, CommandError> {
    open_library(service)?
        .section_shortcut()
        .map(settings)
        .map_err(|error| CommandError::from(&error))
}

/// `settings_set_section_shortcut`: stores the shortcut in its canonical spelling.
pub fn set_section_shortcut(
    service: &LibraryService,
    args: &SetSectionShortcutArgs,
) -> Result<Settings, CommandError> {
    let chosen = shortcut(&args.shortcut)?;
    open_library(service)?
        .set_section_shortcut(args.expected_revision, chosen)
        .map(settings)
        .map_err(|error| CommandError::from(&error))
}

#[cfg(test)]
mod tests {
    use skribe_store::StorageRoots;
    use tempfile::TempDir;

    use super::*;
    use crate::ErrorCode;

    fn service() -> (TempDir, LibraryService) {
        let dir = tempfile::tempdir().expect("temporary folder");
        let roots = StorageRoots::under(dir.path()).expect("valid roots");
        (dir, LibraryService::new(roots, "0.0.0-test"))
    }

    fn set(
        service: &LibraryService,
        shortcut: &str,
        expected_revision: u32,
    ) -> Result<Settings, CommandError> {
        set_section_shortcut(
            service,
            &SetSectionShortcutArgs {
                shortcut: shortcut.to_owned(),
                expected_revision,
            },
        )
    }

    #[test]
    fn the_shortcut_starts_at_its_default_and_is_saved_in_canonical_form() {
        let (_dir, service) = service();
        let initial = get(&service).expect("settings");
        assert_eq!(initial.section_shortcut, "Ctrl+Shift+F9");
        assert_eq!(initial.default_section_shortcut, "Ctrl+Shift+F9");
        assert_eq!(initial.section_shortcut_revision, 0);

        let saved = set(&service, "alt + ctrl + n", 0).expect("save");

        assert_eq!(saved.section_shortcut, "Ctrl+Alt+N");
        assert_eq!(saved.section_shortcut_revision, 1);
        assert_eq!(get(&service), Ok(saved));
    }

    #[test]
    fn an_unusable_shortcut_is_explained_and_not_saved() {
        let (_dir, service) = service();

        for (typed, expected) in [
            ("", "Choose a shortcut."),
            (
                "Ctrl+Shift",
                "Add a letter, digit or function key to the shortcut.",
            ),
            ("Ctrl+N+N", "N appears more than once in the shortcut."),
        ] {
            let error = set(&service, typed, 0).expect_err("rejected");
            assert_eq!(error.code, ErrorCode::InvalidInput);
            assert_eq!(error.message, expected);
        }
        let error = set(&service, "Shift+N", 0).expect_err("rejected");
        assert!(error.message.contains("must include Ctrl, Alt"));
        let error = set(&service, "Ctrl+Enter", 0).expect_err("rejected");
        assert!(error.message.starts_with("Enter cannot be used"));
        let error = set(&service, &"x".repeat(MAX_SHORTCUT_LEN + 1), 0).expect_err("rejected");
        assert_eq!(error.message, "That is not a shortcut.");

        assert_eq!(
            get(&service).expect("settings").section_shortcut_revision,
            0
        );
    }

    #[test]
    fn a_stale_shortcut_change_is_a_conflict() {
        let (_dir, service) = service();
        set(&service, "Ctrl+Alt+N", 0).expect("save");

        let error = set(&service, "Ctrl+Alt+M", 0).expect_err("conflict");

        assert_eq!(error.code, ErrorCode::RevisionConflict);
        assert_eq!(
            get(&service).expect("settings").section_shortcut,
            "Ctrl+Alt+N"
        );
    }
}
