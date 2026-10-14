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
mod meetings;
mod migrate;
mod recording;
mod roots;
mod service;
mod settings;
mod snapshot;
mod worker;

pub use error::{FailedMigrationOutcome, StoreError};
pub use library::{
    InterruptedMigrationRecovery, Library, LibraryDescription, OpenReport, RestoreReport,
};
pub use manifest::{LIBRARY_FORMAT_VERSION, LibraryManifest};
pub use meetings::{AgendaItemRecord, MeetingChanges, MeetingDetail, MeetingRecord};
pub use migrate::{MigrationReport, latest_schema_version};
pub use recording::{
    HealthEvent, HealthEventKind, NewTrack, TrackFormat, TrackRecord, TrackResult,
};
pub use roots::StorageRoots;
pub use service::{LibraryAvailability, LibraryService};
pub use settings::{DeviceSetting, RecordingDeviceSettings, Stored};
pub use snapshot::SnapshotInfo;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod testutil;
