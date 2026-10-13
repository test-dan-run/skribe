import { LibraryGate } from "../features/library/LibraryGate";
import { LibraryNotices, StorageDetails } from "../features/library/LibrarySummary";
import { MeetingsHome } from "../features/meetings/MeetingsHome";
import { ErrorBoundary } from "./ErrorBoundary";

export function App() {
  return (
    <div className="app">
      <header className="app__header">
        <h1>Skribe</h1>
      </header>
      <main className="app__main">
        <ErrorBoundary>
          <LibraryGate>
            {(library) => (
              <>
                <LibraryNotices library={library} />
                <MeetingsHome />
                <StorageDetails library={library} />
              </>
            )}
          </LibraryGate>
        </ErrorBoundary>
      </main>
    </div>
  );
}
