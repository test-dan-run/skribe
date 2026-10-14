import { useId } from "react";

import type { CommandError, SnapshotSummary } from "../../ipc";
import { Button } from "../../ui/Button";
import { ConfirmPanel, ConfirmTrigger, useConfirmation } from "../../ui/Confirmation";
import { ErrorDetails, ErrorNotice } from "../../ui/ErrorNotice";
import { formatBytes, formatTimestamp } from "../../ui/format";
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

      {recoveryError !== null && <ErrorNotice error={recoveryError} />}

      <p className="progress" role="status">
        {pending === null ? "" : progressText[pending]}
      </p>

      {canRetry && (
        <Button unavailable={busy} onClick={onRetry}>
          Try again
        </Button>
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
        <ul className="plain-list">
          {snapshots.map((snapshot) => (
            <SnapshotRow
              key={snapshot.name}
              snapshot={snapshot}
              busy={busy}
              onRestore={() => {
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
  onRestore: () => void;
}

function SnapshotRow({ snapshot, busy, onRestore }: SnapshotRowProps) {
  const confirmation = useConfirmation();
  const taken = formatTimestamp(snapshot.createdAtUnixMs);

  return (
    <li className="row">
      <div className="row__summary">
        <div>
          <p className="row__title">Snapshot from {taken}</p>
          <p className="row__meta">
            Database schema {snapshot.schemaVersion} · {formatBytes(snapshot.sizeBytes)} ·{" "}
            <code>{snapshot.name}</code>
          </p>
        </div>
        <ConfirmTrigger
          confirmation={confirmation}
          label={`Restore the snapshot from ${taken}`}
          unavailable={busy}
        >
          Restore…
        </ConfirmTrigger>
      </div>
      <ConfirmPanel
        confirmation={confirmation}
        question="Replace the library’s database with this snapshot?"
        confirmLabel="Restore this snapshot"
        onConfirm={onRestore}
      >
        <p>
          The current database file is moved aside and kept, not deleted. Anything saved after{" "}
          {taken} is not in this snapshot. Audio files are not changed.
        </p>
      </ConfirmPanel>
    </li>
  );
}
