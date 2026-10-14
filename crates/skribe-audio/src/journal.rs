//! The track journal: what has been made durable, and enough to rebuild the track's header.
//!
//! While a track is written, its WAV header does not yet know how long the audio is. After
//! every flush the writer appends one record here saying how many bytes of audio and of the
//! clock map are safely on disk. If the process dies, the last intact record says exactly
//! where the track ends, and the header at the front of the journal says what format it is
//! in, so the track can be rebuilt without trusting anything written after that record.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::clock::array;
use crate::{AudioError, AudioFormat, SampleKind};

const MAGIC: &[u8; 8] = b"SKRJNL01";
const HEADER_LEN: usize = 32;
const RECORD_LEN: usize = 48;
const TAG_COMMIT: u32 = 1;
const TAG_FINALISED: u32 = 2;

/// How much of a track is durable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Committed {
    /// Bytes of audio after the WAV header.
    pub data_bytes: u64,
    /// Bytes of clock map entries after the clock header.
    pub clock_bytes: u64,
    /// Frames dropped so far because the capture buffer was full.
    pub dropped_frames: u64,
    /// Timeline time of the first frame; meaningful once `data_bytes` is not zero.
    pub start_us: i64,
    /// Timeline time just after the last frame.
    pub end_us: i64,
}

/// What a journal says about its track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalState {
    pub format: AudioFormat,
    /// The last intact record. All zeros when nothing was committed.
    pub committed: Committed,
    /// The track was finalised after `committed` was written.
    pub finalised: bool,
}

/// IEEE CRC-32, bit by bit. Records are a few dozen bytes written once a second.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn header(format: &AudioFormat) -> [u8; HEADER_LEN] {
    let mut out = Vec::with_capacity(HEADER_LEN);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&format.sample_rate.to_le_bytes());
    out.extend_from_slice(&format.channels.to_le_bytes());
    out.extend_from_slice(&format.bits_per_sample.to_le_bytes());
    out.extend_from_slice(&format.valid_bits.to_le_bytes());
    let kind: u16 = match format.kind {
        SampleKind::Int => 1,
        SampleKind::Float => 3,
    };
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&format.channel_mask.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    let crc = crc32(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    array(&out)
}

fn record(tag: u32, committed: &Committed) -> [u8; RECORD_LEN] {
    let mut out = Vec::with_capacity(RECORD_LEN);
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&committed.data_bytes.to_le_bytes());
    out.extend_from_slice(&committed.clock_bytes.to_le_bytes());
    out.extend_from_slice(&committed.dropped_frames.to_le_bytes());
    out.extend_from_slice(&committed.start_us.to_le_bytes());
    out.extend_from_slice(&committed.end_us.to_le_bytes());
    let crc = crc32(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    array(&out)
}

/// An open journal being appended to.
#[derive(Debug)]
pub(crate) struct Journal {
    file: File,
}

impl Journal {
    /// Creates the journal with its format header and makes that durable.
    pub(crate) fn create(path: &Path, format: &AudioFormat) -> Result<Self, AudioError> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(AudioError::io("create", path))?;
        file.write_all(&header(format))
            .and_then(|()| file.sync_all())
            .map_err(AudioError::io("write", path))?;
        Ok(Self { file })
    }

    /// Opens an existing journal to add its final record.
    pub(crate) fn append_to(path: &Path) -> Result<Self, AudioError> {
        let file = OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(AudioError::io("open", path))?;
        Ok(Self { file })
    }

    /// Records that `committed` is on disk. Call only after the audio and clock map it
    /// describes have been flushed; the record is itself flushed before this returns.
    pub(crate) fn commit(&mut self, committed: &Committed) -> std::io::Result<()> {
        self.append(TAG_COMMIT, committed)
    }

    pub(crate) fn finalised(&mut self, committed: &Committed) -> std::io::Result<()> {
        self.append(TAG_FINALISED, committed)
    }

    fn append(&mut self, tag: u32, committed: &Committed) -> std::io::Result<()> {
        self.file.write_all(&record(tag, committed))?;
        self.file.sync_data()
    }
}

/// Reads a journal. Records are read until the first one that is short or fails its
/// checksum; a torn record at the end is what a crash mid-write leaves, and is ignored.
pub fn read(path: &Path) -> Result<JournalState, AudioError> {
    let bytes = fs::read(path).map_err(AudioError::io("read", path))?;
    let invalid = |reason: &str| AudioError::JournalInvalid {
        path: path.to_owned(),
        reason: reason.to_owned(),
    };
    if bytes.len() < HEADER_LEN || &bytes[..8] != MAGIC {
        return Err(invalid("it does not start with a journal header"));
    }
    let (head, body) = bytes.split_at(HEADER_LEN);
    if crc32(&head[..HEADER_LEN - 4]) != u32::from_le_bytes(array(&head[HEADER_LEN - 4..])) {
        return Err(invalid("its header fails its checksum"));
    }
    let kind = match u16::from_le_bytes(array(&head[18..])) {
        1 => SampleKind::Int,
        3 => SampleKind::Float,
        _ => return Err(invalid("its sample kind is not PCM or float")),
    };
    let format = AudioFormat {
        sample_rate: u32::from_le_bytes(array(&head[8..])),
        channels: u16::from_le_bytes(array(&head[12..])),
        bits_per_sample: u16::from_le_bytes(array(&head[14..])),
        valid_bits: u16::from_le_bytes(array(&head[16..])),
        kind,
        channel_mask: u32::from_le_bytes(array(&head[20..])),
    };
    if let Err(error) = format.validate() {
        return Err(invalid(&format!("it describes {error}")));
    }

    let mut state = JournalState {
        format,
        committed: Committed::default(),
        finalised: false,
    };
    let (records, _) = body.as_chunks::<RECORD_LEN>();
    for entry in records {
        let (fields, crc) = entry.split_at(RECORD_LEN - 4);
        if crc32(fields) != u32::from_le_bytes(array(crc)) {
            break;
        }
        let tag = u32::from_le_bytes(array(fields));
        if tag != TAG_COMMIT && tag != TAG_FINALISED {
            break;
        }
        state.committed = Committed {
            data_bytes: u64::from_le_bytes(array(&fields[4..])),
            clock_bytes: u64::from_le_bytes(array(&fields[12..])),
            dropped_frames: u64::from_le_bytes(array(&fields[20..])),
            start_us: i64::from_le_bytes(array(&fields[28..])),
            end_us: i64::from_le_bytes(array(&fields[36..])),
        };
        state.finalised = tag == TAG_FINALISED;
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORMAT: AudioFormat = AudioFormat {
        sample_rate: 48_000,
        channels: 2,
        bits_per_sample: 32,
        valid_bits: 32,
        kind: SampleKind::Float,
        channel_mask: 3,
    };

    fn committed(data_bytes: u64) -> Committed {
        Committed {
            data_bytes,
            clock_bytes: data_bytes / 160,
            dropped_frames: 7,
            start_us: 1_234,
            end_us: 1_234 + i64::try_from(data_bytes).expect("small"),
        }
    }

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn a_new_journal_knows_the_format_and_that_nothing_is_committed() {
        let dir = tempfile::tempdir().expect("temporary folder");
        let path = dir.path().join("track.journal");
        drop(Journal::create(&path, &FORMAT).expect("create"));

        assert_eq!(
            read(&path).expect("read"),
            JournalState {
                format: FORMAT,
                committed: Committed::default(),
                finalised: false
            }
        );
        // A track's journal is never silently replaced.
        assert!(Journal::create(&path, &FORMAT).is_err());
    }

    #[test]
    fn the_last_intact_record_wins() {
        let dir = tempfile::tempdir().expect("temporary folder");
        let path = dir.path().join("track.journal");
        let mut journal = Journal::create(&path, &FORMAT).expect("create");
        for bytes in [3_840, 7_680, 11_520] {
            journal.commit(&committed(bytes)).expect("commit");
        }
        drop(journal);

        let state = read(&path).expect("read");
        assert_eq!(state.committed, committed(11_520));
        assert!(!state.finalised);
    }

    #[test]
    fn a_record_torn_or_damaged_by_a_crash_is_ignored_along_with_what_follows() {
        let dir = tempfile::tempdir().expect("temporary folder");
        let path = dir.path().join("track.journal");
        let mut journal = Journal::create(&path, &FORMAT).expect("create");
        journal.commit(&committed(3_840)).expect("commit");
        journal.commit(&committed(7_680)).expect("commit");
        drop(journal);
        let intact = fs::read(&path).expect("read");

        // Half a record appended.
        let mut torn = intact.clone();
        torn.extend_from_slice(&record(TAG_COMMIT, &committed(11_520))[..20]);
        fs::write(&path, &torn).expect("write");
        assert_eq!(read(&path).expect("read").committed, committed(7_680));

        // The second record damaged in place: only the first can be trusted.
        let mut damaged = intact;
        damaged[HEADER_LEN + RECORD_LEN + 10] ^= 0xFF;
        fs::write(&path, &damaged).expect("write");
        assert_eq!(read(&path).expect("read").committed, committed(3_840));
    }

    #[test]
    fn finalising_is_recorded_so_recovery_knows_the_track_is_complete() {
        let dir = tempfile::tempdir().expect("temporary folder");
        let path = dir.path().join("track.journal");
        let mut journal = Journal::create(&path, &FORMAT).expect("create");
        journal.commit(&committed(3_840)).expect("commit");
        drop(journal);

        Journal::append_to(&path)
            .expect("open")
            .finalised(&committed(3_840))
            .expect("finalise");

        let state = read(&path).expect("read");
        assert!(state.finalised);
        assert_eq!(state.committed, committed(3_840));
    }

    #[test]
    fn a_journal_with_a_damaged_header_cannot_rebuild_a_track() {
        let dir = tempfile::tempdir().expect("temporary folder");
        let path = dir.path().join("track.journal");
        drop(Journal::create(&path, &FORMAT).expect("create"));
        let mut bytes = fs::read(&path).expect("read");
        bytes[9] ^= 0xFF;
        fs::write(&path, &bytes).expect("write");

        assert!(matches!(
            read(&path),
            Err(AudioError::JournalInvalid { .. })
        ));
        fs::write(&path, b"short").expect("write");
        assert!(matches!(
            read(&path),
            Err(AudioError::JournalInvalid { .. })
        ));
    }
}
