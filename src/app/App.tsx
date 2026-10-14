import { LibraryGate } from "../features/library/LibraryGate";
import { ErrorBoundary } from "./ErrorBoundary";
import { Workspace } from "./Workspace";

export function App() {
  return (
    <div className="app">
      <header className="app__header">
        <h1>Skribe</h1>
      </header>
      <main className="app__main">
        <ErrorBoundary>
          <LibraryGate>{(library) => <Workspace library={library} />}</LibraryGate>
        </ErrorBoundary>
      </main>
    </div>
  );
}
