import { useId } from "react";

/**
 * The meetings area. This build has no meeting features yet, and says so rather than showing
 * controls that do nothing.
 */
export function MeetingsHome() {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId}>
      <h2 id={headingId}>Meetings</h2>
      <div className="empty">
        <p className="empty__title">No meetings yet</p>
        <p>
          This early build sets up and protects your local library. Preparing, recording and
          reviewing meetings are not available in it yet.
        </p>
      </div>
    </section>
  );
}
