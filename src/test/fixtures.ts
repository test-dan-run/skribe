import type {
  CommandError,
  LibraryInfo,
  LibraryStatus,
  MeetingDetail,
  LiveTrack,
  MeetingSummary,
  RecordingDevices,
  RecordingStatus,
  Settings,
  SnapshotSummary,
  TrackSummary,
} from "../ipc";

export const library: LibraryInfo = {
  libraryId: "0192f0c8-7b1a-7c3e-9d2f-1a2b3c4d5e6f",
  formatVersion: 1,
  schemaVersion: 3,
  createdAtUnixMs: 1_791_234_567_000,
  createdByAppVersion: "0.1.0",
  locations: {
    library: "C:\\Users\\sam\\AppData\\Local\\com.skribe.app\\library",
    models: "C:\\Users\\sam\\AppData\\Local\\com.skribe.app\\models",
    temp: "C:\\Users\\sam\\AppData\\Local\\com.skribe.app\\temp",
  },
  openNotices: [],
};

export const ready: LibraryStatus = { state: "ready", library };

export const snapshot: SnapshotSummary = {
  name: "skribe-v1-1791234567000.sqlite",
  schemaVersion: 1,
  createdAtUnixMs: 1_791_234_567_000,
  sizeBytes: 20_480,
};

export const corrupt: CommandError = {
  code: "database_corrupt",
  message: "The library's database is damaged and cannot be opened safely.",
  retained: "The damaged file was left exactly as it was found.",
  recovery: "Restore a snapshot. The damaged file is moved aside and kept, not deleted.",
  actions: ["restore_snapshot", "retry_open"],
  detail: "/library/db/skribe.sqlite failed its integrity check: file is not a database",
};

export const inUse: CommandError = {
  code: "library_in_use",
  message: "Your library is already open in another Skribe window.",
  retained: "Nothing was changed.",
  recovery: "Close the other Skribe window, then try again.",
  actions: ["retry_open"],
  detail: null,
};

export const snapshotGone: CommandError = {
  code: "snapshot_not_found",
  message: "That snapshot is no longer in the library's backups folder.",
  retained: "Nothing was changed.",
  recovery: "Choose a different snapshot.",
  actions: [],
  detail: null,
};

export function unavailable(error: CommandError, snapshots: SnapshotSummary[] = []): LibraryStatus {
  return { state: "unavailable", error, snapshots };
}

export function meeting(overrides: Partial<MeetingSummary> = {}): MeetingSummary {
  return {
    id: "0192f0c8-7b1a-7c3e-9d2f-000000000001",
    title: "Weekly sync",
    plannedStartUnixMs: null,
    recordingState: "setup",
    processingState: "none",
    minutesState: "none",
    revision: 4,
    createdAtUnixMs: 1_791_234_567_000,
    updatedAtUnixMs: 1_791_234_567_000,
    trashedAtUnixMs: null,
    agendaItemCount: 0,
    recordingStartedAtUnixMs: null,
    recordingDurationUs: null,
    ...overrides,
  };
}

/** A meeting with one agenda item per title. Item IDs follow from the titles. */
export function detail(
  agenda: string[] = [],
  overrides: Partial<MeetingSummary> = {},
  extra: Partial<MeetingDetail> = {},
): MeetingDetail {
  return {
    meeting: meeting({ agendaItemCount: agenda.length, ...overrides }),
    agenda: agenda.map((title) => ({ id: `item-${title}`, title })),
    agendaEditable: true,
    tracks: [],
    ...extra,
  };
}

export const conflict: CommandError = {
  code: "revision_conflict",
  message: "This was changed after it was loaded here, so your change was not saved.",
  retained: "The saved version was kept. Your change was not applied to it.",
  recovery: "Reload to see the saved version, then make your change again.",
  actions: ["reload"],
  detail: "the meeting is at revision 5, not the expected revision 4",
};

export const invalidTitle: CommandError = {
  code: "invalid_input",
  message: "Give the meeting a title.",
  retained: "Nothing was changed.",
  recovery: "Check the value and try again.",
  actions: [],
  detail: null,
};

export const settings: Settings = {
  sectionShortcut: "Ctrl+Shift+F9",
  defaultSectionShortcut: "Ctrl+Shift+F9",
  sectionShortcutRevision: 0,
};

export function ok<T>(value: T): { ok: true; value: T } {
  return { ok: true, value };
}

export function failed(error: CommandError): { ok: false; error: CommandError } {
  return { ok: false, error };
}

export const idle: RecordingStatus = { phase: "idle" };

export function liveTrack(overrides: Partial<LiveTrack> = {}): LiveTrack {
  return {
    source: "microphone",
    deviceName: "Headset Microphone",
    format: "48000 Hz, 1 channel, 32-bit float",
    capturedUs: 62_000_000,
    droppedFrames: 0,
    level: 0.4,
    capturing: true,
    problem: null,
    stoppedAtUs: null,
    ...overrides,
  };
}

export function recordingStatus(
  meetingId: string,
  tracks: LiveTrack[] = [
    liveTrack(),
    liveTrack({ source: "system", deviceName: "Speakers", format: "48000 Hz, 2 channels, 32-bit float" }),
  ],
): RecordingStatus {
  return { phase: "recording", meetingId, synthetic: false, elapsedUs: 62_500_000, tracks };
}

export function trackSummary(overrides: Partial<TrackSummary> = {}): TrackSummary {
  return {
    id: "0192f0c8-7b1a-7c3e-9d2f-0000000000a1",
    source: "microphone",
    deviceName: "Headset Microphone",
    format: "48000 Hz, 1 channel, 32-bit float",
    finalisation: "finalised",
    health: "healthy",
    problem: null,
    stoppedAtUs: null,
    startUs: 1_200,
    audioUs: 3_723_000_000,
    sizeBytes: 714_816_000,
    droppedFrames: 0,
    sha256: "ab".repeat(32),
    ...overrides,
  };
}

export const noMicrophone: CommandError = {
  code: "capture_device_missing",
  message: "Windows has no default microphone.",
  retained: "Nothing was recorded and the meeting was not changed.",
  recovery:
    "Connect or enable the device, make it the default in Windows sound settings, then try again.",
  actions: [],
  detail: "microphone capture failed (NoDevice): 0x80070490",
};

/** Two microphones and two playback devices, with nothing chosen yet. */
export const recordingDevices: RecordingDevices = {
  revision: 0,
  captureName: "Windows audio (WASAPI, shared mode)",
  synthetic: false,
  microphone: {
    choice: { kind: "default" },
    choiceAvailable: true,
    available: [
      { id: "mic-laptop", name: "Laptop Microphone", isDefault: true, isCommunicationsDefault: false },
      { id: "mic-headset", name: "Headset Microphone", isDefault: false, isCommunicationsDefault: true },
    ],
    unavailable: null,
  },
  system: {
    choice: { kind: "default" },
    choiceAvailable: true,
    available: [
      { id: "out-speakers", name: "Speakers", isDefault: true, isCommunicationsDefault: false },
      { id: "out-headset", name: "Headset Earphones", isDefault: false, isCommunicationsDefault: true },
    ],
    unavailable: null,
  },
};
