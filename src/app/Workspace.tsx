import { useEffect, useId, useRef, useState } from "react";

import { LibraryNotices, StorageDetails } from "../features/library/LibrarySummary";
import { MeetingsHome } from "../features/meetings/MeetingsHome";
import { PrepareMeeting } from "../features/prepare/PrepareMeeting";
import { RecordPanel } from "../features/record/RecordPanel";
import { useLiveRecording } from "../features/record/useRecording";
import { SettingsPanel } from "../features/settings/SettingsPanel";
import type { LibraryInfo } from "../ipc";
import { Button } from "../ui/Button";

type View =
  | { name: "meetings" }
  | { name: "meeting"; meetingId: string }
  | { name: "settings" };

/** The app once the library is open: navigation between the meeting list, a meeting and settings. */
export function Workspace({ library }: { library: LibraryInfo }) {
  const [view, setView] = useState<View>({ name: "meetings" });
  // Whether the current page holds text that has not been saved, and where the user was
  // heading when that stopped them.
  const [dirty, setDirty] = useState(false);
  const [blocked, setBlocked] = useState<View | null>(null);
  const live = useLiveRecording();
  const showingLiveMeeting = view.name === "meeting" && view.meetingId === live.meetingId;

  const go = (next: View) => {
    if (dirty) {
      setBlocked(next);
    } else {
      setView(next);
    }
  };

  return (
    <>
      <nav className="nav" aria-label="Main">
        <Button
          aria-current={view.name === "settings" ? undefined : "page"}
          onClick={() => {
            go({ name: "meetings" });
          }}
        >
          Meetings
        </Button>
        <Button
          aria-current={view.name === "settings" ? "page" : undefined}
          onClick={() => {
            go({ name: "settings" });
          }}
        >
          Settings
        </Button>
      </nav>
      <LibraryNotices library={library} />
      {live.closeBlocked !== null && (
        <div className="notice notice--error" role="alert">
          <p>
            <strong>Skribe was not closed.</strong> {live.closeBlocked}
          </p>
          <div className="notice__actions">
            <Button onClick={live.dismissCloseBlocked}>Dismiss</Button>
          </div>
        </div>
      )}
      {live.meetingId !== null && !showingLiveMeeting && (
        <div className="notice">
          <p>
            <strong>A recording is in progress.</strong>
          </p>
          <div className="notice__actions">
            <Button
              onClick={() => {
                if (live.meetingId !== null) {
                  go({ name: "meeting", meetingId: live.meetingId });
                }
              }}
            >
              Go to the recording
            </Button>
          </div>
        </div>
      )}
      {blocked !== null && (
        <UnsavedChanges
          onStay={() => {
            setBlocked(null);
          }}
          onLeave={() => {
            setDirty(false);
            setView(blocked);
            setBlocked(null);
          }}
        />
      )}
      {view.name === "meetings" && (
        <MeetingsHome
          onOpen={(meetingId) => {
            go({ name: "meeting", meetingId });
          }}
        />
      )}
      {view.name === "meeting" && (
        <PrepareMeeting
          key={view.meetingId}
          meetingId={view.meetingId}
          onBack={() => {
            go({ name: "meetings" });
          }}
          onDirtyChange={setDirty}
        >
          {(detail, controller) => (
            <RecordPanel
              detail={detail}
              onMeetingStale={controller.reload}
              onMeeting={controller.adopt}
            />
          )}
        </PrepareMeeting>
      )}
      {view.name === "settings" && (
        <SettingsPanel onDirtyChange={setDirty}>
          <StorageDetails library={library} />
        </SettingsPanel>
      )}
    </>
  );
}

/** Asked before leaving a page that holds unsaved text. Focus opens on staying. */
function UnsavedChanges({ onStay, onLeave }: { onStay: () => void; onLeave: () => void }) {
  const questionId = useId();
  const stay = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    stay.current?.focus();
  }, []);

  return (
    <div className="notice" role="group" aria-labelledby={questionId}>
      <p id={questionId}>
        <strong>Leave this page without saving?</strong>
      </p>
      <p>Text you have typed on this page has not been saved and will be lost.</p>
      <div className="confirm__actions">
        <Button ref={stay} onClick={onStay}>
          Stay on this page
        </Button>
        <Button variant="danger" onClick={onLeave}>
          Leave without saving
        </Button>
      </div>
    </div>
  );
}
