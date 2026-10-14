//! What a track's capture thread and writer thread report while recording, read by the
//! session for the live health display.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Something that happened to a track and is worth keeping with the recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthEvent {
    /// Timeline time the event applies from.
    pub at_us: i64,
    pub kind: HealthEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthEventKind {
    /// The capture buffer was full, so these frames were not recorded. The clock map shows
    /// the gap; capture continued.
    Overflow { frames: u64, until_us: i64 },
    /// The device disappeared or was disabled. The track ends here; nothing takes its place.
    DeviceLost { detail: String },
    /// The device reported an error other than going away.
    CaptureFailed { detail: String },
    /// The track's files could not be written, for example because the disk is full.
    WriteFailed { detail: String },
}

/// Why a track stopped before the recording did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackStop {
    pub at_us: i64,
    pub reason: StopReason,
    /// Technical detail such as a device error code. Never audio content.
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    DeviceLost,
    CaptureFailed,
    WriteFailed,
}

/// Most health events kept between two collections. Events beyond it are counted, not kept,
/// so a track in trouble cannot grow memory without bound.
const MAX_BUFFERED_EVENTS: usize = 256;

#[derive(Debug, Default)]
struct Notes {
    stop: Option<TrackStop>,
    events: Vec<HealthEvent>,
    events_discarded: u64,
}

#[derive(Debug, Default)]
pub(crate) struct TrackLive {
    frames: AtomicU64,
    dropped_frames: AtomicU64,
    /// Bits of the loudest level since it was last read. Non-negative floats order the same
    /// way as their bits.
    peak_bits: AtomicU32,
    notes: Mutex<Notes>,
}

impl TrackLive {
    fn notes(&self) -> MutexGuard<'_, Notes> {
        self.notes.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn add_frames(&self, frames: u32) {
        self.frames.fetch_add(u64::from(frames), Ordering::Relaxed);
    }

    pub(crate) fn add_dropped(&self, frames: u32) {
        self.dropped_frames
            .fetch_add(u64::from(frames), Ordering::Relaxed);
    }

    pub(crate) fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }

    pub(crate) fn dropped_frames(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
    }

    pub(crate) fn raise_peak(&self, level: f32) {
        self.peak_bits
            .fetch_max(level.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// The loudest level since the last call.
    pub(crate) fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak_bits.swap(0, Ordering::Relaxed))
    }

    pub(crate) fn event(&self, at_us: i64, kind: HealthEventKind) {
        let mut notes = self.notes();
        if notes.events.len() < MAX_BUFFERED_EVENTS {
            notes.events.push(HealthEvent { at_us, kind });
        } else {
            notes.events_discarded += 1;
        }
    }

    /// Records why the track stopped. The first reason stands: a write failure that follows
    /// a lost device does not replace it.
    pub(crate) fn stop(&self, at_us: i64, reason: StopReason, detail: &str) {
        let kind = match reason {
            StopReason::DeviceLost => HealthEventKind::DeviceLost {
                detail: detail.to_owned(),
            },
            StopReason::CaptureFailed => HealthEventKind::CaptureFailed {
                detail: detail.to_owned(),
            },
            StopReason::WriteFailed => HealthEventKind::WriteFailed {
                detail: detail.to_owned(),
            },
        };
        self.event(at_us, kind);
        let mut notes = self.notes();
        if notes.stop.is_none() {
            notes.stop = Some(TrackStop {
                at_us,
                reason,
                detail: detail.to_owned(),
            });
        }
    }

    pub(crate) fn stopped(&self) -> Option<TrackStop> {
        self.notes().stop.clone()
    }

    pub(crate) fn take_events(&self) -> Vec<HealthEvent> {
        std::mem::take(&mut self.notes().events)
    }

    pub(crate) fn events_discarded(&self) -> u64 {
        self.notes().events_discarded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_peak_is_the_loudest_level_since_it_was_last_read() {
        let live = TrackLive::default();
        live.raise_peak(0.25);
        live.raise_peak(0.75);
        live.raise_peak(0.5);
        assert_eq!(live.take_peak(), 0.75);
        assert_eq!(live.take_peak(), 0.0);
    }

    #[test]
    fn the_first_reason_a_track_stopped_is_the_one_reported() {
        let live = TrackLive::default();
        live.stop(5_000, StopReason::DeviceLost, "unplugged");
        live.stop(6_000, StopReason::WriteFailed, "later");

        let stop = live.stopped().expect("stopped");
        assert_eq!((stop.at_us, stop.reason), (5_000, StopReason::DeviceLost));
        // Both are still kept as events.
        assert_eq!(live.take_events().len(), 2);
        assert!(live.take_events().is_empty());
    }

    #[test]
    fn events_are_bounded_and_the_overflow_is_counted() {
        let live = TrackLive::default();
        for index in 0..(MAX_BUFFERED_EVENTS as i64 + 10) {
            live.event(
                index,
                HealthEventKind::Overflow {
                    frames: 1,
                    until_us: index + 1,
                },
            );
        }
        assert_eq!(live.take_events().len(), MAX_BUFFERED_EVENTS);
        assert_eq!(live.events_discarded(), 10);
    }
}
