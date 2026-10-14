import { useId, useState } from "react";
import type { SyntheticEvent } from "react";

import type { MeetingSummary } from "../../ipc";
import { Button } from "../../ui/Button";
import { ConfirmPanel, ConfirmTrigger, useConfirmation } from "../../ui/Confirmation";
import { ErrorNotice } from "../../ui/ErrorNotice";
import { formatTimestamp } from "../../ui/format";
import { fromInputValue } from "../prepare/plannedTime";
import { recordingStateLabel } from "./labels";
import { useMeetings } from "./useMeetings";
import type { MeetingsController } from "./useMeetings";

interface MeetingsHomeProps {
  onOpen: (meetingId: string) => void;
}

/** The meeting list: create a meeting, open one, and manage the trash. */
export function MeetingsHome({ onOpen }: MeetingsHomeProps) {
  const headingId = useId();
  const controller = useMeetings();
  const { view } = controller;

  return (
    <section className="stack" aria-labelledby={headingId}>
      <h2 id={headingId}>Meetings</h2>
      {view.phase === "loading" && (
        <p className="progress" role="status">
          Loading your meetings…
        </p>
      )}
      {view.phase === "failed" && (
        <ErrorNotice error={view.error}>
          <Button onClick={controller.reload}>Try again</Button>
        </ErrorNotice>
      )}
      {view.phase === "ready" && (
        <>
          <NewMeetingForm controller={controller} busy={view.busy} onOpen={onOpen} />
          {view.error !== null && <ErrorNotice error={view.error} />}
          <p className="visually-hidden" role="status">
            {view.announcement}
          </p>
          {view.list.meetings.length === 0 ? (
            <div className="empty">
              <p className="empty__title">No meetings yet</p>
              <p>Create a meeting above to set its title, planned time and agenda.</p>
            </div>
          ) : (
            <ul className="plain-list" aria-label="Meetings">
              {view.list.meetings.map((meeting) => (
                <MeetingRow
                  key={meeting.id}
                  meeting={meeting}
                  busy={view.busy}
                  onOpen={onOpen}
                  onTrash={controller.trash}
                />
              ))}
            </ul>
          )}
          <Trash controller={controller} trashed={view.list.trashed} busy={view.busy} />
        </>
      )}
    </section>
  );
}

interface NewMeetingFormProps {
  controller: MeetingsController;
  busy: boolean;
  onOpen: (meetingId: string) => void;
}

function NewMeetingForm({ controller, busy, onOpen }: NewMeetingFormProps) {
  const titleId = useId();
  const plannedId = useId();
  const hintId = useId();
  const [title, setTitle] = useState("");
  const [planned, setPlanned] = useState("");
  const [problem, setProblem] = useState("");

  const submit = (event: SyntheticEvent) => {
    event.preventDefault();
    if (busy) {
      return;
    }
    const plannedStartUnixMs = fromInputValue(planned);
    if (plannedStartUnixMs === "invalid") {
      setProblem("Enter the planned date and time in full, or leave it empty.");
      return;
    }
    setProblem("");
    void controller.create(title, plannedStartUnixMs).then((created) => {
      if (created !== null) {
        onOpen(created.meeting.id);
      }
    });
  };

  return (
    <form className="panel form" onSubmit={submit} aria-label="New meeting">
      <div className="field">
        <label htmlFor={titleId}>Title</label>
        <input
          id={titleId}
          type="text"
          value={title}
          maxLength={200}
          autoComplete="off"
          onChange={(event) => {
            setTitle(event.target.value);
          }}
        />
      </div>
      <div className="field">
        <label htmlFor={plannedId}>Planned time (optional)</label>
        <input
          id={plannedId}
          type="datetime-local"
          value={planned}
          aria-describedby={hintId}
          onChange={(event) => {
            setPlanned(event.target.value);
          }}
        />
        <p id={hintId} className="hint">
          In your computer&rsquo;s time zone.
        </p>
      </div>
      {problem !== "" && (
        <p className="field__problem" role="alert">
          {problem}
        </p>
      )}
      <Button type="submit" variant="primary" unavailable={busy}>
        Create meeting
      </Button>
    </form>
  );
}

function describe(meeting: MeetingSummary): string {
  const parts = [
    meeting.plannedStartUnixMs === null
      ? "No planned time"
      : `Planned for ${formatTimestamp(meeting.plannedStartUnixMs)}`,
    meeting.agendaItemCount === 1
      ? "1 agenda item"
      : `${String(meeting.agendaItemCount)} agenda items`,
    recordingStateLabel[meeting.recordingState],
  ];
  return parts.join(" · ");
}

interface MeetingRowProps {
  meeting: MeetingSummary;
  busy: boolean;
  onOpen: (meetingId: string) => void;
  onTrash: (meetingId: string) => void;
}

function MeetingRow({ meeting, busy, onOpen, onTrash }: MeetingRowProps) {
  const confirmation = useConfirmation();
  return (
    <li className="row">
      <div className="row__summary">
        <div>
          <p className="row__title">{meeting.title}</p>
          <p className="row__meta">{describe(meeting)}</p>
        </div>
        <div className="row__actions">
          <Button
            aria-label={`Open ${meeting.title}`}
            onClick={() => {
              onOpen(meeting.id);
            }}
          >
            Open
          </Button>
          <ConfirmTrigger
            confirmation={confirmation}
            label={`Move ${meeting.title} to the trash`}
            unavailable={busy}
          >
            Move to trash…
          </ConfirmTrigger>
        </div>
      </div>
      <ConfirmPanel
        confirmation={confirmation}
        question={`Move “${meeting.title}” to the trash?`}
        confirmLabel="Move to trash"
        onConfirm={() => {
          onTrash(meeting.id);
        }}
      >
        <p>
          Nothing is deleted. The meeting, its agenda and any recording stay in your library and
          can be restored from the trash.
        </p>
      </ConfirmPanel>
    </li>
  );
}

interface TrashProps {
  controller: MeetingsController;
  trashed: MeetingSummary[];
  busy: boolean;
}

function Trash({ controller, trashed, busy }: TrashProps) {
  const headingId = useId();
  return (
    <section className="stack" aria-labelledby={headingId}>
      <h3 id={headingId}>Trash</h3>
      {trashed.length === 0 ? (
        <p className="empty-note">The trash is empty.</p>
      ) : (
        <ul className="plain-list" aria-label="Meetings in the trash">
          {trashed.map((meeting) => (
            <TrashedRow key={meeting.id} meeting={meeting} busy={busy} controller={controller} />
          ))}
        </ul>
      )}
    </section>
  );
}

interface TrashedRowProps {
  meeting: MeetingSummary;
  busy: boolean;
  controller: MeetingsController;
}

function TrashedRow({ meeting, busy, controller }: TrashedRowProps) {
  const confirmation = useConfirmation();
  const hasRecording = meeting.recordingState !== "setup";
  return (
    <li className="row">
      <div className="row__summary">
        <div>
          <p className="row__title">{meeting.title}</p>
          <p className="row__meta">
            {meeting.trashedAtUnixMs === null
              ? describe(meeting)
              : `Moved to the trash ${formatTimestamp(meeting.trashedAtUnixMs)} · ${describe(meeting)}`}
          </p>
        </div>
        <div className="row__actions">
          <Button
            aria-label={`Restore ${meeting.title}`}
            unavailable={busy}
            onClick={() => {
              controller.restore(meeting.id);
            }}
          >
            Restore
          </Button>
          <ConfirmTrigger
            confirmation={confirmation}
            label={`Permanently delete ${meeting.title}`}
            unavailable={busy}
            variant="danger"
          >
            Delete permanently…
          </ConfirmTrigger>
        </div>
      </div>
      <ConfirmPanel
        confirmation={confirmation}
        question={`Permanently delete “${meeting.title}”?`}
        confirmLabel="Delete permanently"
        variant="danger"
        onConfirm={() => {
          controller.deletePermanently(meeting.id);
        }}
      >
        <p>
          This cannot be undone. The meeting and its agenda are deleted from your library
          {hasRecording ? ", together with its audio files" : ""}. Database snapshots taken
          earlier may still contain the meeting&rsquo;s title and agenda.
        </p>
      </ConfirmPanel>
    </li>
  );
}
