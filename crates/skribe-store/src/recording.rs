//! A meeting's recording: its two tracks, their results and their health events, and the
//! recording state transitions. Every method is one short transaction.

use rusqlite::{Connection, Row, Transaction};
use skribe_domain::{
    MeetingId, RecordingState, TrackFinalisation, TrackHealth, TrackId, TrackSource,
};

use crate::meetings::{MeetingDetail, begin, commit, load_detail, load_meeting, signed, unsigned};
use crate::{Library, StoreError};

/// A track's native sample format, as recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub valid_bits: u16,
    /// IEEE float samples; otherwise signed integer PCM.
    pub float: bool,
    pub channel_mask: u32,
}

/// A track being registered before capture starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTrack {
    pub id: TrackId,
    pub source: TrackSource,
    pub device_id: String,
    pub device_name: String,
    pub format: TrackFormat,
    /// Relative to the library folder, with forward slashes.
    pub audio_path: String,
}

/// How a track ended: written when its recording is finalised or recovered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackResult {
    pub id: TrackId,
    pub finalisation: TrackFinalisation,
    pub health: TrackHealth,
    /// A stable code for why the track is degraded or failed.
    pub problem: Option<String>,
    /// Technical detail such as a device error code. Never audio content.
    pub problem_detail: Option<String>,
    pub stopped_at_us: Option<i64>,
    pub start_us: Option<i64>,
    pub end_us: Option<i64>,
    pub frame_count: Option<u64>,
    pub data_bytes: Option<u64>,
    pub dropped_frames: u64,
    pub sha256: Option<String>,
}

/// A track as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackRecord {
    pub source: TrackSource,
    pub device_id: String,
    pub device_name: String,
    pub format: TrackFormat,
    pub audio_path: String,
    pub result: TrackResult,
}

/// What kind of thing happened to a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthEventKind {
    Overflow,
    DeviceLost,
    CaptureFailed,
    WriteFailed,
    /// More events arrived at once than could be kept; `frames` counts those that were not.
    EventsDiscarded,
}

impl HealthEventKind {
    pub const ALL: &[Self] = &[
        Self::Overflow,
        Self::DeviceLost,
        Self::CaptureFailed,
        Self::WriteFailed,
        Self::EventsDiscarded,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Overflow => "overflow",
            Self::DeviceLost => "device_lost",
            Self::CaptureFailed => "capture_failed",
            Self::WriteFailed => "write_failed",
            Self::EventsDiscarded => "events_discarded",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.as_str() == text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthEvent {
    pub track: TrackId,
    pub at_us: i64,
    pub kind: HealthEventKind,
    pub frames: Option<u64>,
    pub until_us: Option<i64>,
    pub detail: Option<String>,
}

const TRACK_COLUMNS: &str = "id, source, device_id, device_name, sample_rate, channels, \
     bits_per_sample, valid_bits, sample_kind, channel_mask, audio_path, finalisation, health, \
     problem, problem_detail, stopped_at_us, start_us, end_us, frame_count, data_bytes, \
     dropped_frames, sha256";

fn invalid(detail: String) -> StoreError {
    StoreError::DataInvalid {
        place: "tracks",
        detail,
    }
}

/// A track row with its text and number columns not yet checked.
struct RawTrack {
    id: String,
    source: String,
    device_id: String,
    device_name: String,
    sample_rate: i64,
    channels: i64,
    bits_per_sample: i64,
    valid_bits: i64,
    sample_kind: String,
    channel_mask: i64,
    audio_path: String,
    finalisation: String,
    health: String,
    problem: Option<String>,
    problem_detail: Option<String>,
    stopped_at_us: Option<i64>,
    start_us: Option<i64>,
    end_us: Option<i64>,
    frame_count: Option<i64>,
    data_bytes: Option<i64>,
    dropped_frames: i64,
    sha256: Option<String>,
}

impl RawTrack {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            source: row.get(1)?,
            device_id: row.get(2)?,
            device_name: row.get(3)?,
            sample_rate: row.get(4)?,
            channels: row.get(5)?,
            bits_per_sample: row.get(6)?,
            valid_bits: row.get(7)?,
            sample_kind: row.get(8)?,
            channel_mask: row.get(9)?,
            audio_path: row.get(10)?,
            finalisation: row.get(11)?,
            health: row.get(12)?,
            problem: row.get(13)?,
            problem_detail: row.get(14)?,
            stopped_at_us: row.get(15)?,
            start_us: row.get(16)?,
            end_us: row.get(17)?,
            frame_count: row.get(18)?,
            data_bytes: row.get(19)?,
            dropped_frames: row.get(20)?,
            sha256: row.get(21)?,
        })
    }

    fn decode(self) -> Result<TrackRecord, StoreError> {
        fn number<T: TryFrom<i64>>(name: &str, value: i64) -> Result<T, StoreError> {
            T::try_from(value).map_err(|_| invalid(format!("{name} {value}")))
        }
        let float = match self.sample_kind.as_str() {
            "float" => true,
            "int" => false,
            other => return Err(invalid(format!("sample kind {other}"))),
        };
        Ok(TrackRecord {
            source: TrackSource::parse(&self.source)
                .ok_or_else(|| invalid(format!("source {}", self.source)))?,
            device_id: self.device_id,
            device_name: self.device_name,
            format: TrackFormat {
                sample_rate: number("sample rate", self.sample_rate)?,
                channels: number("channels", self.channels)?,
                bits_per_sample: number("bits per sample", self.bits_per_sample)?,
                valid_bits: number("valid bits", self.valid_bits)?,
                float,
                channel_mask: number("channel mask", self.channel_mask)?,
            },
            audio_path: self.audio_path,
            result: TrackResult {
                id: self
                    .id
                    .parse()
                    .map_err(|error| invalid(format!("id {}: {error}", self.id)))?,
                finalisation: TrackFinalisation::parse(&self.finalisation)
                    .ok_or_else(|| invalid(format!("finalisation {}", self.finalisation)))?,
                health: TrackHealth::parse(&self.health)
                    .ok_or_else(|| invalid(format!("health {}", self.health)))?,
                problem: self.problem,
                problem_detail: self.problem_detail,
                stopped_at_us: self.stopped_at_us,
                start_us: self.start_us,
                end_us: self.end_us,
                frame_count: self.frame_count.map(unsigned),
                data_bytes: self.data_bytes.map(unsigned),
                dropped_frames: unsigned(self.dropped_frames),
                sha256: self.sha256,
            },
        })
    }
}

/// A meeting's tracks, microphone first.
pub(crate) fn load_tracks(
    conn: &Connection,
    meeting: &MeetingId,
) -> Result<Vec<TrackRecord>, StoreError> {
    let mut statement = conn
        .prepare(&format!(
            "SELECT {TRACK_COLUMNS} FROM tracks WHERE meeting_id = ?1 ORDER BY source"
        ))
        .map_err(StoreError::sqlite("read tracks"))?;
    let rows = statement
        .query_map([meeting.to_string()], RawTrack::read)
        .map_err(StoreError::sqlite("read tracks"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::sqlite("read tracks"))?;
    rows.into_iter().map(RawTrack::decode).collect()
}

/// Moves a meeting's recording from one state to the next, refusing an edge the state
/// machine does not have.
fn transition(
    tx: &Transaction<'_>,
    meeting: &MeetingId,
    to: RecordingState,
) -> Result<(), StoreError> {
    let from = load_meeting(tx, meeting)?.recording_state;
    if !from.can_become(to) {
        return Err(StoreError::RecordingStateConflict { from, to });
    }
    tx.execute(
        "UPDATE meetings SET recording_state = ?2 WHERE id = ?1",
        (meeting.to_string(), to.as_str()),
    )
    .map_err(StoreError::sqlite("update recording state"))?;
    Ok(())
}

impl Library {
    /// Registers both tracks and moves the meeting from `setup` to `ready`: preflight has
    /// passed and capture is about to start. Refused for a meeting in the trash.
    pub fn begin_recording(
        &self,
        meeting: MeetingId,
        tracks: Vec<NewTrack>,
    ) -> Result<(), StoreError> {
        self.database().call(move |conn| {
            let tx = begin(conn)?;
            if load_meeting(&tx, &meeting)?.trashed_at_unix_ms.is_some() {
                return Err(StoreError::MeetingInTrash);
            }
            transition(&tx, &meeting, RecordingState::Ready)?;
            for track in &tracks {
                tx.execute(
                    "INSERT INTO tracks (id, meeting_id, source, device_id, device_name, \
                     sample_rate, channels, bits_per_sample, valid_bits, sample_kind, \
                     channel_mask, audio_path) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    (
                        track.id.to_string(),
                        meeting.to_string(),
                        track.source.as_str(),
                        &track.device_id,
                        &track.device_name,
                        track.format.sample_rate,
                        track.format.channels,
                        track.format.bits_per_sample,
                        track.format.valid_bits,
                        if track.format.float { "float" } else { "int" },
                        track.format.channel_mask,
                        &track.audio_path,
                    ),
                )
                .map_err(StoreError::sqlite("register track"))?;
            }
            commit(tx)
        })
    }

    /// Undoes [`Library::begin_recording`] when capture could not start: back to `setup`
    /// with no tracks, because nothing was recorded.
    pub fn abandon_recording_start(&self, meeting: MeetingId) -> Result<(), StoreError> {
        self.database().call(move |conn| {
            let tx = begin(conn)?;
            transition(&tx, &meeting, RecordingState::Setup)?;
            tx.execute(
                "DELETE FROM tracks WHERE meeting_id = ?1",
                [meeting.to_string()],
            )
            .map_err(StoreError::sqlite("remove tracks"))?;
            commit(tx)
        })
    }

    /// `ready` to `recording`: both devices are capturing.
    pub fn mark_recording_started(
        &self,
        meeting: MeetingId,
        started_at_unix_ms: u64,
    ) -> Result<(), StoreError> {
        self.database().call(move |conn| {
            let tx = begin(conn)?;
            transition(&tx, &meeting, RecordingState::Recording)?;
            tx.execute(
                "UPDATE meetings SET recording_started_at_unix_ms = ?2 WHERE id = ?1",
                (meeting.to_string(), signed(started_at_unix_ms)),
            )
            .map_err(StoreError::sqlite("update meeting"))?;
            commit(tx)
        })
    }

    /// `recording` to `finalising`: capture is stopping and the tracks are being completed.
    pub fn mark_finalising(&self, meeting: MeetingId) -> Result<(), StoreError> {
        self.database().call(move |conn| {
            let tx = begin(conn)?;
            transition(&tx, &meeting, RecordingState::Finalising)?;
            commit(tx)
        })
    }

    /// Saves every track's result and settles the recording as `recorded` or `failed`, from
    /// `finalising` after a stop or from `interrupted` after a recovery. `results` must
    /// cover each of the meeting's tracks exactly once.
    pub fn finish_recording(
        &self,
        meeting: MeetingId,
        duration_us: Option<i64>,
        results: Vec<TrackResult>,
    ) -> Result<MeetingDetail, StoreError> {
        self.database().call(move |conn| {
            let tx = begin(conn)?;
            let mut expected: Vec<TrackId> = load_tracks(&tx, &meeting)?
                .into_iter()
                .map(|track| track.result.id)
                .collect();
            let mut given: Vec<TrackId> = results.iter().map(|result| result.id).collect();
            expected.sort();
            given.sort();
            if expected != given {
                return Err(invalid(format!(
                    "results for {} tracks do not match the meeting's {} tracks",
                    given.len(),
                    expected.len()
                )));
            }
            let finalisations: Vec<TrackFinalisation> =
                results.iter().map(|result| result.finalisation).collect();
            transition(
                &tx,
                &meeting,
                RecordingState::after_finalisation(&finalisations),
            )?;
            if let Some(duration_us) = duration_us {
                tx.execute(
                    "UPDATE meetings SET recording_duration_us = ?2 WHERE id = ?1",
                    (meeting.to_string(), duration_us),
                )
                .map_err(StoreError::sqlite("update meeting"))?;
            }
            for result in &results {
                tx.execute(
                    "UPDATE tracks SET finalisation = ?2, health = ?3, problem = ?4, \
                     problem_detail = ?5, stopped_at_us = ?6, start_us = ?7, end_us = ?8, \
                     frame_count = ?9, data_bytes = ?10, dropped_frames = ?11, sha256 = ?12 \
                     WHERE id = ?1",
                    (
                        result.id.to_string(),
                        result.finalisation.as_str(),
                        result.health.as_str(),
                        &result.problem,
                        &result.problem_detail,
                        result.stopped_at_us,
                        result.start_us,
                        result.end_us,
                        result.frame_count.map(signed),
                        result.data_bytes.map(signed),
                        signed(result.dropped_frames),
                        &result.sha256,
                    ),
                )
                .map_err(StoreError::sqlite("save track result"))?;
            }
            let detail = load_detail(&tx, &meeting)?;
            commit(tx)?;
            Ok(detail)
        })
    }

    /// Appends health events. Called while a recording runs and once more when it stops.
    pub fn add_health_events(&self, events: Vec<HealthEvent>) -> Result<(), StoreError> {
        if events.is_empty() {
            return Ok(());
        }
        self.database().call(move |conn| {
            let tx = begin(conn)?;
            for event in &events {
                tx.execute(
                    "INSERT INTO track_health_events \
                     (track_id, at_us, kind, frames, until_us, detail) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    (
                        event.track.to_string(),
                        event.at_us,
                        event.kind.as_str(),
                        event.frames.map(signed),
                        event.until_us,
                        &event.detail,
                    ),
                )
                .map_err(StoreError::sqlite("save health event"))?;
            }
            commit(tx)
        })
    }

    /// A track's health events in the order they were saved.
    pub fn health_events(&self, track: TrackId) -> Result<Vec<HealthEvent>, StoreError> {
        self.database().call(move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT at_us, kind, frames, until_us, detail FROM track_health_events \
                     WHERE track_id = ?1 ORDER BY id",
                )
                .map_err(StoreError::sqlite("read health events"))?;
            let rows = statement
                .query_map([track.to_string()], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                })
                .map_err(StoreError::sqlite("read health events"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StoreError::sqlite("read health events"))?;
            rows.into_iter()
                .map(|(at_us, kind, frames, until_us, detail)| {
                    Ok(HealthEvent {
                        track,
                        at_us,
                        kind: HealthEventKind::parse(&kind).ok_or_else(|| {
                            StoreError::DataInvalid {
                                place: "track_health_events",
                                detail: format!("kind {kind}"),
                            }
                        })?,
                        frames: frames.map(unsigned),
                        until_us,
                        detail,
                    })
                })
                .collect()
        })
    }
}

/// Settles recordings that no capture session owns any more, as found when the library is
/// opened: one library is open in one process at a time, so none can be live.
///
/// A meeting that had reached `recording` or `finalising` has audio on disk and becomes
/// `interrupted`, to be recovered from its journals. A meeting still in `ready` never started
/// capturing, so it returns to `setup` and its track rows are removed.
///
/// Returns how many recordings were marked interrupted.
pub(crate) fn settle_orphaned_recordings(conn: &mut Connection) -> Result<u32, StoreError> {
    let fail = StoreError::sqlite("settle interrupted recordings");
    let has_meetings: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'tracks')",
            [],
            |row| row.get(0),
        )
        .map_err(fail)?;
    if !has_meetings {
        return Ok(0);
    }
    let tx = begin(conn)?;
    tx.execute(
        "DELETE FROM tracks WHERE meeting_id IN \
         (SELECT id FROM meetings WHERE recording_state = 'ready')",
        [],
    )
    .map_err(StoreError::sqlite("settle interrupted recordings"))?;
    tx.execute(
        "UPDATE meetings SET recording_state = 'setup' WHERE recording_state = 'ready'",
        [],
    )
    .map_err(StoreError::sqlite("settle interrupted recordings"))?;
    let interrupted = tx
        .execute(
            "UPDATE meetings SET recording_state = 'interrupted' \
             WHERE recording_state IN ('recording', 'finalising')",
            [],
        )
        .map_err(StoreError::sqlite("settle interrupted recordings"))?;
    commit(tx)?;
    Ok(u32::try_from(interrupted).unwrap_or(u32::MAX))
}
