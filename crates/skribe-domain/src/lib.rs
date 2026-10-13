//! Skribe domain: identifiers, states and timeline rules.
//!
//! This crate depends on nothing else in the workspace and performs no file, database, device
//! or network I/O, so its rules test the same way on Windows and in WSL.

mod ids;

pub use ids::{IdError, LibraryId};
