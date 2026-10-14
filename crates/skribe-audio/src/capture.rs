//! The capture boundary. A backend lists a source's devices, opens the chosen one and delivers its
//! packets, in the device's native format and with the device's own timestamps, into a
//! [`CaptureSink`]. Everything behind the sink is the same for every backend.

use std::fmt;
use std::sync::Arc;

use crate::live::{StopReason, TrackLive};
use crate::ring::{PacketMeta, Producer, Pushed};
use crate::{AudioFormat, CaptureError, CaptureErrorKind, SessionClock, Source};

/// The device a track was recorded from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The system's identifier for the device.
    pub id: String,
    /// The name the system shows for it.
    pub name: String,
}

/// A device a source can be recorded from, as the system lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedDevice {
    pub device: DeviceInfo,
    /// The system's default device for this source.
    pub is_default: bool,
    /// The system's default device for calls and meetings, which can be a different one.
    pub is_communications_default: bool,
}

/// Which device to record a source from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DeviceChoice {
    /// Whatever the system's default device is when recording starts.
    #[default]
    Default,
    /// Whatever the system's default communications device is when recording starts. Meeting
    /// apps commonly play and record through it.
    Communications,
    /// The device with this identifier, and no other.
    Specific(String),
}

/// The device chosen for each of a recording's two tracks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeviceChoices {
    pub microphone: DeviceChoice,
    pub system: DeviceChoice,
}

impl DeviceChoices {
    pub fn of(&self, source: Source) -> &DeviceChoice {
        match source {
            Source::Microphone => &self.microphone,
            Source::System => &self.system,
        }
    }
}

/// Picks the device `choice` names from the devices present, by position in `listed`.
///
/// A specific device that is not present is an error. It is never replaced by the default or
/// by any other device: what gets recorded is what was chosen, or nothing.
pub fn choose(
    source: Source,
    listed: &[ListedDevice],
    choice: &DeviceChoice,
) -> Result<usize, CaptureError> {
    let (found, kind, missing) = match choice {
        DeviceChoice::Default => (
            listed.iter().position(|device| device.is_default),
            CaptureErrorKind::NoDevice,
            "there is no default device".to_owned(),
        ),
        DeviceChoice::Communications => (
            listed
                .iter()
                .position(|device| device.is_communications_default),
            CaptureErrorKind::NoDevice,
            "there is no default communications device".to_owned(),
        ),
        DeviceChoice::Specific(id) => (
            listed.iter().position(|device| device.device.id == *id),
            CaptureErrorKind::ChosenDeviceMissing,
            "the chosen device is not among the active devices".to_owned(),
        ),
    };
    found.ok_or_else(|| CaptureError::new(source, kind, missing))
}

/// What this build captures with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendInfo {
    pub name: &'static str,
    /// A generated test signal, not real devices. Never true in a release build.
    pub synthetic: bool,
}

pub trait CaptureBackend: Send + Sync + fmt::Debug {
    fn info(&self) -> BackendInfo;

    /// The active devices `source` can be recorded from.
    fn devices(&self, source: Source) -> Result<Vec<ListedDevice>, CaptureError>;

    /// Opens the device `choice` names for `source` and reads its native format, without
    /// capturing yet. This is the device half of preflight: it fails with the specific reason
    /// when the device is missing, refused or in a format that cannot be recorded.
    fn open(
        &self,
        source: Source,
        choice: &DeviceChoice,
    ) -> Result<Box<dyn CaptureStream>, CaptureError>;
}

/// One opened device. Dropping it stops capture.
pub trait CaptureStream: Send + fmt::Debug {
    fn device(&self) -> &DeviceInfo;

    fn format(&self) -> AudioFormat;

    /// Starts delivering packets into `sink`.
    fn start(&mut self, sink: CaptureSink) -> Result<(), CaptureError>;

    /// Stops delivery and releases the sink. No packet arrives after this returns.
    fn stop(&mut self);
}

/// Where a capture thread puts what the device delivers. It never blocks on the disk: packets
/// go into the track's fixed-size buffer, and a packet that does not fit is counted and
/// dropped. Dropping the sink ends the track.
pub struct CaptureSink {
    source: Source,
    format: AudioFormat,
    clock: SessionClock,
    producer: Producer,
    live: Arc<TrackLive>,
}

impl fmt::Debug for CaptureSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CaptureSink")
            .field("source", &self.source)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

impl CaptureSink {
    pub(crate) fn new(
        source: Source,
        format: AudioFormat,
        clock: SessionClock,
        producer: Producer,
        live: Arc<TrackLive>,
    ) -> Self {
        Self {
            source,
            format,
            clock,
            producer,
            live,
        }
    }

    pub fn source(&self) -> Source {
        self.source
    }

    pub fn format(&self) -> AudioFormat {
        self.format
    }

    /// The recording's clock, for converting device timestamps to timeline time.
    pub fn clock(&self) -> SessionClock {
        self.clock
    }

    /// Delivers one packet. `timeline_us` is the device's timestamp for its first frame on the
    /// recording's timeline; `flags` are clock map flags. `data` must hold exactly `frames`
    /// frames in the stream's format.
    pub fn packet(&self, timeline_us: i64, frames: u32, flags: u32, data: &[u8]) {
        let expected = u64::from(frames) * u64::from(self.format.bytes_per_frame());
        if data.len() as u64 != expected {
            self.failed(&format!(
                "a packet of {frames} frames held {} bytes, not {expected}",
                data.len()
            ));
            return;
        }
        if frames == 0 {
            return;
        }
        let meta = PacketMeta {
            timeline_us,
            frames,
            flags,
        };
        let duration_us = self.format.duration_us(u64::from(frames));
        match self.producer.push(meta, data, duration_us) {
            Pushed::Stored => self.live.add_frames(frames),
            Pushed::Dropped => self.live.add_dropped(frames),
            Pushed::Closed => {}
        }
    }

    /// The device disappeared. The track ends at this point on the timeline; the other track
    /// is not affected and nothing is substituted.
    pub fn device_lost(&self, detail: &str) {
        self.live
            .stop(self.clock.now_us(), StopReason::DeviceLost, detail);
    }

    /// The device reported an error that ends capture.
    pub fn failed(&self, detail: &str) {
        self.live
            .stop(self.clock.now_us(), StopReason::CaptureFailed, detail);
    }
}

impl Drop for CaptureSink {
    fn drop(&mut self) {
        self.producer.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(id: &str, is_default: bool, is_communications_default: bool) -> ListedDevice {
        ListedDevice {
            device: DeviceInfo {
                id: id.to_owned(),
                name: format!("Device {id}"),
            },
            is_default,
            is_communications_default,
        }
    }

    #[test]
    fn a_choice_picks_exactly_the_device_it_names() {
        let devices = [
            listed("speakers", true, false),
            listed("headset", false, true),
            listed("monitor", false, false),
        ];
        let pick = |choice| choose(Source::System, &devices, &choice);

        assert_eq!(pick(DeviceChoice::Default), Ok(0));
        assert_eq!(pick(DeviceChoice::Communications), Ok(1));
        assert_eq!(pick(DeviceChoice::Specific("monitor".into())), Ok(2));
    }

    #[test]
    fn a_chosen_device_that_is_absent_is_an_error_never_a_substitution() {
        let devices = [listed("speakers", true, true)];

        let error = choose(
            Source::Microphone,
            &devices,
            &DeviceChoice::Specific("headset".into()),
        )
        .expect_err("not present");

        assert_eq!(error.kind, CaptureErrorKind::ChosenDeviceMissing);
        assert_eq!(error.source_name, Source::Microphone);
    }

    #[test]
    fn following_a_default_that_does_not_exist_is_an_error() {
        let devices = [listed("monitor", false, false)];
        for choice in [DeviceChoice::Default, DeviceChoice::Communications] {
            let error = choose(Source::System, &devices, &choice).expect_err("no default");
            assert_eq!(error.kind, CaptureErrorKind::NoDevice);
        }
        assert!(choose(Source::System, &[], &DeviceChoice::Default).is_err());
    }

    #[test]
    fn the_choice_for_each_source_is_kept_apart() {
        let choices = DeviceChoices {
            microphone: DeviceChoice::Specific("headset".into()),
            system: DeviceChoice::Communications,
        };
        assert_eq!(
            choices.of(Source::Microphone),
            &DeviceChoice::Specific("headset".into())
        );
        assert_eq!(choices.of(Source::System), &DeviceChoice::Communications);
        assert_eq!(
            DeviceChoices::default().of(Source::System),
            &DeviceChoice::Default
        );
    }
}
