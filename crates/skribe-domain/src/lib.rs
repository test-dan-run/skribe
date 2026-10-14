//! Skribe domain: identifiers, states and timeline rules.
//!
//! This crate depends on nothing else in the workspace and performs no file, database, device
//! or network I/O, so its rules test the same way on Windows and in WSL.

mod ids;
mod shortcut;
mod states;
mod text;
pub mod timeline;

pub use ids::{AgendaItemId, IdError, LibraryId, MeetingId, SectionId, TrackId};
pub use shortcut::{Shortcut, ShortcutError, ShortcutKey};
pub use states::{
    MinutesState, ProcessingState, RecordingState, TrackFinalisation, TrackHealth, TrackSource,
};
pub use text::{MAX_AGENDA_ITEMS, MAX_TITLE_CHARS, Title, TitleError};
