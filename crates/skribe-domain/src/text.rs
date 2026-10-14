//! Limits and normalisation for short text the user types: meeting and agenda titles.

use std::fmt;

/// Longest meeting, agenda item or section title, in characters.
pub const MAX_TITLE_CHARS: usize = 200;
/// Most agenda items one meeting can hold.
pub const MAX_AGENDA_ITEMS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleError {
    Empty,
    TooLong {
        max: usize,
    },
    /// Line breaks, tabs and other control characters do not belong in a one-line title.
    ControlCharacter,
}

impl fmt::Display for TitleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("the title is empty"),
            Self::TooLong { max } => write!(f, "the title is longer than {max} characters"),
            Self::ControlCharacter => f.write_str("the title contains a control character"),
        }
    }
}

impl std::error::Error for TitleError {}

/// A one-line title that has passed the checks, so storage never receives an unchecked one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Title(String);

impl Title {
    /// Trims surrounding white space and checks what is left. Nothing else is rewritten: the
    /// title is stored as the user typed it.
    pub fn parse(text: &str) -> Result<Self, TitleError> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(TitleError::Empty);
        }
        if trimmed.chars().any(char::is_control) {
            return Err(TitleError::ControlCharacter);
        }
        if trimmed.chars().count() > MAX_TITLE_CHARS {
            return Err(TitleError::TooLong {
                max: MAX_TITLE_CHARS,
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Result<String, TitleError> {
        Title::parse(text).map(|title| title.as_str().to_owned())
    }

    #[test]
    fn trims_and_keeps_the_text_as_typed() {
        assert_eq!(
            parsed("  Budget  review — Q3 ").as_deref(),
            Ok("Budget  review — Q3")
        );
    }

    #[test]
    fn rejects_empty_control_and_overlong_titles() {
        assert_eq!(parsed(""), Err(TitleError::Empty));
        assert_eq!(parsed(" \t\n "), Err(TitleError::Empty));
        assert_eq!(parsed("a\nb"), Err(TitleError::ControlCharacter));
        assert_eq!(parsed("a\u{0}b"), Err(TitleError::ControlCharacter));
        let long = "é".repeat(MAX_TITLE_CHARS + 1);
        assert_eq!(
            parsed(&long),
            Err(TitleError::TooLong {
                max: MAX_TITLE_CHARS
            })
        );
    }

    #[test]
    fn the_limit_counts_characters_not_bytes() {
        let exactly = "é".repeat(MAX_TITLE_CHARS);
        assert_eq!(parsed(&exactly).as_deref(), Ok(exactly.as_str()));
    }
}
