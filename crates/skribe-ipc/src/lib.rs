//! The contract between the Skribe frontend and its Rust services: command names, argument and
//! result types, the typed error model, and the handlers that turn service results into them.
//!
//! Nothing here depends on Tauri, so the contract is tested on every platform. The TypeScript
//! in `src/ipc/generated` is produced from these types by this crate's tests.

mod command_names;
mod error;
pub mod library;

pub use command_names::COMMAND_NAMES;
pub use error::{CommandError, ErrorCode, RecoveryAction};

#[cfg(test)]
mod bindings;
