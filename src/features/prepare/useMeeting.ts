import { useEffect, useRef, useState } from "react";

import { call } from "../../ipc";
import type { CommandError, CommandResult, MeetingDetail } from "../../ipc";

export type MeetingView =
  | { phase: "loading" }
  | { phase: "failed"; error: CommandError }
  | {
      phase: "ready";
      detail: MeetingDetail;
      /** A change is being saved. */
      busy: boolean;
      /** Why the last change was not saved. Stays until the next change or a reload. */
      error: CommandError | null;
      /** What the last change did, for a screen reader. */
      announcement: string;
    };

export interface MeetingController {
  view: MeetingView;
  /** Loads the saved meeting again, replacing what is on screen. */
  reload: () => void;
  /** Each change resolves to whether it was saved. */
  save: (title: string, plannedStartUnixMs: number | null) => Promise<boolean>;
  addItem: (title: string) => Promise<boolean>;
  renameItem: (itemId: string, title: string) => Promise<boolean>;
  removeItem: (itemId: string) => Promise<boolean>;
  /** Moves an item one place up (-1) or down (1). The whole new order is saved at once. */
  moveItem: (itemId: string, offset: -1 | 1) => Promise<boolean>;
  /** Replaces the meeting with a newer copy another part of the app received. */
  adopt: (detail: MeetingDetail) => void;
}

/**
 * One meeting being prepared. Every change quotes the revision last loaded, so a change made
 * against an out-of-date copy is refused by Rust and reported here instead of overwriting.
 */
export function useMeeting(meetingId: string): MeetingController {
  const [view, setView] = useState<MeetingView>({ phase: "loading" });
  const inFlight = useRef(false);

  const show = (result: CommandResult<MeetingDetail>) => {
    setView(
      result.ok
        ? { phase: "ready", detail: result.value, busy: false, error: null, announcement: "" }
        : { phase: "failed", error: result.error },
    );
  };

  useEffect(() => {
    let current = true;
    void call("meeting_get", { meetingId }).then((result) => {
      if (current) {
        show(result);
      }
    });
    return () => {
      current = false;
    };
  }, [meetingId]);

  const change = async (
    request: (detail: MeetingDetail) => Promise<CommandResult<MeetingDetail>> | null,
    announce: (saved: MeetingDetail) => string,
  ): Promise<boolean> => {
    if (view.phase !== "ready" || inFlight.current) {
      return false;
    }
    const pending = request(view.detail);
    if (pending === null) {
      return false;
    }
    inFlight.current = true;
    setView({ ...view, busy: true, error: null, announcement: "" });
    const result = await pending;
    inFlight.current = false;
    setView((current) => {
      if (current.phase !== "ready") {
        return current;
      }
      return result.ok
        ? {
            phase: "ready",
            detail: result.value,
            busy: false,
            error: null,
            announcement: announce(result.value),
          }
        : { ...current, busy: false, error: result.error };
    });
    return result.ok;
  };

  const target = (detail: MeetingDetail) => ({
    meetingId: detail.meeting.id,
    expectedRevision: detail.meeting.revision,
  });

  return {
    view,
    reload: () => {
      void call("meeting_get", { meetingId }).then(show);
    },
    adopt: (detail) => {
      setView((current) => (current.phase === "ready" ? { ...current, detail } : current));
    },
    save: (title, plannedStartUnixMs) =>
      change(
        (detail) => call("meeting_update", { ...target(detail), title, plannedStartUnixMs }),
        () => "Meeting details saved.",
      ),
    addItem: (title) =>
      change(
        (detail) => call("agenda_add", { ...target(detail), title }),
        (saved) => `Added as item ${String(saved.agenda.length)} of ${String(saved.agenda.length)}.`,
      ),
    renameItem: (itemId, title) =>
      change(
        (detail) => call("agenda_edit", { ...target(detail), itemId, title }),
        () => "Agenda item renamed.",
      ),
    removeItem: (itemId) =>
      change(
        (detail) => call("agenda_remove", { ...target(detail), itemId }),
        () => "Agenda item removed.",
      ),
    moveItem: (itemId, offset) =>
      change(
        (detail) => {
          const order = detail.agenda.map((item) => item.id);
          const from = order.indexOf(itemId);
          const to = from + offset;
          if (from < 0 || to < 0 || to >= order.length) {
            return null;
          }
          order.splice(to, 0, ...order.splice(from, 1));
          return call("agenda_reorder", { ...target(detail), itemIds: order });
        },
        (saved) => {
          const position = saved.agenda.findIndex((item) => item.id === itemId) + 1;
          return `Moved to position ${String(position)} of ${String(saved.agenda.length)}.`;
        },
      ),
  };
}
