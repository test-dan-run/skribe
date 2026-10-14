//! The global keyboard shortcut that advances to the next section while recording.

use std::fmt;
use std::str::FromStr;

/// The key a shortcut ends with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShortcutKey {
    /// `A` to `Z`.
    Letter(char),
    /// `0` to `9`.
    Digit(char),
    /// `F1` to `F24`.
    Function(u8),
}

/// A key combination in one canonical spelling, such as `Ctrl+Shift+F9`: modifiers in the
/// order Ctrl, Alt, Shift, Super, then the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// The Windows key.
    pub super_key: bool,
    pub key: ShortcutKey,
}

impl Shortcut {
    /// Used until the user chooses another.
    pub const DEFAULT: Self = Self {
        ctrl: true,
        alt: false,
        shift: true,
        super_key: false,
        key: ShortcutKey::Function(9),
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutError {
    Empty,
    UnknownKey(String),
    /// More than one non-modifier key, or the same modifier twice.
    Repeated(String),
    NoKey,
    /// The shortcut is global, so Shift alone or no modifier would capture ordinary typing.
    NeedsModifier,
}

impl fmt::Display for ShortcutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("the shortcut is empty"),
            Self::UnknownKey(key) => write!(f, "{key} is not a key a shortcut can use"),
            Self::Repeated(key) => write!(f, "{key} appears more than once"),
            Self::NoKey => f.write_str("the shortcut has modifiers but no key"),
            Self::NeedsModifier => {
                f.write_str("the shortcut must include Ctrl, Alt or the Windows key")
            }
        }
    }
}

impl std::error::Error for ShortcutError {}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (held, name) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.super_key, "Super"),
        ] {
            if held {
                write!(f, "{name}+")?;
            }
        }
        match self.key {
            ShortcutKey::Letter(key) | ShortcutKey::Digit(key) => write!(f, "{key}"),
            ShortcutKey::Function(number) => write!(f, "F{number}"),
        }
    }
}

impl FromStr for Shortcut {
    type Err = ShortcutError;

    /// Accepts the parts in any order and any letter case; [`fmt::Display`] prints the one
    /// canonical form.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.trim().is_empty() {
            return Err(ShortcutError::Empty);
        }
        let (mut ctrl, mut alt, mut shift, mut super_key) = (false, false, false, false);
        let mut key = None;
        for part in text.split('+') {
            let part = part.trim();
            let upper = part.to_ascii_uppercase();
            let modifier = match upper.as_str() {
                "CTRL" | "CONTROL" => Some(&mut ctrl),
                "ALT" => Some(&mut alt),
                "SHIFT" => Some(&mut shift),
                "SUPER" | "WIN" | "META" => Some(&mut super_key),
                _ => None,
            };
            if let Some(held) = modifier {
                if *held {
                    return Err(ShortcutError::Repeated(part.to_owned()));
                }
                *held = true;
                continue;
            }
            let parsed = parse_key(&upper).ok_or_else(|| ShortcutError::UnknownKey(part.into()))?;
            if key.replace(parsed).is_some() {
                return Err(ShortcutError::Repeated(part.to_owned()));
            }
        }
        let key = key.ok_or(ShortcutError::NoKey)?;
        if !(ctrl || alt || super_key) {
            return Err(ShortcutError::NeedsModifier);
        }
        Ok(Self {
            ctrl,
            alt,
            shift,
            super_key,
            key,
        })
    }
}

fn parse_key(upper: &str) -> Option<ShortcutKey> {
    let mut chars = upper.chars();
    match (chars.next(), chars.as_str()) {
        (Some(letter @ 'A'..='Z'), "") => Some(ShortcutKey::Letter(letter)),
        (Some(digit @ '0'..='9'), "") => Some(ShortcutKey::Digit(digit)),
        (Some('F'), number) if !number.starts_with('0') && number.len() <= 2 => number
            .parse::<u8>()
            .ok()
            .filter(|number| (1..=24).contains(number))
            .map(ShortcutKey::Function),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_prints_and_parses_as_itself() {
        assert_eq!(Shortcut::DEFAULT.to_string(), "Ctrl+Shift+F9");
        assert_eq!("Ctrl+Shift+F9".parse(), Ok(Shortcut::DEFAULT));
    }

    #[test]
    fn any_order_and_case_becomes_the_canonical_form() {
        for (typed, canonical) in [
            ("shift+ctrl+f9", "Ctrl+Shift+F9"),
            ("n + ALT + control", "Ctrl+Alt+N"),
            ("win+7", "Super+7"),
            ("Meta+Shift+z", "Shift+Super+Z"),
            ("alt+F24", "Alt+F24"),
        ] {
            let shortcut: Shortcut = typed.parse().expect(typed);
            assert_eq!(shortcut.to_string(), canonical);
            assert_eq!(canonical.parse(), Ok(shortcut));
        }
    }

    #[test]
    fn rejects_what_cannot_be_a_global_shortcut() {
        use ShortcutError::{Empty, NeedsModifier, NoKey, Repeated, UnknownKey};
        for (typed, expected) in [
            ("", Empty),
            ("  ", Empty),
            ("Ctrl+Shift", NoKey),
            ("N", NeedsModifier),
            ("Shift+N", NeedsModifier),
            ("F5", NeedsModifier),
            ("Ctrl+Ctrl+N", Repeated("Ctrl".into())),
            ("Ctrl+A+B", Repeated("B".into())),
            ("Ctrl+F25", UnknownKey("F25".into())),
            ("Ctrl+F0", UnknownKey("F0".into())),
            ("Ctrl+F09", UnknownKey("F09".into())),
            ("Ctrl+Enter", UnknownKey("Enter".into())),
            ("Ctrl+é", UnknownKey("é".into())),
            ("Ctrl++N", UnknownKey(String::new())),
        ] {
            assert_eq!(typed.parse::<Shortcut>(), Err(expected), "{typed}");
        }
    }
}
