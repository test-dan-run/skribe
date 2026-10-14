import { useEffect, useId, useRef, useState } from "react";
import type { ReactNode, SyntheticEvent } from "react";

import type { AgendaItem, MeetingDetail } from "../../ipc";
import { Button } from "../../ui/Button";
import { ConfirmPanel, ConfirmTrigger, useConfirmation } from "../../ui/Confirmation";
import { ErrorNotice } from "../../ui/ErrorNotice";
import { recordingStateLabel } from "../meetings/labels";
import { fromInputValue, toInputValue } from "./plannedTime";
import { useMeeting } from "./useMeeting";
import type { MeetingController } from "./useMeeting";

interface PrepareMeetingProps {
  meetingId: string;
  onBack: () => void;
  /** Reports whether the page holds text that has not been saved. */
  onDirtyChange: (dirty: boolean) => void;
  /** The recording area for this meeting, given the loaded meeting and ways to refresh it. */
  children?: (
    detail: MeetingDetail,
    controller: Pick<MeetingController, "reload" | "adopt">,
  ) => ReactNode;
}

/** One meeting: its title and planned time, and its agenda while that can still change. */
export function PrepareMeeting({ meetingId, onBack, onDirtyChange, children }: PrepareMeetingProps) {
  const controller = useMeeting(meetingId);
  const { view } = controller;

  if (view.phase === "loading") {
    return (
      <p className="progress" role="status">
        Loading the meeting…
      </p>
    );
  }
  if (view.phase === "failed") {
    return (
      <ErrorNotice error={view.error}>
        <Button onClick={controller.reload}>Try again</Button>
        <Button onClick={onBack}>Back to meetings</Button>
      </ErrorNotice>
    );
  }
  return (
    <Prepared
      controller={controller}
      detail={view.detail}
      busy={view.busy}
      onBack={onBack}
      onDirtyChange={onDirtyChange}
    >
      {view.error !== null && (
        <ErrorNotice error={view.error}>
          {view.error.actions.includes("reload") && (
            <Button onClick={controller.reload}>Load the saved version</Button>
          )}
        </ErrorNotice>
      )}
      <p className="visually-hidden" role="status">
        {view.announcement}
      </p>
      {children?.(view.detail, controller)}
    </Prepared>
  );
}

interface PreparedProps {
  controller: MeetingController;
  detail: MeetingDetail;
  busy: boolean;
  onBack: () => void;
  onDirtyChange: (dirty: boolean) => void;
  children: ReactNode;
}

function Prepared({ controller, detail, busy, onBack, onDirtyChange, children }: PreparedProps) {
  const headingId = useId();
  const [detailsDirty, setDetailsDirty] = useState(false);
  const [agendaDirty, setAgendaDirty] = useState(false);
  const dirty = detailsDirty || agendaDirty;
  const { meeting } = detail;

  useEffect(() => {
    onDirtyChange(dirty);
    return () => {
      onDirtyChange(false);
    };
  }, [dirty, onDirtyChange]);

  return (
    <section className="stack" aria-labelledby={headingId}>
      <div>
        <Button onClick={onBack}>Back to meetings</Button>
      </div>
      <h2 id={headingId}>{meeting.title}</h2>
      <p className="hint">{recordingStateLabel[meeting.recordingState]}</p>
      {meeting.trashedAtUnixMs !== null && (
        <p className="notice">
          This meeting is in the trash. Restore it from the list of meetings to change it.
        </p>
      )}
      {children}
      <DetailsForm
        // A new saved title or time replaces the draft; an agenda change does not.
        key={`${meeting.title}\n${String(meeting.plannedStartUnixMs)}`}
        controller={controller}
        detail={detail}
        busy={busy}
        onDirtyChange={setDetailsDirty}
      />
      <Agenda controller={controller} detail={detail} busy={busy} onDirtyChange={setAgendaDirty} />
    </section>
  );
}

interface SectionProps {
  controller: MeetingController;
  detail: MeetingDetail;
  busy: boolean;
  onDirtyChange: (dirty: boolean) => void;
}

function DetailsForm({ controller, detail, busy, onDirtyChange }: SectionProps) {
  const headingId = useId();
  const titleId = useId();
  const plannedId = useId();
  const hintId = useId();
  const { meeting } = detail;
  const savedPlanned = toInputValue(meeting.plannedStartUnixMs);
  const [title, setTitle] = useState(meeting.title);
  const [planned, setPlanned] = useState(savedPlanned);
  const [problem, setProblem] = useState("");
  const locked = meeting.trashedAtUnixMs !== null;
  const dirty = title !== meeting.title || planned !== savedPlanned;

  useEffect(() => {
    onDirtyChange(dirty);
  }, [dirty, onDirtyChange]);

  const submit = (event: SyntheticEvent) => {
    event.preventDefault();
    if (busy || locked || !dirty) {
      return;
    }
    const plannedStartUnixMs = fromInputValue(planned);
    if (plannedStartUnixMs === "invalid") {
      setProblem("Enter the planned date and time in full, or leave it empty.");
      return;
    }
    setProblem("");
    void controller.save(title, plannedStartUnixMs);
  };

  return (
    <form className="panel form" onSubmit={submit} aria-labelledby={headingId}>
      <h3 id={headingId}>Details</h3>
      <div className="field">
        <label htmlFor={titleId}>Title</label>
        <input
          id={titleId}
          type="text"
          value={title}
          maxLength={200}
          autoComplete="off"
          readOnly={locked}
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
          readOnly={locked}
          aria-describedby={hintId}
          onChange={(event) => {
            setPlanned(event.target.value);
          }}
        />
        <p id={hintId} className="hint">
          In your computer&rsquo;s time zone. Clear the field to remove the planned time.
        </p>
      </div>
      {problem !== "" && (
        <p className="field__problem" role="alert">
          {problem}
        </p>
      )}
      {!locked && (
        <div className="form__actions">
          <Button type="submit" variant="primary" unavailable={busy || !dirty}>
            Save details
          </Button>
          {dirty && <span className="hint">Unsaved changes</span>}
        </div>
      )}
    </form>
  );
}

type Control = "up" | "down";

function Agenda({ controller, detail, busy, onDirtyChange }: SectionProps) {
  const headingId = useId();
  const newItemId = useId();
  const [newTitle, setNewTitle] = useState("");
  const [renaming, setRenaming] = useState<string | null>(null);
  // Which move button to put focus back on once the reordered list has rendered.
  const [moved, setMoved] = useState<{ itemId: string; control: Control } | null>(null);
  const editable = detail.agendaEditable;
  const dirty = newTitle.trim() !== "" || renaming !== null;

  useEffect(() => {
    onDirtyChange(dirty);
  }, [dirty, onDirtyChange]);

  const add = (event: SyntheticEvent) => {
    event.preventDefault();
    if (busy) {
      return;
    }
    void controller.addItem(newTitle).then((saved) => {
      if (saved) {
        setNewTitle("");
      }
    });
  };

  return (
    <section className="panel" aria-labelledby={headingId}>
      <h3 id={headingId}>Agenda</h3>
      {!editable && detail.meeting.trashedAtUnixMs === null && (
        <p className="notice">
          Recording has started for this meeting, so its agenda can no longer be changed.
        </p>
      )}
      {detail.agenda.length === 0 ? (
        <p className="empty-note">
          No agenda items{editable ? " yet. Add the topics you plan to cover, in order." : "."}
        </p>
      ) : (
        <ol className="plain-list agenda">
          {detail.agenda.map((item, index) => (
            <AgendaRow
              key={item.id}
              item={item}
              index={index}
              count={detail.agenda.length}
              editable={editable}
              busy={busy}
              renaming={renaming === item.id}
              focus={moved?.itemId === item.id ? moved.control : null}
              onRenameStart={() => {
                setRenaming(item.id);
              }}
              onRenameEnd={() => {
                setRenaming(null);
              }}
              onRename={(title) => controller.renameItem(item.id, title)}
              onRemove={() => {
                void controller.removeItem(item.id);
              }}
              onMove={(control) => {
                setMoved({ itemId: item.id, control });
                void controller.moveItem(item.id, control === "up" ? -1 : 1);
              }}
            />
          ))}
        </ol>
      )}
      {editable && (
        <form className="inline-form" onSubmit={add} aria-label="Add an agenda item">
          <div className="field field--grow">
            <label htmlFor={newItemId}>New agenda item</label>
            <input
              id={newItemId}
              type="text"
              value={newTitle}
              maxLength={200}
              autoComplete="off"
              onChange={(event) => {
                setNewTitle(event.target.value);
              }}
            />
          </div>
          <Button type="submit" unavailable={busy}>
            Add item
          </Button>
        </form>
      )}
    </section>
  );
}

interface AgendaRowProps {
  item: AgendaItem;
  index: number;
  count: number;
  editable: boolean;
  busy: boolean;
  renaming: boolean;
  focus: Control | null;
  onRenameStart: () => void;
  onRenameEnd: () => void;
  onRename: (title: string) => Promise<boolean>;
  onRemove: () => void;
  onMove: (control: Control) => void;
}

function AgendaRow({
  item,
  index,
  count,
  editable,
  busy,
  renaming,
  focus,
  onRenameStart,
  onRenameEnd,
  onRename,
  onRemove,
  onMove,
}: AgendaRowProps) {
  const confirmation = useConfirmation();
  const up = useRef<HTMLButtonElement>(null);
  const down = useRef<HTMLButtonElement>(null);
  const renameButton = useRef<HTMLButtonElement>(null);
  const wasRenaming = useRef(false);
  const first = index === 0;
  const last = index === count - 1;

  // Reordering moves this row in the document, which drops focus. Put it back on the button
  // that was used, so the item can be moved again without hunting for it.
  useEffect(() => {
    if (focus === "up") {
      up.current?.focus();
    } else if (focus === "down") {
      down.current?.focus();
    }
  }, [focus, index]);

  useEffect(() => {
    if (wasRenaming.current && !renaming) {
      renameButton.current?.focus();
    }
    wasRenaming.current = renaming;
  }, [renaming]);

  return (
    <li className="row">
      <div className="row__summary">
        {renaming ? (
          <RenameForm item={item} busy={busy} onRename={onRename} onEnd={onRenameEnd} />
        ) : (
          <p className="row__title">
            <span className="agenda__number">{index + 1}.</span> {item.title}
          </p>
        )}
        {editable && !renaming && (
          <div className="row__actions">
            <Button
              ref={up}
              aria-label={`Move ${item.title} up`}
              unavailable={busy || first}
              onClick={() => {
                onMove("up");
              }}
            >
              Up
            </Button>
            <Button
              ref={down}
              aria-label={`Move ${item.title} down`}
              unavailable={busy || last}
              onClick={() => {
                onMove("down");
              }}
            >
              Down
            </Button>
            <Button
              ref={renameButton}
              aria-label={`Rename ${item.title}`}
              unavailable={busy}
              onClick={onRenameStart}
            >
              Rename
            </Button>
            <ConfirmTrigger
              confirmation={confirmation}
              label={`Remove ${item.title}`}
              unavailable={busy}
            >
              Remove…
            </ConfirmTrigger>
          </div>
        )}
      </div>
      <ConfirmPanel
        confirmation={confirmation}
        question={`Remove “${item.title}” from the agenda?`}
        confirmLabel="Remove item"
        onConfirm={onRemove}
      >
        <p>The item is removed from this meeting&rsquo;s agenda. The other items keep their order.</p>
      </ConfirmPanel>
    </li>
  );
}

interface RenameFormProps {
  item: AgendaItem;
  busy: boolean;
  onRename: (title: string) => Promise<boolean>;
  onEnd: () => void;
}

function RenameForm({ item, busy, onRename, onEnd }: RenameFormProps) {
  const inputId = useId();
  const input = useRef<HTMLInputElement>(null);
  const [title, setTitle] = useState(item.title);

  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);

  const submit = (event: SyntheticEvent) => {
    event.preventDefault();
    if (busy) {
      return;
    }
    if (title === item.title) {
      onEnd();
      return;
    }
    void onRename(title).then((saved) => {
      if (saved) {
        onEnd();
      }
    });
  };

  return (
    <form
      className="inline-form inline-form--grow"
      onSubmit={submit}
      aria-label={`Rename ${item.title}`}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          onEnd();
        }
      }}
    >
      <div className="field field--grow">
        <label htmlFor={inputId}>Agenda item title</label>
        <input
          ref={input}
          id={inputId}
          type="text"
          value={title}
          maxLength={200}
          autoComplete="off"
          onChange={(event) => {
            setTitle(event.target.value);
          }}
        />
      </div>
      <Button type="submit" variant="primary" unavailable={busy}>
        Save
      </Button>
      <Button onClick={onEnd}>Cancel</Button>
    </form>
  );
}
