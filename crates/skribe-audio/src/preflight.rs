//! The storage half of preflight: can the recording's folder be written, and is there room
//! for a recording of a stated minimum length.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use crate::{AudioError, AudioFormat};

/// A recording is not started unless the disk has room for at least this much of it.
pub const MINIMUM_RECORDING: Duration = Duration::from_secs(60 * 60);

/// Clock maps and journals add well under one percent to the audio.
const OVERHEAD_PERCENT: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskSpace {
    pub available_bytes: u64,
    /// What [`MINIMUM_RECORDING`] of both tracks needs.
    pub required_bytes: u64,
    /// What both tracks write each second.
    pub bytes_per_second: u64,
}

impl DiskSpace {
    /// How long a recording the free space allows, at these formats.
    pub fn available_duration(&self) -> Duration {
        Duration::from_secs(self.available_bytes / self.bytes_per_second.max(1))
    }
}

/// Bytes both tracks write per second, including their clock maps and journals.
pub fn bytes_per_second(formats: &[AudioFormat]) -> u64 {
    let audio: u64 = formats.iter().map(AudioFormat::bytes_per_second).sum();
    audio + audio * OVERHEAD_PERCENT / 100
}

/// Checks that the disk holding `dir` has room for [`MINIMUM_RECORDING`] at `formats`.
pub fn check_disk_space(dir: &Path, formats: &[AudioFormat]) -> Result<DiskSpace, AudioError> {
    let available_bytes = fs4::available_space(dir).map_err(AudioError::io("measure", dir))?;
    let bytes_per_second = bytes_per_second(formats);
    let space = DiskSpace {
        available_bytes,
        required_bytes: bytes_per_second * MINIMUM_RECORDING.as_secs(),
        bytes_per_second,
    };
    if space.available_bytes < space.required_bytes {
        return Err(AudioError::DiskSpaceLow {
            path: dir.to_owned(),
            available_bytes: space.available_bytes,
            required_bytes: space.required_bytes,
        });
    }
    Ok(space)
}

/// Creates `dir` if needed and proves a file can be created, flushed and removed in it.
pub fn check_writable(dir: &Path) -> Result<(), AudioError> {
    fs::create_dir_all(dir).map_err(AudioError::io("create", dir))?;
    let probe = dir.join(".skribe-write-check");
    let result = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&probe)
        .and_then(|mut file| {
            file.write_all(b"skribe")?;
            file.sync_all()
        });
    let removed = fs::remove_file(&probe);
    result.and(removed).map_err(AudioError::io("write to", dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SampleKind;

    const FLOAT_STEREO: AudioFormat = AudioFormat {
        sample_rate: 48_000,
        channels: 2,
        bits_per_sample: 32,
        valid_bits: 32,
        kind: SampleKind::Float,
        channel_mask: 3,
    };

    #[test]
    fn the_requirement_is_an_hour_of_both_tracks() {
        let rate = bytes_per_second(&[FLOAT_STEREO, FLOAT_STEREO]);
        assert_eq!(rate, 768_000 + 7_680);

        let dir = tempfile::tempdir().expect("temporary folder");
        let space = check_disk_space(dir.path(), &[FLOAT_STEREO, FLOAT_STEREO])
            .expect("the test machine has a few gigabytes free");
        assert_eq!(space.required_bytes, rate * 3_600);
        assert!(space.available_duration() >= MINIMUM_RECORDING);
    }

    #[test]
    fn too_little_space_is_refused_with_both_figures() {
        let dir = tempfile::tempdir().expect("temporary folder");
        // No disk has room for an hour at this rate.
        let absurd = AudioFormat {
            sample_rate: 384_000,
            channels: 32,
            ..FLOAT_STEREO
        };
        let formats = vec![absurd; 100_000];

        match check_disk_space(dir.path(), &formats) {
            Err(AudioError::DiskSpaceLow {
                available_bytes,
                required_bytes,
                ..
            }) => assert!(available_bytes < required_bytes),
            other => panic!("expected too little space, got {other:?}"),
        }
    }

    #[test]
    fn a_folder_that_can_be_written_passes_and_leaves_nothing_behind() {
        let dir = tempfile::tempdir().expect("temporary folder");
        let target = dir.path().join("audio").join("meeting");

        check_writable(&target).expect("writable");

        assert!(target.is_dir());
        assert_eq!(fs::read_dir(&target).expect("list").count(), 0);
    }

    #[test]
    fn a_folder_that_cannot_be_created_is_refused() {
        let dir = tempfile::tempdir().expect("temporary folder");
        let file = dir.path().join("not-a-folder");
        fs::write(&file, b"x").expect("write");

        assert!(matches!(
            check_writable(&file.join("meeting")),
            Err(AudioError::Io { .. })
        ));
    }
}
