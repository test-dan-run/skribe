//! Application settings. A setting that was never saved has its default and revision zero.

use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use skribe_domain::Shortcut;

use crate::meetings::{begin, commit, now};
use crate::{Library, StoreError};

const SECTION_SHORTCUT: &str = "section_shortcut";
const RECORDING_DEVICES: &str = "recording_devices";

/// A setting's value with the revision to quote when changing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stored<T> {
    pub value: T,
    /// Zero until the setting is first saved.
    pub revision: u32,
}

/// Which device a track is recorded from, as the user chose it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeviceSetting {
    /// Whatever the system's default device is when recording starts.
    #[default]
    Default,
    /// Whatever the system's default communications device is when recording starts.
    Communications,
    /// One specific device. Its name is kept so it can still be named when it is unplugged.
    Device { id: String, name: String },
}

/// The device chosen for each of a recording's two tracks.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RecordingDeviceSettings {
    pub microphone: DeviceSetting,
    pub system: DeviceSetting,
}

fn invalid(detail: String) -> StoreError {
    StoreError::DataInvalid {
        place: "settings",
        detail,
    }
}

/// A setting's stored text and revision, or `None` when it was never saved.
fn read(conn: &Connection, key: &'static str) -> Result<Option<(String, u32)>, StoreError> {
    let row: Option<(String, i64)> = conn
        .query_row(
            "SELECT value, revision FROM settings WHERE key = ?1",
            [key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(StoreError::sqlite("read setting"))?;
    row.map(|(text, revision)| {
        let revision =
            u32::try_from(revision).map_err(|_| invalid(format!("{key} revision {revision}")))?;
        Ok((text, revision))
    })
    .transpose()
}

/// Saves a setting if it is still at the revision the caller saw.
fn write(
    tx: &Transaction<'_>,
    key: &'static str,
    expected_revision: u32,
    value: &str,
) -> Result<(), StoreError> {
    let found = read(tx, key)?.map_or(0, |(_, revision)| revision);
    if found != expected_revision {
        return Err(StoreError::RevisionConflict {
            expected: expected_revision,
            found,
        });
    }
    tx.execute(
        "INSERT INTO settings (key, value, revision, updated_at_unix_ms) \
         VALUES (?1, ?2, 1, ?3) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value, \
         revision = revision + 1, updated_at_unix_ms = excluded.updated_at_unix_ms",
        (key, value, now()),
    )
    .map_err(StoreError::sqlite("save setting"))?;
    Ok(())
}

fn read_shortcut(conn: &Connection) -> Result<Stored<Shortcut>, StoreError> {
    let Some((text, revision)) = read(conn, SECTION_SHORTCUT)? else {
        return Ok(Stored {
            value: Shortcut::DEFAULT,
            revision: 0,
        });
    };
    Ok(Stored {
        value: text
            .parse()
            .map_err(|error| invalid(format!("section shortcut {text}: {error}")))?,
        revision,
    })
}

fn read_devices(conn: &Connection) -> Result<Stored<RecordingDeviceSettings>, StoreError> {
    let Some((text, revision)) = read(conn, RECORDING_DEVICES)? else {
        return Ok(Stored {
            value: RecordingDeviceSettings::default(),
            revision: 0,
        });
    };
    Ok(Stored {
        value: serde_json::from_str(&text)
            .map_err(|error| invalid(format!("recording devices: {error}")))?,
        revision,
    })
}

impl Library {
    /// The shortcut that advances to the next section while recording.
    pub fn section_shortcut(&self) -> Result<Stored<Shortcut>, StoreError> {
        self.database().call(|conn| read_shortcut(conn))
    }

    pub fn set_section_shortcut(
        &self,
        expected_revision: u32,
        shortcut: Shortcut,
    ) -> Result<Stored<Shortcut>, StoreError> {
        self.database().call(move |conn| {
            let tx = begin(conn)?;
            write(
                &tx,
                SECTION_SHORTCUT,
                expected_revision,
                &shortcut.to_string(),
            )?;
            let saved = read_shortcut(&tx)?;
            commit(tx)?;
            Ok(saved)
        })
    }

    /// Which devices recordings are made from. Both follow the system default until chosen.
    pub fn recording_devices(&self) -> Result<Stored<RecordingDeviceSettings>, StoreError> {
        self.database().call(|conn| read_devices(conn))
    }

    pub fn set_recording_devices(
        &self,
        expected_revision: u32,
        devices: RecordingDeviceSettings,
    ) -> Result<Stored<RecordingDeviceSettings>, StoreError> {
        self.database().call(move |conn| {
            let text = serde_json::to_string(&devices)
                .map_err(|error| invalid(format!("recording devices: {error}")))?;
            let tx = begin(conn)?;
            write(&tx, RECORDING_DEVICES, expected_revision, &text)?;
            let saved = read_devices(&tx)?;
            commit(tx)?;
            Ok(saved)
        })
    }
}
