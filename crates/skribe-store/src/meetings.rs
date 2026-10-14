//! Meetings and their agendas. Every change is one short transaction on the database worker,
//! checked against the revision the caller last saw.

use std::fs;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior};
use skribe_domain::{
    AgendaItemId, MAX_AGENDA_ITEMS, MeetingId, MinutesState, ProcessingState, RecordingState, Title,
};

use crate::recording::{TrackRecord, load_tracks};
use crate::{Library, StoreError, fsutil};

/// A meeting as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingRecord {
    pub id: MeetingId,
    pub title: String,
    pub planned_start_unix_ms: Option<u64>,
    pub recording_state: RecordingState,
    pub processing_state: ProcessingState,
    pub minutes_state: MinutesState,
    /// Bumped by every change to the title, planned time or agenda.
    pub revision: u32,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    /// Set while the meeting is in the trash.
    pub trashed_at_unix_ms: Option<u64>,
    pub agenda_len: u32,
    /// When capture started, by the wall clock.
    pub recording_started_at_unix_ms: Option<u64>,
    /// How long the recording ran on its own clock, in microseconds.
    pub recording_duration_us: Option<u64>,
}

/// One agenda item. Its place in the agenda is its place in [`MeetingDetail::agenda`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgendaItemRecord {
    pub id: AgendaItemId,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingDetail {
    pub meeting: MeetingRecord,
    pub agenda: Vec<AgendaItemRecord>,
    /// The recording's tracks, microphone first. Empty until recording starts.
    pub tracks: Vec<TrackRecord>,
}

/// The fields of a meeting the user edits directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingChanges {
    pub title: Title,
    pub planned_start_unix_ms: Option<u64>,
}

const MEETING_COLUMNS: &str = "id, title, planned_start_unix_ms, recording_state, \
     processing_state, minutes_state, revision, created_at_unix_ms, updated_at_unix_ms, \
     trashed_at_unix_ms, \
     (SELECT count(*) FROM agenda_items WHERE agenda_items.meeting_id = meetings.id), \
     recording_started_at_unix_ms, recording_duration_us";

/// A meeting row before its text columns have been checked.
struct RawMeeting {
    id: String,
    title: String,
    planned_start_unix_ms: Option<i64>,
    recording_state: String,
    processing_state: String,
    minutes_state: String,
    revision: i64,
    created_at_unix_ms: i64,
    updated_at_unix_ms: i64,
    trashed_at_unix_ms: Option<i64>,
    agenda_len: i64,
    recording_started_at_unix_ms: Option<i64>,
    recording_duration_us: Option<i64>,
}

impl RawMeeting {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            title: row.get(1)?,
            planned_start_unix_ms: row.get(2)?,
            recording_state: row.get(3)?,
            processing_state: row.get(4)?,
            minutes_state: row.get(5)?,
            revision: row.get(6)?,
            created_at_unix_ms: row.get(7)?,
            updated_at_unix_ms: row.get(8)?,
            trashed_at_unix_ms: row.get(9)?,
            agenda_len: row.get(10)?,
            recording_started_at_unix_ms: row.get(11)?,
            recording_duration_us: row.get(12)?,
        })
    }

    fn decode(self) -> Result<MeetingRecord, StoreError> {
        let invalid = |detail: String| StoreError::DataInvalid {
            place: "meetings",
            detail,
        };
        Ok(MeetingRecord {
            id: self
                .id
                .parse()
                .map_err(|error| invalid(format!("id {}: {error}", self.id)))?,
            title: self.title,
            planned_start_unix_ms: self.planned_start_unix_ms.map(unsigned),
            recording_state: RecordingState::parse(&self.recording_state)
                .ok_or_else(|| invalid(format!("recording state {}", self.recording_state)))?,
            processing_state: ProcessingState::parse(&self.processing_state)
                .ok_or_else(|| invalid(format!("processing state {}", self.processing_state)))?,
            minutes_state: MinutesState::parse(&self.minutes_state)
                .ok_or_else(|| invalid(format!("minutes state {}", self.minutes_state)))?,
            revision: u32::try_from(self.revision)
                .map_err(|_| invalid(format!("revision {}", self.revision)))?,
            created_at_unix_ms: unsigned(self.created_at_unix_ms),
            updated_at_unix_ms: unsigned(self.updated_at_unix_ms),
            trashed_at_unix_ms: self.trashed_at_unix_ms.map(unsigned),
            agenda_len: u32::try_from(self.agenda_len).unwrap_or(0),
            recording_started_at_unix_ms: self.recording_started_at_unix_ms.map(unsigned),
            recording_duration_us: self.recording_duration_us.map(unsigned),
        })
    }
}

/// SQLite integers are signed; times and sizes are never negative.
pub(crate) fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

pub(crate) fn signed(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub(crate) fn now() -> i64 {
    signed(fsutil::now_unix_ms())
}

pub(crate) fn begin(conn: &mut Connection) -> Result<Transaction<'_>, StoreError> {
    conn.transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(StoreError::sqlite("begin"))
}

pub(crate) fn commit(tx: Transaction<'_>) -> Result<(), StoreError> {
    tx.commit().map_err(StoreError::sqlite("commit"))
}

pub(crate) fn load_meeting(conn: &Connection, id: &MeetingId) -> Result<MeetingRecord, StoreError> {
    conn.query_row(
        &format!("SELECT {MEETING_COLUMNS} FROM meetings WHERE id = ?1"),
        [id.to_string()],
        RawMeeting::read,
    )
    .optional()
    .map_err(StoreError::sqlite("read meeting"))?
    .ok_or_else(|| StoreError::MeetingNotFound { id: id.to_string() })?
    .decode()
}

fn load_agenda(conn: &Connection, id: &MeetingId) -> Result<Vec<AgendaItemRecord>, StoreError> {
    let mut statement = conn
        .prepare("SELECT id, title FROM agenda_items WHERE meeting_id = ?1 ORDER BY position")
        .map_err(StoreError::sqlite("read agenda"))?;
    let rows = statement
        .query_map([id.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(StoreError::sqlite("read agenda"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::sqlite("read agenda"))?;
    rows.into_iter()
        .map(|(item_id, title)| {
            let id = item_id.parse().map_err(|error| StoreError::DataInvalid {
                place: "agenda_items",
                detail: format!("id {item_id}: {error}"),
            })?;
            Ok(AgendaItemRecord { id, title })
        })
        .collect()
}

pub(crate) fn load_detail(conn: &Connection, id: &MeetingId) -> Result<MeetingDetail, StoreError> {
    Ok(MeetingDetail {
        meeting: load_meeting(conn, id)?,
        agenda: load_agenda(conn, id)?,
        tracks: load_tracks(conn, id)?,
    })
}

/// What a change to a meeting requires of it beyond matching the expected revision.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Needs {
    /// The title and planned time stay editable for the life of the meeting.
    NotTrashed,
    /// The agenda is fixed once recording has started.
    Preparation,
}

/// Runs one checked change to a meeting: verifies it exists, is not in the trash, is at the
/// revision the caller saw and is in a state that allows the change; applies `change`; then
/// bumps the revision. Any failure leaves the meeting exactly as it was.
fn change_meeting(
    conn: &mut Connection,
    id: &MeetingId,
    expected_revision: u32,
    needs: Needs,
    change: impl FnOnce(&Transaction<'_>, &MeetingRecord) -> Result<(), StoreError>,
) -> Result<MeetingDetail, StoreError> {
    let tx = begin(conn)?;
    let meeting = load_meeting(&tx, id)?;
    if meeting.trashed_at_unix_ms.is_some() {
        return Err(StoreError::MeetingInTrash);
    }
    if meeting.revision != expected_revision {
        return Err(StoreError::RevisionConflict {
            expected: expected_revision,
            found: meeting.revision,
        });
    }
    if needs == Needs::Preparation && !meeting.recording_state.allows_preparation() {
        return Err(StoreError::PreparationClosed {
            state: meeting.recording_state,
        });
    }
    change(&tx, &meeting)?;
    tx.execute(
        "UPDATE meetings SET revision = revision + 1, updated_at_unix_ms = ?2 WHERE id = ?1",
        (id.to_string(), now()),
    )
    .map_err(StoreError::sqlite("update meeting"))?;
    let detail = load_detail(&tx, id)?;
    commit(tx)?;
    Ok(detail)
}

/// Rewrites the agenda's positions to follow `order`. Positions pass through negative values
/// first so the unique (meeting, position) pair is never violated part-way.
fn write_positions(
    tx: &Transaction<'_>,
    meeting: &MeetingId,
    order: &[AgendaItemId],
) -> Result<(), StoreError> {
    let meeting = meeting.to_string();
    tx.execute(
        "UPDATE agenda_items SET position = -position - 1 WHERE meeting_id = ?1",
        [&meeting],
    )
    .map_err(StoreError::sqlite("reorder agenda"))?;
    let mut statement = tx
        .prepare("UPDATE agenda_items SET position = ?3 WHERE meeting_id = ?1 AND id = ?2")
        .map_err(StoreError::sqlite("reorder agenda"))?;
    for (position, item) in order.iter().enumerate() {
        statement
            .execute((&meeting, item.to_string(), signed(position as u64)))
            .map_err(StoreError::sqlite("reorder agenda"))?;
    }
    Ok(())
}

fn require_item(agenda: &[AgendaItemRecord], item: &AgendaItemId) -> Result<(), StoreError> {
    if agenda.iter().any(|existing| existing.id == *item) {
        Ok(())
    } else {
        Err(StoreError::AgendaItemNotFound {
            id: item.to_string(),
        })
    }
}

impl Library {
    /// Every meeting, newest first, including those in the trash.
    pub fn meetings(&self) -> Result<Vec<MeetingRecord>, StoreError> {
        self.database().call(|conn| {
            let mut statement = conn
                .prepare(&format!(
                    "SELECT {MEETING_COLUMNS} FROM meetings \
                     ORDER BY created_at_unix_ms DESC, id DESC"
                ))
                .map_err(StoreError::sqlite("list meetings"))?;
            let rows = statement
                .query_map([], RawMeeting::read)
                .map_err(StoreError::sqlite("list meetings"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StoreError::sqlite("list meetings"))?;
            rows.into_iter().map(RawMeeting::decode).collect()
        })
    }

    pub fn meeting(&self, id: MeetingId) -> Result<MeetingDetail, StoreError> {
        self.database().call(move |conn| load_detail(conn, &id))
    }

    /// Creates a meeting in `setup` with an empty agenda.
    pub fn create_meeting(&self, new: MeetingChanges) -> Result<MeetingDetail, StoreError> {
        self.database().call(move |conn| {
            let id = MeetingId::generate();
            let tx = begin(conn)?;
            let created = now();
            tx.execute(
                "INSERT INTO meetings \
                 (id, title, planned_start_unix_ms, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?4)",
                (
                    id.to_string(),
                    new.title.as_str(),
                    new.planned_start_unix_ms.map(signed),
                    created,
                ),
            )
            .map_err(StoreError::sqlite("create meeting"))?;
            let detail = load_detail(&tx, &id)?;
            commit(tx)?;
            Ok(detail)
        })
    }

    pub fn update_meeting(
        &self,
        id: MeetingId,
        expected_revision: u32,
        changes: MeetingChanges,
    ) -> Result<MeetingDetail, StoreError> {
        self.database().call(move |conn| {
            change_meeting(conn, &id, expected_revision, Needs::NotTrashed, |tx, _| {
                tx.execute(
                    "UPDATE meetings SET title = ?2, planned_start_unix_ms = ?3 WHERE id = ?1",
                    (
                        id.to_string(),
                        changes.title.as_str(),
                        changes.planned_start_unix_ms.map(signed),
                    ),
                )
                .map_err(StoreError::sqlite("update meeting"))?;
                Ok(())
            })
        })
    }

    /// Appends an agenda item.
    pub fn add_agenda_item(
        &self,
        meeting: MeetingId,
        expected_revision: u32,
        title: Title,
    ) -> Result<MeetingDetail, StoreError> {
        self.database().call(move |conn| {
            change_meeting(
                conn,
                &meeting,
                expected_revision,
                Needs::Preparation,
                |tx, current| {
                    if current.agenda_len as usize >= MAX_AGENDA_ITEMS {
                        return Err(StoreError::AgendaFull {
                            limit: MAX_AGENDA_ITEMS,
                        });
                    }
                    tx.execute(
                        "INSERT INTO agenda_items (id, meeting_id, position, title) \
                         VALUES (?1, ?2, ?3, ?4)",
                        (
                            AgendaItemId::generate().to_string(),
                            meeting.to_string(),
                            i64::from(current.agenda_len),
                            title.as_str(),
                        ),
                    )
                    .map_err(StoreError::sqlite("add agenda item"))?;
                    Ok(())
                },
            )
        })
    }

    pub fn edit_agenda_item(
        &self,
        meeting: MeetingId,
        expected_revision: u32,
        item: AgendaItemId,
        title: Title,
    ) -> Result<MeetingDetail, StoreError> {
        self.database().call(move |conn| {
            change_meeting(
                conn,
                &meeting,
                expected_revision,
                Needs::Preparation,
                |tx, _| {
                    let changed = tx
                        .execute(
                            "UPDATE agenda_items SET title = ?3 WHERE meeting_id = ?1 AND id = ?2",
                            (meeting.to_string(), item.to_string(), title.as_str()),
                        )
                        .map_err(StoreError::sqlite("edit agenda item"))?;
                    if changed == 0 {
                        return Err(StoreError::AgendaItemNotFound {
                            id: item.to_string(),
                        });
                    }
                    Ok(())
                },
            )
        })
    }

    /// Removes an agenda item and closes the gap it leaves.
    pub fn remove_agenda_item(
        &self,
        meeting: MeetingId,
        expected_revision: u32,
        item: AgendaItemId,
    ) -> Result<MeetingDetail, StoreError> {
        self.database().call(move |conn| {
            change_meeting(
                conn,
                &meeting,
                expected_revision,
                Needs::Preparation,
                |tx, _| {
                    let agenda = load_agenda(tx, &meeting)?;
                    require_item(&agenda, &item)?;
                    tx.execute(
                        "DELETE FROM agenda_items WHERE meeting_id = ?1 AND id = ?2",
                        (meeting.to_string(), item.to_string()),
                    )
                    .map_err(StoreError::sqlite("remove agenda item"))?;
                    let remaining: Vec<AgendaItemId> = agenda
                        .into_iter()
                        .map(|existing| existing.id)
                        .filter(|existing| *existing != item)
                        .collect();
                    write_positions(tx, &meeting, &remaining)
                },
            )
        })
    }

    /// Puts the whole agenda in `order` in one transaction. `order` must list every current
    /// item exactly once.
    pub fn reorder_agenda(
        &self,
        meeting: MeetingId,
        expected_revision: u32,
        order: Vec<AgendaItemId>,
    ) -> Result<MeetingDetail, StoreError> {
        self.database().call(move |conn| {
            change_meeting(
                conn,
                &meeting,
                expected_revision,
                Needs::Preparation,
                |tx, _| {
                    let mut current: Vec<AgendaItemId> = load_agenda(tx, &meeting)?
                        .into_iter()
                        .map(|item| item.id)
                        .collect();
                    let mut requested = order.clone();
                    current.sort();
                    requested.sort();
                    if current != requested {
                        return Err(StoreError::AgendaOrderInvalid);
                    }
                    write_positions(tx, &meeting, &order)
                },
            )
        })
    }

    /// Moves a meeting to the trash. Nothing is deleted: its rows and files stay in place.
    /// Refused while a capture session owns the meeting.
    pub fn trash_meeting(&self, id: MeetingId) -> Result<(), StoreError> {
        self.database().call(move |conn| {
            let tx = begin(conn)?;
            let meeting = load_meeting(&tx, &id)?;
            if meeting.trashed_at_unix_ms.is_some() {
                return Err(StoreError::MeetingInTrash);
            }
            if meeting.recording_state.is_live() {
                return Err(StoreError::MeetingBusy {
                    state: meeting.recording_state,
                });
            }
            tx.execute(
                "UPDATE meetings SET trashed_at_unix_ms = ?2 WHERE id = ?1",
                (id.to_string(), now()),
            )
            .map_err(StoreError::sqlite("trash meeting"))?;
            commit(tx)
        })
    }

    pub fn restore_meeting(&self, id: MeetingId) -> Result<(), StoreError> {
        self.database().call(move |conn| {
            let tx = begin(conn)?;
            if load_meeting(&tx, &id)?.trashed_at_unix_ms.is_none() {
                return Err(StoreError::MeetingNotInTrash);
            }
            tx.execute(
                "UPDATE meetings SET trashed_at_unix_ms = NULL WHERE id = ?1",
                [id.to_string()],
            )
            .map_err(StoreError::sqlite("restore meeting"))?;
            commit(tx)
        })
    }

    /// Deletes a meeting that is in the trash, with its audio and derived files. This cannot
    /// be undone.
    ///
    /// Files go first. If they cannot all be removed the meeting stays in the trash, so the
    /// deletion can be tried again and no folder is left without a meeting that owns it.
    pub fn delete_meeting_permanently(&self, id: MeetingId) -> Result<(), StoreError> {
        let trashed = self
            .database()
            .call(move |conn| Ok(load_meeting(conn, &id)?.trashed_at_unix_ms.is_some()))?;
        if !trashed {
            return Err(StoreError::MeetingNotInTrash);
        }
        for folder in [
            self.roots().meeting_audio_dir(&id),
            self.roots().meeting_derived_dir(&id),
        ] {
            remove_folder(&folder)?;
        }
        self.database().call(move |conn| {
            let tx = begin(conn)?;
            // Checked again inside the transaction that deletes.
            if load_meeting(&tx, &id)?.trashed_at_unix_ms.is_none() {
                return Err(StoreError::MeetingNotInTrash);
            }
            tx.execute("DELETE FROM meetings WHERE id = ?1", [id.to_string()])
                .map_err(StoreError::sqlite("delete meeting"))?;
            commit(tx)
        })
    }
}

fn remove_folder(folder: &Path) -> Result<(), StoreError> {
    let not_removed = |source| StoreError::MeetingFilesNotRemoved {
        path: folder.to_owned(),
        source,
    };
    match fs::remove_dir_all(folder) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(not_removed(error)),
    }
    match folder.parent() {
        Some(parent) => fsutil::sync_dir(parent),
        None => Ok(()),
    }
}
