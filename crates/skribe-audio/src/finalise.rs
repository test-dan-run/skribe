//! Finalising a track, and rebuilding one after a crash. Both end the same way: the audio
//! and clock map are cut at what the journal says is durable, the header is written with the
//! true length, the file is flushed, and its SHA-256 is computed by reading it back.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

use sha2::{Digest, Sha256};

use crate::journal::{self, Committed, Journal};
use crate::{AudioError, AudioFormat, TrackFiles, clock, wav};

/// A finalised track file and what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalisedTrack {
    pub frames: u64,
    /// Bytes of audio, excluding the header.
    pub data_bytes: u64,
    /// SHA-256 of the whole file, in lower-case hexadecimal.
    pub sha256: String,
    /// Timeline time of the first frame. `None` when the device delivered nothing, as the
    /// system track does when nothing is played.
    pub start_us: Option<i64>,
    /// Timeline time just after the last frame.
    pub end_us: Option<i64>,
    /// Frames lost to a full capture buffer. Their place is a gap in the clock map.
    pub dropped_frames: u64,
    /// The file outgrew 4 GB and was written as RF64.
    pub rf64: bool,
}

/// A track rebuilt from its journal after an interrupted recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredTrack {
    pub format: AudioFormat,
    pub track: FinalisedTrack,
    /// The track had been finalised before the interruption; it was verified, not rebuilt.
    pub was_finalised: bool,
}

/// Finalises a track whose writer has finished, at the length its journal last recorded.
pub(crate) fn finalise(
    files: &TrackFiles,
    format: &AudioFormat,
    committed: &Committed,
) -> Result<FinalisedTrack, AudioError> {
    seal(files, format, committed, false)
}

/// Rebuilds a track from its journal: the last intact record says where the audio ends and
/// the journal's header says what format it is in. Whatever was written after that record,
/// at most one flush interval, is cut off. Running it again on a finalised track verifies the
/// track and returns the same result.
pub fn recover(files: &TrackFiles) -> Result<RecoveredTrack, AudioError> {
    let state = journal::read(&files.journal)?;
    let track = seal(files, &state.format, &state.committed, state.finalised)?;
    Ok(RecoveredTrack {
        format: state.format,
        track,
        was_finalised: state.finalised,
    })
}

fn seal(
    files: &TrackFiles,
    format: &AudioFormat,
    committed: &Committed,
    already_finalised: bool,
) -> Result<FinalisedTrack, AudioError> {
    let frame_bytes = u64::from(format.bytes_per_frame());
    let data_bytes = committed.data_bytes - committed.data_bytes % frame_bytes;
    let clock_bytes = committed.clock_bytes - committed.clock_bytes % clock::ENTRY_LEN;

    let mut wav_file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&files.wav)
        .map_err(AudioError::io("open", &files.wav))?;
    let on_disk = wav_file
        .metadata()
        .map_err(AudioError::io("read", &files.wav))?
        .len();
    if on_disk < wav::HEADER_LEN + data_bytes {
        return Err(AudioError::WavInvalid {
            path: files.wav.clone(),
            reason: format!(
                "it holds {on_disk} bytes, fewer than the {} its journal recorded",
                wav::HEADER_LEN + data_bytes
            ),
        });
    }
    // Cutting to the recorded length also drops anything written but never confirmed.
    wav_file
        .set_len(wav::HEADER_LEN + data_bytes + wav::pad_bytes(data_bytes))
        .and_then(|()| wav_file.seek(SeekFrom::Start(0)))
        .and_then(|_| wav_file.write_all(&wav::header(format, data_bytes)))
        .and_then(|()| wav_file.sync_all())
        .map_err(AudioError::io("finalise", &files.wav))?;

    let clock_file = OpenOptions::new()
        .write(true)
        .open(&files.clock)
        .map_err(AudioError::io("open", &files.clock))?;
    let clock_on_disk = clock_file
        .metadata()
        .map_err(AudioError::io("read", &files.clock))?
        .len();
    if clock_on_disk < clock::HEADER_LEN + clock_bytes {
        return Err(AudioError::ClockInvalid {
            path: files.clock.clone(),
            reason: format!(
                "it holds {clock_on_disk} bytes, fewer than the {} its journal recorded",
                clock::HEADER_LEN + clock_bytes
            ),
        });
    }
    clock_file
        .set_len(clock::HEADER_LEN + clock_bytes)
        .and_then(|()| clock_file.sync_all())
        .map_err(AudioError::io("finalise", &files.clock))?;

    let sha256 = sha256_of(&mut wav_file).map_err(AudioError::io("read", &files.wav))?;

    let sealed = Committed {
        data_bytes,
        clock_bytes,
        ..*committed
    };
    if !already_finalised {
        Journal::append_to(&files.journal)?
            .finalised(&sealed)
            .map_err(AudioError::io("write", &files.journal))?;
    }

    let has_audio = data_bytes > 0;
    let riff_size = wav::HEADER_LEN - 8 + data_bytes + wav::pad_bytes(data_bytes);
    Ok(FinalisedTrack {
        frames: data_bytes / frame_bytes,
        data_bytes,
        sha256,
        start_us: has_audio.then_some(committed.start_us),
        end_us: has_audio.then_some(committed.end_us),
        dropped_frames: committed.dropped_frames,
        rf64: riff_size > u64::from(u32::MAX),
    })
}

/// Hashes the file from its first byte with a fixed buffer, however long the recording is.
fn sha256_of(file: &mut File) -> std::io::Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
