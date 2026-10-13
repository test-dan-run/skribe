import type { ReactNode } from "react";

import type { LibraryInfo } from "../../ipc";
import { LibraryUnavailable } from "./LibraryUnavailable";
import { useLibrary } from "./useLibrary";

/**
 * Shows the rest of the app only once the meeting library is open. Until then it says the
 * library is opening, or shows the recovery screen.
 */
export function LibraryGate({ children }: { children: (library: LibraryInfo) => ReactNode }) {
  const { view, retry, restoreSnapshot } = useLibrary();

  switch (view.phase) {
    case "opening":
      return (
        <div className="panel">
          <p className="progress" role="status">
            Opening your library…
          </p>
          <p className="hint">
            Skribe checks the library before using it. After an update this can take a little
            longer.
          </p>
        </div>
      );
    case "unavailable":
      return (
        <LibraryUnavailable
          error={view.error}
          snapshots={view.snapshots}
          pending={view.pending}
          recoveryError={view.recoveryError}
          onRetry={retry}
          onRestore={restoreSnapshot}
        />
      );
    case "ready":
      return children(view.library);
  }
}
