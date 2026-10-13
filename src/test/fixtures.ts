import type { CommandError, LibraryInfo, LibraryStatus, SnapshotSummary } from "../ipc";

export const library: LibraryInfo = {
  libraryId: "0192f0c8-7b1a-7c3e-9d2f-1a2b3c4d5e6f",
  formatVersion: 1,
  schemaVersion: 1,
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
