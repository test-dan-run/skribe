# Storage

Where Skribe keeps data on disk, and what it does to protect it. The code is in
`crates/skribe-store`, the only crate that touches SQLite.

## Three roots

```
<base>/library/    meeting data, owned by one library ID
<base>/models/     model and runtime packages (nothing writes here yet)
<base>/temp/       scratch space (nothing writes here yet)
```

`<base>` is the app's local data folder for the identifier `com.skribe.app`:
`%LOCALAPPDATA%\com.skribe.app` on Windows, `~/.local/share/com.skribe.app` in WSL. The
identifier fixes this location, so it must not change once a build has been distributed.

The roots are siblings. Skribe refuses to start with roots that overlap each other or sit
inside the install folder, because an update or uninstall replaces that folder. A
user-selected library folder is not implemented (decision D7).

The webview keeps its own cache in `<base>` beside these folders. It holds no meeting data.

## Inside the library

```
library/
  library.json                   library ID, folder format version, creating app version
  library.lock                   held while a Skribe process has the library open
  db/skribe.sqlite               structured data (plus -wal and -shm while open)
  db/migration-in-progress.json  present only while a migration is running
  db/set-aside-<time>/           a database that a restore replaced, kept as found
  backups/skribe-v<schema>-<time>.sqlite   database snapshots
  audio/  derived/  trash/       created empty; used by later phases
```

`<time>` is Unix time in milliseconds.

## Identity and ownership

- `library.json` and the database both record the library ID. If they differ, Skribe refuses to
  open the library and changes nothing.
- Skribe creates a new identity only for a folder with no data in it. If `library.json` is
  missing but a database, audio or snapshots exist, it stops instead.
- If the database is missing but audio, snapshots or trash exist, it stops instead of starting
  a new, empty database over them.
- Only one process can have a library open. A second one is told the library is in use.
- A library written by a newer folder format or database schema is refused unchanged.

## The database

- SQLite in write-ahead-log mode with `synchronous = FULL` and foreign keys enforced. `FULL`
  means a committed change survives power loss, not only a crash.
- One dedicated thread owns the connection. Commands run on a blocking thread pool and hand
  that thread short units of work; nothing touches SQLite on the UI thread.
- Before the database is opened for writing it is inspected over a read-only connection:
  integrity check, schema version, library ID. A read-only connection cannot checkpoint the
  log into the main file, so a damaged file is left byte for byte as it was found.

## Migrations

Migrations are forward-only and listed in `crates/skribe-store/src/migrate.rs`. Shipped
entries are never edited. For a database that already holds data:

1. Copy the database into `backups/` and verify the copy: it opens, passes the integrity
   check, is at the expected schema and belongs to this library. Only then does it get a
   snapshot name.
2. Write `db/migration-in-progress.json`, naming the snapshot.
3. Apply every pending migration in one transaction, with a foreign-key check before commit.
4. Remove the marker.

If a migration fails, the transaction rolls back. Skribe then checks that the database is
intact at its previous schema. If it is not, the snapshot is restored. Either way the library
stays closed, the error says which happened, and the snapshot is kept.

If the marker is found at startup, the previous run was interrupted:

| Database found | What Skribe does |
|----------------|------------------|
| Intact, at the new schema | Removes the marker. The migration had committed |
| Intact, at the old schema | Removes the marker and runs the migration again |
| Anything else | Restores the snapshot named in the marker, then migrates |
| Unusable, and the snapshot is missing or invalid | Stops, changes nothing, offers the other snapshots |

A new, empty database is given its schema without a snapshot, since there is nothing to lose.

## Snapshots and restore

- A snapshot is a complete copy of the database only. It contains no audio and no models, and
  it is not a backup of the library.
- For now snapshots are taken only before migrations. A user-initiated snapshot and a full
  library backup are planned for Phase 10. A library that has never been migrated has no
  snapshots, and the recovery screen says so.
- Restore is offered when the database is damaged, missing or not recognised, or when an
  interrupted migration could not be recovered. The user picks a snapshot and confirms.
- Restore verifies the snapshot first and rejects one that is damaged, from another library or
  from a newer schema. It then moves the current database and its log files together into
  `db/set-aside-<time>/` and puts a copy of the snapshot in place. Nothing is deleted.
- The frontend sends only a snapshot's file name. Rust accepts a name only if it has exactly
  the snapshot form and names a regular file in `backups/`.

## Updates and uninstall

Application updates replace only the install folder. Tauri's NSIS uninstaller removes the app
data folder only when the user ticks its option to delete application data, which is off by
default. Both behaviours come from the installer template and have not yet been verified on
Windows; that check is part of Phase 10.
