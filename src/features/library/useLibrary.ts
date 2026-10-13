import { useCallback, useEffect, useReducer } from "react";

import { call } from "../../ipc";
import type {
  CommandError,
  CommandResult,
  LibraryInfo,
  LibraryStatus,
  SnapshotSummary,
} from "../../ipc";

/** Which recovery is running, if any. */
export type PendingRecovery = "retry" | "restore" | null;

export type LibraryView =
  | { phase: "opening" }
  | { phase: "ready"; library: LibraryInfo }
  | {
      phase: "unavailable";
      /** Why the library is closed. Stays in place until the library opens. */
      error: CommandError;
      snapshots: SnapshotSummary[];
      pending: PendingRecovery;
      /** Why the last recovery attempt did not run, shown alongside `error`. */
      recoveryError: CommandError | null;
    };

export interface LibraryController {
  view: LibraryView;
  retry: () => void;
  restoreSnapshot: (snapshotName: string) => void;
}

type Action =
  | { type: "status"; result: CommandResult<LibraryStatus> }
  | { type: "recovery_started"; recovery: Exclude<PendingRecovery, null> }
  | { type: "recovery_finished"; result: CommandResult<LibraryStatus> };

function fromStatus(status: LibraryStatus): LibraryView {
  if (status.state === "ready") {
    return { phase: "ready", library: status.library };
  }
  return {
    phase: "unavailable",
    error: status.error,
    snapshots: status.snapshots,
    pending: null,
    recoveryError: null,
  };
}

function reduce(view: LibraryView, action: Action): LibraryView {
  switch (action.type) {
    case "status":
      if (action.result.ok) {
        return fromStatus(action.result.value);
      }
      return {
        phase: "unavailable",
        error: action.result.error,
        snapshots: [],
        pending: null,
        recoveryError: null,
      };
    case "recovery_started":
      if (view.phase !== "unavailable") {
        return view;
      }
      return { ...view, pending: action.recovery, recoveryError: null };
    case "recovery_finished":
      if (action.result.ok) {
        return fromStatus(action.result.value);
      }
      if (view.phase !== "unavailable") {
        return view;
      }
      // The recovery did not run. Keep the original failure and its choices on screen.
      return { ...view, pending: null, recoveryError: action.result.error };
  }
}

/**
 * Tracks whether the meeting library is open and runs the recoveries the user asks for.
 * Rust decides everything; this hook only mirrors what it reports.
 */
export function useLibrary(): LibraryController {
  const [view, dispatch] = useReducer(reduce, { phase: "opening" });

  useEffect(() => {
    let current = true;
    void call("library_status").then((result) => {
      if (current) {
        dispatch({ type: "status", result });
      }
    });
    return () => {
      current = false;
    };
  }, []);

  const retry = useCallback(() => {
    dispatch({ type: "recovery_started", recovery: "retry" });
    void call("library_retry_open").then((result) => {
      dispatch({ type: "recovery_finished", result });
    });
  }, []);

  const restoreSnapshot = useCallback((snapshotName: string) => {
    dispatch({ type: "recovery_started", recovery: "restore" });
    void call("library_restore_snapshot", { snapshotName }).then((result) => {
      dispatch({ type: "recovery_finished", result });
    });
  }, []);

  return { view, retry, restoreSnapshot };
}
