import type { RecordingState } from "../../ipc";

/** How a meeting's recording state is named on screen. */
export const recordingStateLabel: Record<RecordingState, string> = {
  setup: "Not recorded yet",
  ready: "Starting to record",
  recording: "Recording",
  finalising: "Saving the recording",
  recorded: "Recorded",
  interrupted: "Recording interrupted",
  failed: "Recording failed",
};
