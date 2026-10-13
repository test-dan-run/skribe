use std::fmt;
use std::str::FromStr;

use uuid::{Uuid, Version};

/// Why a string was rejected as an identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdError {
    /// Not a UUID in canonical lower-case hyphenated form.
    NotCanonical,
    /// A UUID, but not version 7.
    WrongVersion,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotCanonical => f.write_str("not a canonical lower-case hyphenated UUID"),
            Self::WrongVersion => f.write_str("not a version 7 UUID"),
        }
    }
}

impl std::error::Error for IdError {}

/// Identity of one meeting library. Written to `library.json` and to the database it owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LibraryId(Uuid);

impl LibraryId {
    pub fn generate() -> Self {
        Self(Uuid::now_v7())
    }
}

impl fmt::Display for LibraryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.as_hyphenated().fmt(f)
    }
}

impl FromStr for LibraryId {
    type Err = IdError;

    /// Accepts only the form [`LibraryId`] itself prints, so an ID has one spelling everywhere
    /// it is stored or compared.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let uuid = Uuid::try_parse(text).map_err(|_| IdError::NotCanonical)?;
        if uuid.as_hyphenated().to_string() != text {
            return Err(IdError::NotCanonical);
        }
        if uuid.get_version() != Some(Version::SortRand) {
            return Err(IdError::WrongVersion);
        }
        Ok(Self(uuid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_round_trip_through_text() {
        let id = LibraryId::generate();
        assert_eq!(id.to_string().parse::<LibraryId>(), Ok(id));
    }

    #[test]
    fn generated_ids_are_distinct() {
        assert_ne!(LibraryId::generate(), LibraryId::generate());
    }

    #[test]
    fn rejects_non_canonical_spellings() {
        let id = LibraryId::generate().to_string();
        let upper = id.to_uppercase();
        let simple = id.replace('-', "");
        let braced = format!("{{{id}}}");
        for text in [
            upper.as_str(),
            simple.as_str(),
            braced.as_str(),
            "",
            "not-an-id",
        ] {
            assert_eq!(
                text.parse::<LibraryId>(),
                Err(IdError::NotCanonical),
                "{text}"
            );
        }
    }

    #[test]
    fn rejects_other_uuid_versions() {
        let v4 = "9b2f6c1e-3d4a-4f5b-8c6d-7e8f9a0b1c2d";
        assert_eq!(v4.parse::<LibraryId>(), Err(IdError::WrongVersion));
    }
}
