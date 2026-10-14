import { useEffect, useRef, useState } from "react";

import { call, listen } from "../../ipc";
import type {
  CommandError,
  CommandResult,
  MeetingDetail,
  PreflightReport,
  RecordingStatus,
} from "../../ipc";

/** Which recording action is running, if any. */
export type RecordingAction = "checking" | "starting" | "stopping" | "recovering" | null;

export interface RecordingView {
  /** What the recording service is doing. `null` until it has been read once. */
  status: RecordingStatus | null;
  /** The result of the last device check. */
  preflight: PreflightReport | null;
  action: RecordingAction;
  /**
   * Why the last action did not happen. Status updates never clear it: it stays until the
   * next action is attempted.
   */
  error: CommandError | null;
}

export interface RecordingController {
  view: RecordingView;
  check: () => void;
  start: () => void;
  stop: () => void;
  recover: () => void;
}

interface Callbacks {
  /** The meeting changed in Rust and its saved form should be loaded again. */
  onMeetingStale: () => void;
  /** Rust returned the meeting's new form. */
  onMeeting: (detail: MeetingDetail) => void;
}

/**
 * Recording for one meeting. Rust owns the session, the clock and every decision; this hook
 * mirrors the status it pushes and forwards what the user asks for.
 */
export function useRecording(meetingId: string, callbacks: Callbacks): RecordingController {
  const [view, setView] = useState<RecordingView>({
    status: null,
    preflight: null,
    action: null,
    error: null,
  });
  const inFlight = useRef(false);

  useEffect(() => {
    let current = true;
    const setStatus = (status: RecordingStatus) => {
      if (current) {
        setView((previous) => ({ ...previous, status }));
      }
    };
    const stop = listen("recording-status", setStatus);
    void call("recording_status").then((result) => {
      if (result.ok) {
        // An event that arrived first is newer than this answer.
        setView((previous) =>
          current && previous.status === null ? { ...previous, status: result.value } : previous,
        );
      } else if (current) {
        setView((previous) => ({ ...previous, error: result.error }));
      }
    });
    return () => {
      current = false;
      stop();
    };
  }, []);

  const run = async <T>(
    action: Exclude<RecordingAction, null>,
    request: () => Promise<CommandResult<T>>,
    done: (value: T, previous: RecordingView) => RecordingView,
  ): Promise<T | null> => {
    if (inFlight.current) {
      return null;
    }
    inFlight.current = true;
    setView((previous) => ({ ...previous, action, error: null }));
    const result = await request();
    inFlight.current = false;
    setView((previous) =>
      result.ok
        ? { ...done(result.value, previous), action: null, error: null }
        : { ...previous, action: null, error: result.error },
    );
    return result.ok ? result.value : null;
  };

  return {
    view,
    check: () => {
      void run(
        "checking",
        () => call("recording_preflight", { meetingId }),
        (preflight, previous) => ({ ...previous, preflight }),
      );
    },
    start: () => {
      void run(
        "starting",
        () => call("recording_start", { meetingId }),
        (status, previous) => ({ ...previous, status, preflight: null }),
      ).then(() => {
        // Started or refused, the meeting's saved state is what Rust says it is now.
        callbacks.onMeetingStale();
      });
    },
    stop: () => {
      void run(
        "stopping",
        () => call("recording_stop", { meetingId }),
        (_, previous) => ({ ...previous, status: { phase: "idle" } }),
      ).then((detail) => {
        if (detail === null) {
          callbacks.onMeetingStale();
        } else {
          callbacks.onMeeting(detail);
        }
      });
    },
    recover: () => {
      void run(
        "recovering",
        () => call("recording_recover", { meetingId }),
        (_, previous) => previous,
      ).then((detail) => {
        if (detail === null) {
          callbacks.onMeetingStale();
        } else {
          callbacks.onMeeting(detail);
        }
      });
    },
  };
}

export interface LiveRecording {
  /** The recording that is starting, running or being saved, if there is one. */
  meetingId: string | null;
  /** Why the window refused to close, when it just did. */
  closeBlocked: string | null;
  dismissCloseBlocked: () => void;
}

/** App-wide view of whether a recording is live, for navigation and the close notice. */
export function useLiveRecording(): LiveRecording {
  const [meetingId, setMeetingId] = useState<string | null>(null);
  const [closeBlocked, setCloseBlocked] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    let heard = false;
    const show = (status: RecordingStatus) => {
      if (!current) {
        return;
      }
      const live = status.phase === "idle" ? null : status.meetingId;
      setMeetingId(live);
      if (live === null) {
        // Nothing is being recorded any more, so nothing stops the window from closing.
        setCloseBlocked(null);
      }
    };
    const stopStatus = listen("recording-status", (status) => {
      heard = true;
      show(status);
    });
    const stopBlocked = listen("close-blocked", (blocked) => {
      if (current) {
        setCloseBlocked(blocked.message);
      }
    });
    void call("recording_status").then((result) => {
      if (result.ok && !heard) {
        show(result.value);
      }
    });
    return () => {
      current = false;
      stopStatus();
      stopBlocked();
    };
  }, []);

  return {
    meetingId,
    closeBlocked,
    dismissCloseBlocked: () => {
      setCloseBlocked(null);
    },
  };
}
