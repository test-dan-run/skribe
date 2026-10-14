//! Which devices recordings are made from: listing what is connected and saving the choice.

use serde::{Deserialize, Serialize};
use skribe_audio::{DeviceChoices, ListedDevice, Source, choose};
use skribe_store::{DeviceSetting, Library, RecordingDeviceSettings, Stored};

use super::{RecordingService, stored};
use crate::{CommandError, open_library};

/// Longest device identifier accepted from the frontend. Windows identifiers are under 100
/// characters.
const MAX_DEVICE_ID_LEN: usize = 512;

/// Which device a track is recorded from, as saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeviceChoice {
    /// Whatever Windows' default device is when recording starts.
    Default,
    /// Whatever Windows' default communications device is when recording starts. Meeting
    /// apps commonly use it, and it can differ from the default device.
    Communications,
    /// One specific device, and no other. `name` is its name when it was chosen.
    Device { id: String, name: String },
}

/// Which device to record a track from, as the frontend asks for it. A specific device is
/// named by identifier only; Rust looks its name up among the connected devices.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DeviceChoiceArg {
    Default,
    Communications,
    Device { device_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AvailableDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub is_communications_default: bool,
}

/// One track's device choice and what it can currently be chosen from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SourceDevices {
    pub choice: DeviceChoice,
    /// Whether the choice names a device that is connected right now. When it does not,
    /// recording is refused; no other device is recorded in its place.
    pub choice_available: bool,
    /// The connected devices. Empty when the list could not be read.
    pub available: Vec<AvailableDevice>,
    /// Why the list could not be read.
    pub unavailable: Option<CommandError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RecordingDevices {
    /// Quote this when changing the devices. Zero until they are first saved.
    pub revision: u32,
    /// What this build captures with.
    pub capture_name: String,
    /// The devices are generated test signals, not real devices. Development builds only.
    pub synthetic: bool,
    pub microphone: SourceDevices,
    /// The playback device whose output is recorded as system audio.
    pub system: SourceDevices,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetRecordingDevicesArgs {
    pub microphone: DeviceChoiceArg,
    pub system: DeviceChoiceArg,
    pub expected_revision: u32,
}

fn audio_choice(setting: &DeviceSetting) -> skribe_audio::DeviceChoice {
    match setting {
        DeviceSetting::Default => skribe_audio::DeviceChoice::Default,
        DeviceSetting::Communications => skribe_audio::DeviceChoice::Communications,
        DeviceSetting::Device { id, .. } => skribe_audio::DeviceChoice::Specific(id.clone()),
    }
}

pub(super) fn choice(setting: &DeviceSetting) -> DeviceChoice {
    match setting {
        DeviceSetting::Default => DeviceChoice::Default,
        DeviceSetting::Communications => DeviceChoice::Communications,
        DeviceSetting::Device { id, name } => DeviceChoice::Device {
            id: id.clone(),
            name: name.clone(),
        },
    }
}

/// The saved choices, in the form the capture layer takes them.
pub(super) fn saved_choices(
    library: &Library,
) -> Result<(RecordingDeviceSettings, DeviceChoices), CommandError> {
    let settings = stored(library.recording_devices())?.value;
    let choices = DeviceChoices {
        microphone: audio_choice(&settings.microphone),
        system: audio_choice(&settings.system),
    };
    Ok((settings, choices))
}

impl RecordingService {
    fn listed(&self, source: Source) -> Result<Vec<ListedDevice>, CommandError> {
        self.backend
            .devices(source)
            .map_err(|error| CommandError::from(&error))
    }

    fn source_devices(&self, source: Source, setting: &DeviceSetting) -> SourceDevices {
        let (listed, unavailable) = match self.listed(source) {
            Ok(listed) => (listed, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        SourceDevices {
            choice: choice(setting),
            choice_available: choose(source, &listed, &audio_choice(setting)).is_ok(),
            available: listed
                .into_iter()
                .map(|listed| AvailableDevice {
                    id: listed.device.id,
                    name: listed.device.name,
                    is_default: listed.is_default,
                    is_communications_default: listed.is_communications_default,
                })
                .collect(),
            unavailable,
        }
    }

    fn describe_devices(&self, saved: &Stored<RecordingDeviceSettings>) -> RecordingDevices {
        let info = self.backend.info();
        RecordingDevices {
            revision: saved.revision,
            capture_name: info.name.to_owned(),
            synthetic: info.synthetic,
            microphone: self.source_devices(Source::Microphone, &saved.value.microphone),
            system: self.source_devices(Source::System, &saved.value.system),
        }
    }

    /// `recording_devices`: the saved choice for each track and the devices connected now.
    pub fn devices(&self) -> Result<RecordingDevices, CommandError> {
        let library = open_library(&self.library)?;
        let saved = stored(library.recording_devices())?;
        Ok(self.describe_devices(&saved))
    }

    /// Turns what the frontend asked for into what is saved. A specific device must be
    /// connected now, so its name comes from the system and not from the frontend; the one
    /// exception is the device already saved, which may be kept while it is unplugged.
    fn setting(
        &self,
        source: Source,
        asked: &DeviceChoiceArg,
        current: &DeviceSetting,
    ) -> Result<DeviceSetting, CommandError> {
        let device_id = match asked {
            DeviceChoiceArg::Default => return Ok(DeviceSetting::Default),
            DeviceChoiceArg::Communications => return Ok(DeviceSetting::Communications),
            DeviceChoiceArg::Device { device_id } => device_id,
        };
        if device_id.is_empty()
            || device_id.len() > MAX_DEVICE_ID_LEN
            || device_id.chars().any(char::is_control)
        {
            return Err(CommandError::invalid_input("That is not a device."));
        }
        if matches!(current, DeviceSetting::Device { id, .. } if id == device_id) {
            return Ok(current.clone());
        }
        self.listed(source)?
            .into_iter()
            .find(|listed| listed.device.id == *device_id)
            .map(|listed| DeviceSetting::Device {
                id: listed.device.id,
                name: listed.device.name,
            })
            .ok_or_else(|| {
                CommandError::invalid_input(
                    "That device is no longer connected. Choose a device from the list.",
                )
            })
    }

    /// `recording_set_devices`: saves the device for each track. It takes effect at the next
    /// recording; one that is running keeps the devices it started with.
    pub fn set_devices(
        &self,
        args: &SetRecordingDevicesArgs,
    ) -> Result<RecordingDevices, CommandError> {
        let library = open_library(&self.library)?;
        let current = stored(library.recording_devices())?.value;
        let chosen = RecordingDeviceSettings {
            microphone: self.setting(Source::Microphone, &args.microphone, &current.microphone)?,
            system: self.setting(Source::System, &args.system, &current.system)?,
        };
        let saved = stored(library.set_recording_devices(args.expected_revision, chosen))?;
        Ok(self.describe_devices(&saved))
    }
}

/// `recording_devices`
pub fn devices(service: &RecordingService) -> Result<RecordingDevices, CommandError> {
    service.devices()
}

/// `recording_set_devices`
pub fn set_devices(
    service: &RecordingService,
    args: &SetRecordingDevicesArgs,
) -> Result<RecordingDevices, CommandError> {
    service.set_devices(args)
}
