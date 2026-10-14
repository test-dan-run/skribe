//! The recording's common clock, and the per-track clock map.
//!
//! One monotonic origin is taken when a recording starts. Every packet a device delivers is
//! stamped with the device's own timestamp, converted to microseconds from that origin, and
//! written to `<track>.clock` beside the audio. Sample position therefore maps to timeline
//! time without assuming the nominal sample rate, which is what keeps two devices that drift
//! apart aligned. Time during which a device delivered nothing is a gap between entries, not
//! missing time and not invented silence.

use std::fs;
use std::path::Path;
use std::time::Instant;

use crate::{AudioError, AudioFormat};

/// The monotonic origin of one recording. Both tracks, and later every note and section
/// transition, are stamped against it.
#[derive(Debug, Clone, Copy)]
pub struct SessionClock {
    origin: Instant,
}

impl SessionClock {
    pub fn start() -> Self {
        Self {
            origin: Instant::now(),
        }
    }

    /// Microseconds since the recording started.
    pub fn now_us(&self) -> i64 {
        self.at(Instant::now())
    }

    pub fn at(&self, instant: Instant) -> i64 {
        let micros = instant.saturating_duration_since(self.origin).as_micros();
        i64::try_from(micros).unwrap_or(i64::MAX)
    }
}

/// Flags on a clock map entry.
pub mod flags {
    /// The device reported a break in its data before this packet.
    pub const DISCONTINUITY: u32 = 1;
    /// The device marked this packet as silence. Its samples were written as zeros.
    pub const SILENT: u32 = 1 << 1;
    /// Frames were dropped before this packet because the capture buffer was full.
    pub const AFTER_DROP: u32 = 1 << 2;
    /// The device gave no usable timestamp; the time is when the packet arrived.
    pub const TIME_ESTIMATED: u32 = 1 << 3;
}

/// One packet's place in the track and on the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockEntry {
    /// Position of the packet's first frame in the track.
    pub frame: u64,
    /// Timeline time of that frame, in microseconds from the recording's origin.
    pub timeline_us: i64,
    pub frames: u32,
    pub flags: u32,
}

pub(crate) const MAGIC: &[u8; 8] = b"SKRCLK01";
pub(crate) const HEADER_LEN: u64 = 16;
pub(crate) const ENTRY_LEN: u64 = 24;

pub(crate) fn header(format: &AudioFormat) -> [u8; HEADER_LEN as usize] {
    let mut bytes = [0; HEADER_LEN as usize];
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..12].copy_from_slice(&format.sample_rate.to_le_bytes());
    bytes
}

impl ClockEntry {
    pub(crate) fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.frame.to_le_bytes());
        out.extend_from_slice(&self.timeline_us.to_le_bytes());
        out.extend_from_slice(&self.frames.to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
    }

    fn decode(bytes: &[u8]) -> Self {
        let field = |from: usize, len: usize| &bytes[from..from + len];
        Self {
            frame: u64::from_le_bytes(array(field(0, 8))),
            timeline_us: i64::from_le_bytes(array(field(8, 8))),
            frames: u32::from_le_bytes(array(field(16, 4))),
            flags: u32::from_le_bytes(array(field(20, 4))),
        }
    }
}

pub(crate) fn array<const N: usize>(bytes: &[u8]) -> [u8; N] {
    let mut out = [0; N];
    out.copy_from_slice(&bytes[..N]);
    out
}

/// A stretch of timeline during which the track holds no audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gap {
    /// The first frame after the gap.
    pub frame: u64,
    pub from_us: i64,
    pub to_us: i64,
}

/// A track's clock map, read back from its `.clock` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockMap {
    pub sample_rate: u32,
    pub entries: Vec<ClockEntry>,
}

impl ClockMap {
    /// Reads the map. A trailing partial entry, left by a crash, is ignored.
    pub fn read(path: &Path) -> Result<Self, AudioError> {
        let bytes = fs::read(path).map_err(AudioError::io("read", path))?;
        let invalid = |reason: &str| AudioError::ClockInvalid {
            path: path.to_owned(),
            reason: reason.to_owned(),
        };
        if bytes.len() < HEADER_LEN as usize || &bytes[..8] != MAGIC {
            return Err(invalid("it does not start with a clock map header"));
        }
        let sample_rate = u32::from_le_bytes(array(&bytes[8..12]));
        if sample_rate == 0 {
            return Err(invalid("its sample rate is zero"));
        }
        // A trailing partial entry falls into the remainder and is left out.
        let (whole, _) = bytes[HEADER_LEN as usize..].as_chunks::<{ ENTRY_LEN as usize }>();
        let entries = whole
            .iter()
            .map(|entry| ClockEntry::decode(entry))
            .collect();
        Ok(Self {
            sample_rate,
            entries,
        })
    }

    fn duration_us(&self, frames: u64) -> i64 {
        let micros = u128::from(frames) * 1_000_000 / u128::from(self.sample_rate);
        i64::try_from(micros).unwrap_or(i64::MAX)
    }

    /// Total frames the map covers.
    pub fn frames(&self) -> u64 {
        self.entries
            .last()
            .map_or(0, |entry| entry.frame + u64::from(entry.frames))
    }

    /// Timeline time of the track's first frame.
    pub fn start_us(&self) -> Option<i64> {
        self.entries.first().map(|entry| entry.timeline_us)
    }

    /// Timeline time just after the track's last frame.
    pub fn end_us(&self) -> Option<i64> {
        self.entries
            .last()
            .map(|entry| entry.timeline_us + self.duration_us(u64::from(entry.frames)))
    }

    /// Timeline time of `frame`: the timestamp of the packet holding it, plus its distance
    /// into that packet at the nominal rate. `None` past the end of the track.
    pub fn time_of_frame(&self, frame: u64) -> Option<i64> {
        let after = self.entries.partition_point(|entry| entry.frame <= frame);
        let entry = self.entries.get(after.checked_sub(1)?)?;
        let into = frame - entry.frame;
        (into < u64::from(entry.frames)).then(|| entry.timeline_us + self.duration_us(into))
    }

    /// Stretches longer than `threshold_us` between the end of one packet and the start of
    /// the next: silence the loopback device did not deliver, a lost device, or dropped
    /// frames.
    pub fn gaps(&self, threshold_us: i64) -> Vec<Gap> {
        self.entries
            .windows(2)
            .filter_map(|pair| {
                let from_us = pair[0].timeline_us + self.duration_us(u64::from(pair[0].frames));
                let to_us = pair[1].timeline_us;
                (to_us - from_us > threshold_us).then_some(Gap {
                    frame: pair[1].frame,
                    from_us,
                    to_us,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::SampleKind;

    const MONO: AudioFormat = AudioFormat {
        sample_rate: 48_000,
        channels: 1,
        bits_per_sample: 16,
        valid_bits: 16,
        kind: SampleKind::Int,
        channel_mask: 0,
    };

    fn write(path: &Path, entries: &[ClockEntry], trailing: &[u8]) {
        let mut bytes = header(&MONO).to_vec();
        for entry in entries {
            entry.encode(&mut bytes);
        }
        bytes.extend_from_slice(trailing);
        fs::write(path, bytes).expect("write clock map");
    }

    fn entry(frame: u64, timeline_us: i64, frames: u32) -> ClockEntry {
        ClockEntry {
            frame,
            timeline_us,
            frames,
            flags: 0,
        }
    }

    #[test]
    fn the_session_clock_counts_from_its_origin() {
        let clock = SessionClock::start();
        let later = clock.origin + Duration::from_millis(1_500);
        assert_eq!(clock.at(later), 1_500_000);
        // A device timestamp from just before the origin does not go negative.
        assert_eq!(clock.at(clock.origin - Duration::from_millis(5)), 0);
        assert!(clock.now_us() >= 0);
    }

    #[test]
    fn entries_round_trip_and_a_torn_last_entry_is_ignored() {
        let dir = tempfile::tempdir().expect("temporary folder");
        let path = dir.path().join("track.clock");
        let entries = [
            entry(0, 2_000, 480),
            ClockEntry {
                flags: flags::AFTER_DROP | flags::DISCONTINUITY,
                ..entry(480, 12_000, 480)
            },
        ];
        write(&path, &entries, &[1, 2, 3, 4, 5]);

        let map = ClockMap::read(&path).expect("read");

        assert_eq!(map.sample_rate, 48_000);
        assert_eq!(map.entries, entries);
        assert_eq!(map.frames(), 960);
        assert_eq!(map.start_us(), Some(2_000));
        assert_eq!(map.end_us(), Some(22_000));
    }

    #[test]
    fn frame_times_follow_device_timestamps_not_the_nominal_rate() {
        let dir = tempfile::tempdir().expect("temporary folder");
        let path = dir.path().join("track.clock");
        // The device runs slightly slow: each 10 ms packet arrives 10.1 ms after the last.
        write(
            &path,
            &[
                entry(0, 0, 480),
                entry(480, 10_100, 480),
                entry(960, 20_200, 480),
            ],
            &[],
        );
        let map = ClockMap::read(&path).expect("read");

        assert_eq!(map.time_of_frame(0), Some(0));
        assert_eq!(map.time_of_frame(240), Some(5_000));
        assert_eq!(map.time_of_frame(480), Some(10_100));
        assert_eq!(map.time_of_frame(960 + 479), Some(20_200 + 9_979));
        assert_eq!(map.time_of_frame(1_440), None);
        // Drift of 0.1 ms per packet is not a gap.
        assert!(map.gaps(1_000).is_empty());
    }

    #[test]
    fn time_without_packets_is_a_gap_on_the_timeline() {
        let dir = tempfile::tempdir().expect("temporary folder");
        let path = dir.path().join("track.clock");
        // Nothing was playing between 10 ms and 5 s, so the loopback device delivered nothing.
        write(&path, &[entry(0, 0, 480), entry(480, 5_000_000, 480)], &[]);
        let map = ClockMap::read(&path).expect("read");

        assert_eq!(
            map.gaps(1_000),
            vec![Gap {
                frame: 480,
                from_us: 10_000,
                to_us: 5_000_000
            }]
        );
        // The frames stay contiguous in the file; only their times jump.
        assert_eq!(map.time_of_frame(479), Some(9_979));
        assert_eq!(map.time_of_frame(480), Some(5_000_000));
        assert_eq!(map.frames(), 960);
    }

    #[test]
    fn a_file_that_is_not_a_clock_map_is_refused() {
        let dir = tempfile::tempdir().expect("temporary folder");
        let path = dir.path().join("track.clock");
        fs::write(&path, b"not a clock map at all").expect("write");

        assert!(matches!(
            ClockMap::read(&path),
            Err(AudioError::ClockInvalid { .. })
        ));
    }
}
