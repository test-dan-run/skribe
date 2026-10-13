import type { LibraryInfo } from "../../ipc";

/** What opening the library had to do this time, when it did anything worth saying. */
export function LibraryNotices({ library }: { library: LibraryInfo }) {
  if (library.openNotices.length === 0) {
    return null;
  }
  return (
    <section className="notice" aria-label="Library notices">
      {library.openNotices.map((notice) => (
        <p key={notice.kind}>{notice.message}</p>
      ))}
    </section>
  );
}

/** Where data is kept. Paths are shown for reference; the window cannot open or read them. */
export function StorageDetails({ library }: { library: LibraryInfo }) {
  return (
    <details className="details">
      <summary>Where Skribe keeps your data</summary>
      <dl className="facts facts--compact">
        <dt>Meeting library</dt>
        <dd>
          <code className="wrap">{library.locations.library}</code>
        </dd>
        <dt>Models</dt>
        <dd>
          <code className="wrap">{library.locations.models}</code>
        </dd>
        <dt>Temporary files</dt>
        <dd>
          <code className="wrap">{library.locations.temp}</code>
        </dd>
        <dt>Library format</dt>
        <dd>
          Folder format {library.formatVersion}, database schema {library.schemaVersion}
        </dd>
      </dl>
    </details>
  );
}
