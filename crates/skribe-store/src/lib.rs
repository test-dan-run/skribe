//! Skribe library storage.
//!
//! This is the only crate that touches SQLite. It owns the storage roots, the library manifest,
//! the schema and its migrations, and database snapshots. Callers receive typed results and
//! [`StoreError`]; no SQLite type crosses the crate boundary.

mod db;
mod error;
mod fsutil;
mod library;
mod lock;
mod manifest;
mod migrate;
mod roots;
mod service;
mod snapshot;
mod worker;

pub use error::{FailedMigrationOutcome, StoreError};
pub use library::{
    InterruptedMigrationRecovery, Library, LibraryDescription, OpenReport, RestoreReport,
};
pub use manifest::{LIBRARY_FORMAT_VERSION, LibraryManifest};
pub use migrate::MigrationReport;
pub use roots::StorageRoots;
pub use service::{LibraryAvailability, LibraryService};
pub use snapshot::SnapshotInfo;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod testutil;
