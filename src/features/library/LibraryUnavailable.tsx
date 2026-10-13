import { useEffect, useId, useRef, useState } from "react";

import type { CommandError, SnapshotSummary } from "../../ipc";
import { ErrorDetails } from "./ErrorDetails";
import { formatBytes, formatTimestamp } from "./format";
import type { PendingRecovery } from "./useLibrary";

interface LibraryUnavailableProps {
  error: CommandError;
  snapshots: SnapshotSummary[];
  pending: PendingRecovery;
  recoveryError: CommandError | null;
  onRetry: () => void;
  onRestore: (snapshotName: string) => void;
}

const progressText: Record<Exclude<PendingRecovery, null>, string> = {
  retry: "Trying to open your library…",
  restore: "Restoring the snapshot…",
};

/** The recovery screen: why the library is closed, what was kept and what can be done. */
export function LibraryUnavailable({
  error,
  snapshots,
  pending,
  recoveryError,
  onRetry,
  onRestore,
}: LibraryUnavailableProps) {
  const headingId = useId();
  const busy = pending !== null;
  const canRetry = error.actions.includes("retry_open");
  const canRestore = error.actions.includes("restore_snapshot");

  return (
    <section className="panel" aria-labelledby={headingId}>
      <h2 id={headingId}>Skribe could not open your library</h2>
      <p className="lead">{error.message}</p>
      <dl className="facts">
        <dt>What was kept</dt>
        <dd>{error.retained}</dd>
        <dt>What you can do</dt>
        <dd>{error.recovery}</dd>
      </dl>

      {recoveryError !== null && (
        <div className="notice notice--error" role="alert">
          <p>
            <strong>{recoveryError.message}</strong>
          </p>
          <p>{recoveryError.retained}</p>
          <p>{recoveryError.recovery}</p>
          <ErrorDetails error={recoveryError} />
        </div>
      )}

      <p className="progress" role="status">
        {pending === null ? "" : progressText[pending]}
      </p>

      {canRetry && (
        <button
          type="button"
          className="button"
          aria-disabled={busy}
          onClick={() => {
            if (!busy) {
              onRetry();
            }
          }}
        >
          Try again
        </button>
      )}

      {canRestore && <SnapshotRestore snapshots={snapshots} busy={busy} onRestore={onRestore} />}

      <ErrorDetails error={error} />
    </section>
  );
}

interface SnapshotRestoreProps {
  snapshots: SnapshotSummary[];
  busy: boolean;
  onRestore: (snapshotName: string) => void;
}

function SnapshotRestore({ snapshots, busy, onRestore }: SnapshotRestoreProps) {
  const headingId = useId();
  const [confirming, setConfirming] = useState<string | null>(null);

  return (
    <section className="snapshots" aria-labelledby={headingId}>
      <h3 id={headingId}>Restore a snapshot</h3>
      <p>
        A snapshot is a copy of the library&rsquo;s database that Skribe takes just before it
        updates the library. It holds no audio and no models.
      </p>
      {snapshots.length === 0 ? (
        <p className="empty-note">
          No snapshots were found in the library&rsquo;s backups folder, so there is nothing to
          restore from.
        </p>
      ) : (
        <ul className="snapshot-list">
          {snapshots.map((snapshot) => (
            <SnapshotRow
              key={snapshot.name}
              snapshot={snapshot}
              busy={busy}
              confirming={confirming === snapshot.name}
              onAsk={() => {
                setConfirming(snapshot.name);
              }}
              onCancel={() => {
                setConfirming(null);
              }}
              onConfirm={() => {
                setConfirming(null);
                onRestore(snapshot.name);
              }}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

interface SnapshotRowProps {
  snapshot: SnapshotSummary;
  busy: boolean;
  confirming: boolean;
  onAsk: () => void;
  onCancel: () => void;
  onConfirm: () => void;
}

function SnapshotRow({ snapshot, busy, confirming, onAsk, onCancel, onConfirm }: SnapshotRowProps) {
  const confirmId = useId();
  const questionId = useId();
  const askButton = useRef<HTMLButtonElement>(null);
  const cancelButton = useRef<HTMLButtonElement>(null);
  const taken = formatTimestamp(snapshot.createdAtUnixMs);

  // Opening the confirmation moves focus to the safe choice.
  useEffect(() => {
    if (confirming) {
      cancelButton.current?.focus();
    }
  }, [confirming]);

  const close = (then: () => void) => {
    then();
    askButton.current?.focus();
  };

  return (
    <li className="snapshot">
      <div className="snapshot__summary">
        <div>
          <p className="snapshot__title">Snapshot from {taken}</p>
          <p className="snapshot__meta">
            Database schema {snapshot.schemaVersion} · {formatBytes(snapshot.sizeBytes)} ·{" "}
            <code>{snapshot.name}</code>
          </p>
        </div>
        <button
          ref={askButton}
          type="button"
          className="button"
          aria-label={`Restore the snapshot from ${taken}`}
          aria-expanded={confirming}
          aria-controls={confirmId}
          aria-disabled={busy}
          onClick={() => {
            if (!busy && !confirming) {
              onAsk();
            }
          }}
        >
          Restore…
        </button>
      </div>
      <div id={confirmId}>
        {confirming && (
          <div className="confirm" role="group" aria-labelledby={questionId}>
            <p id={questionId}>
              <strong>Replace the library&rsquo;s database with this snapshot?</strong>
            </p>
            <p>
              The current database file is moved aside and kept, not deleted. Anything saved
              after {taken} is not in this snapshot. Audio files are not changed.
            </p>
            <div className="confirm__actions">
              <button
                ref={cancelButton}
                type="button"
                className="button"
                onClick={() => {
                  close(onCancel);
                }}
              >
                Cancel
              </button>
              <button
                type="button"
                className="button button--primary"
                onClick={() => {
                  close(onConfirm);
                }}
              >
                Restore this snapshot
              </button>
            </div>
          </div>
        )}
      </div>
    </li>
  );
}
