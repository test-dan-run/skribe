import { useEffect, useRef, useState } from "react";

import { call } from "../../ipc";
import type { CommandError, CommandResult, MeetingDetail, MeetingList } from "../../ipc";

export type MeetingsView =
  | { phase: "loading" }
  | { phase: "failed"; error: CommandError }
  | {
      phase: "ready";
      list: MeetingList;
      /** A change is being saved. */
      busy: boolean;
      /** Why the last change did not happen. Stays until the next change is attempted. */
      error: CommandError | null;
      /** What the last change did, for a screen reader. */
      announcement: string;
    };

export interface MeetingsController {
  view: MeetingsView;
  reload: () => void;
  /** Creates a meeting and resolves to it, or to `null` when it could not be created. */
  create: (title: string, plannedStartUnixMs: number | null) => Promise<MeetingDetail | null>;
  trash: (meetingId: string) => void;
  restore: (meetingId: string) => void;
  deletePermanently: (meetingId: string) => void;
}

/** The meeting list and the trash. Rust owns both; this hook mirrors what it reports. */
export function useMeetings(): MeetingsController {
  const [view, setView] = useState<MeetingsView>({ phase: "loading" });
  const inFlight = useRef(false);

  const load = () => {
    void call("meetings_list").then((result) => {
      setView(
        result.ok
          ? { phase: "ready", list: result.value, busy: false, error: null, announcement: "" }
          : { phase: "failed", error: result.error },
      );
    });
  };

  useEffect(() => {
    let current = true;
    void call("meetings_list").then((result) => {
      if (current) {
        setView(
          result.ok
            ? { phase: "ready", list: result.value, busy: false, error: null, announcement: "" }
            : { phase: "failed", error: result.error },
        );
      }
    });
    return () => {
      current = false;
    };
  }, []);

  /** Runs one change at a time. A failure leaves the list as it was and is shown beside it. */
  const change = async <T>(
    request: () => Promise<CommandResult<T>>,
    apply: (list: MeetingList, value: T) => MeetingList,
    announcement: string,
  ): Promise<T | null> => {
    if (inFlight.current) {
      return null;
    }
    inFlight.current = true;
    setView((current) =>
      current.phase === "ready" ? { ...current, busy: true, error: null, announcement: "" } : current,
    );
    const result = await request();
    inFlight.current = false;
    setView((current) => {
      if (current.phase !== "ready") {
        return current;
      }
      return result.ok
        ? {
            ...current,
            list: apply(current.list, result.value),
            busy: false,
            error: null,
            announcement,
          }
        : { ...current, busy: false, error: result.error };
    });
    return result.ok ? result.value : null;
  };

  const replace = (_: MeetingList, list: MeetingList) => list;

  return {
    view,
    reload: () => {
      setView({ phase: "loading" });
      load();
    },
    create: (title, plannedStartUnixMs) =>
      change(
        () => call("meeting_create", { title, plannedStartUnixMs }),
        (list, created) => ({ ...list, meetings: [created.meeting, ...list.meetings] }),
        "Meeting created.",
      ),
    trash: (meetingId) => {
      void change(
        () => call("meeting_trash", { meetingId }),
        replace,
        "Meeting moved to the trash.",
      );
    },
    restore: (meetingId) => {
      void change(
        () => call("meeting_restore", { meetingId }),
        replace,
        "Meeting restored from the trash.",
      );
    },
    deletePermanently: (meetingId) => {
      void change(
        () => call("meeting_delete_permanently", { meetingId }),
        replace,
        "Meeting permanently deleted.",
      );
    },
  };
}
