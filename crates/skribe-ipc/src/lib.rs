//! The contract between the Skribe frontend and its Rust services: command names, argument and
//! result types, the typed error model, and the handlers that turn service results into them.
//!
//! Nothing here depends on Tauri, so the contract is tested on every platform. The TypeScript
//! in `src/ipc/generated` is produced from these types by this crate's tests.

use std::sync::Arc;

use skribe_store::{Library, LibraryAvailability, LibraryService};

mod command_names;
mod error;
pub mod library;
pub mod meetings;
pub mod recording;
pub mod settings;
pub mod states;

pub use command_names::COMMAND_NAMES;
pub use error::{CommandError, ErrorCode, RecoveryAction};

/// The open library, or the reason it is closed as the command's error.
pub(crate) fn open_library(service: &LibraryService) -> Result<Arc<Library>, CommandError> {
    match service.availability() {
        LibraryAvailability::Ready(library) => Ok(library),
        LibraryAvailability::Unavailable(error) => Err(CommandError::from(&*error)),
    }
}

#[cfg(test)]
mod bindings;
