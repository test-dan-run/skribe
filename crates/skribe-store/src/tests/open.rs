use std::fs;

use rusqlite::Connection;

use crate::fsutil;
use crate::migrate::MigrationReport;
use crate::testutil::{APP_VERSION, Sandbox, copy_tree, read, v1, v2_items};
use crate::{LIBRARY_FORMAT_VERSION, StoreError};

#[test]
fn creates_a_new_library_with_a_matching_identity() {
    let sandbox = Sandbox::new();
    let library = sandbox.open(&[v1()]).expect("open");

    assert!(library.open_report().created);
    assert_eq!(
        library.open_report().migration,
        Some(MigrationReport {
            from: 0,
            to: 1,
            snapshot: None
        })
    );
    assert_eq!(library.manifest().format_version, LIBRARY_FORMAT_VERSION);

    let description = library.describe().expect("describe");
    assert_eq!(
        description.library_id,
        library.manifest().library_id.to_string()
    );
    assert_eq!(description.schema_version, 1);
    assert_eq!(description.created_by_app_version, APP_VERSION);
    assert!(description.created_at_unix_ms > 0);

    for folder in ["db", "audio", "derived", "backups", "trash"] {
        assert!(sandbox.roots.library().join(folder).is_dir(), "{folder}");
    }
    assert!(sandbox.roots.models().is_dir());
    assert!(sandbox.roots.temp().is_dir());
}

#[test]
fn uses_write_ahead_logging_durable_commits_and_foreign_keys() {
    let sandbox = Sandbox::new();
    let library = sandbox.open(&[v1()]).expect("open");
    let pragmas = library
        .database()
        .call(|conn| {
            let read = |name: &str| -> rusqlite::Result<String> {
                conn.pragma_query_value(None, name, |row| {
                    row.get::<_, rusqlite::types::Value>(0)
                        .map(|value| format!("{value:?}"))
                })
            };
            Ok((
                read("journal_mode"),
                read("synchronous"),
                read("foreign_keys"),
            ))
        })
        .expect("worker");
    assert_eq!(pragmas.0.as_deref(), Ok("Text(\"wal\")"));
    assert_eq!(pragmas.1.as_deref(), Ok("Integer(2)"), "synchronous = FULL");
    assert_eq!(pragmas.2.as_deref(), Ok("Integer(1)"));
}

#[test]
fn database_work_runs_on_the_dedicated_worker_thread() {
    let sandbox = Sandbox::new();
    let library = sandbox.open(&[v1()]).expect("open");
    let worker = library
        .database()
        .call(|_| Ok(std::thread::current().name().map(str::to_owned)))
        .expect("worker");
    assert_eq!(worker.as_deref(), Some("skribe-db"));
    assert_ne!(std::thread::current().name(), Some("skribe-db"));
}

#[test]
fn reopening_keeps_the_identity_and_migrates_nothing() {
    let sandbox = Sandbox::new();
    let first = sandbox.open(&[v1()]).expect("open");
    let id = first.manifest().library_id;
    drop(first);

    let second = sandbox.open(&[v1()]).expect("reopen");
    assert_eq!(second.manifest().library_id, id);
    assert!(!second.open_report().created);
    assert_eq!(second.open_report().migration, None);
    assert_eq!(second.open_report().recovery, None);
}

#[test]
fn a_second_open_is_refused_while_the_library_is_in_use() {
    let sandbox = Sandbox::new();
    let first = sandbox.open(&[v1()]).expect("open");

    let second = sandbox.open(&[v1()]);
    assert!(
        matches!(second, Err(StoreError::LibraryLocked { .. })),
        "{second:?}"
    );

    drop(first);
    sandbox.open(&[v1()]).expect("open after release");
}

#[test]
fn refuses_a_database_that_belongs_to_another_library() {
    let ours = Sandbox::new();
    let theirs = Sandbox::new();
    ours.create_v1();
    theirs.create_v1();
    fs::copy(theirs.roots.manifest_path(), ours.roots.manifest_path()).expect("swap manifest");
    let before = read(&ours.db_path());

    let result = ours.open(&[v1()]);

    assert!(
        matches!(result, Err(StoreError::LibraryIdMismatch { .. })),
        "{result:?}"
    );
    assert_eq!(read(&ours.db_path()), before);
}

#[test]
fn refuses_to_invent_an_identity_for_an_existing_database() {
    let sandbox = Sandbox::new();
    sandbox.create_v1();
    fs::remove_file(sandbox.roots.manifest_path()).expect("remove manifest");

    let result = sandbox.open(&[v1()]);

    assert!(
        matches!(result, Err(StoreError::ManifestMissing { .. })),
        "{result:?}"
    );
    assert!(!sandbox.roots.manifest_path().exists());
}

#[test]
fn refuses_an_unreadable_manifest_and_leaves_it_alone() {
    let sandbox = Sandbox::new();
    sandbox.create_v1();
    fs::write(sandbox.roots.manifest_path(), b"{ not json").expect("damage manifest");

    let result = sandbox.open(&[v1()]);

    assert!(
        matches!(result, Err(StoreError::ManifestInvalid { .. })),
        "{result:?}"
    );
    assert_eq!(read(&sandbox.roots.manifest_path()), b"{ not json");
}

#[test]
fn refuses_a_manifest_with_a_malformed_library_id() {
    let sandbox = Sandbox::new();
    sandbox.create_v1();
    let json = r#"{"library_id":"../other","format_version":1,"created_by_app_version":"1"}"#;
    fs::write(sandbox.roots.manifest_path(), json).expect("write manifest");

    let result = sandbox.open(&[v1()]);
    assert!(
        matches!(result, Err(StoreError::ManifestInvalid { .. })),
        "{result:?}"
    );
}

#[test]
fn refuses_a_library_written_by_a_newer_format() {
    let sandbox = Sandbox::new();
    sandbox.create_v1();
    let manifest = String::from_utf8(read(&sandbox.roots.manifest_path())).expect("utf-8");
    let newer = manifest.replace("\"format_version\": 1", "\"format_version\": 2");
    assert_ne!(manifest, newer);
    fs::write(sandbox.roots.manifest_path(), newer).expect("write manifest");

    let result = sandbox.open(&[v1()]);
    assert!(
        matches!(
            result,
            Err(StoreError::LibraryFormatTooNew {
                found: 2,
                supported: 1
            })
        ),
        "{result:?}"
    );
}

#[test]
fn refuses_a_database_with_a_newer_schema_without_changing_it() {
    let sandbox = Sandbox::new();
    drop(
        sandbox
            .open(&[v1(), v2_items()])
            .expect("create at schema 2"),
    );
    let before = read(&sandbox.db_path());

    let result = sandbox.open(&[v1()]);

    assert!(
        matches!(
            result,
            Err(StoreError::DatabaseTooNew {
                found: 2,
                supported: 1
            })
        ),
        "{result:?}"
    );
    assert_eq!(read(&sandbox.db_path()), before);
}

#[test]
fn refuses_to_start_an_empty_database_over_remaining_library_content() {
    let sandbox = Sandbox::new();
    sandbox.create_v1();
    fs::remove_file(sandbox.db_path()).expect("remove database");
    let meeting = sandbox.roots.library().join("audio").join("some-meeting");
    fs::create_dir(&meeting).expect("leave audio behind");

    let result = sandbox.open(&[v1()]);

    assert!(
        matches!(result, Err(StoreError::DatabaseMissing { .. })),
        "{result:?}"
    );
    assert!(!sandbox.db_path().exists());
}

#[test]
fn finishes_creating_a_library_whose_database_was_never_written() {
    let sandbox = Sandbox::new();
    let id = sandbox.open(&[v1()]).expect("open").manifest().library_id;
    // A crash after library.json was written but before the first transaction committed.
    fs::remove_file(sandbox.db_path()).expect("remove database");
    fs::write(sandbox.db_path(), b"").expect("empty database file");

    let library = sandbox.open(&[v1()]).expect("open");

    assert_eq!(library.manifest().library_id, id);
    assert_eq!(
        library.describe().expect("describe").library_id,
        id.to_string()
    );
}

#[test]
fn refuses_a_database_it_does_not_recognise() {
    let sandbox = Sandbox::new();
    sandbox.create_v1();
    fs::remove_file(sandbox.db_path()).expect("remove database");
    let foreign = Connection::open(sandbox.db_path()).expect("create foreign database");
    foreign
        .execute_batch("CREATE TABLE someone_elses (id INTEGER PRIMARY KEY);")
        .expect("table");
    drop(foreign);
    let before = read(&sandbox.db_path());

    let result = sandbox.open(&[v1()]);

    assert!(
        matches!(result, Err(StoreError::DatabaseUnrecognised { .. })),
        "{result:?}"
    );
    assert_eq!(read(&sandbox.db_path()), before);
}

#[test]
fn reopens_after_a_shutdown_that_left_the_write_ahead_log_behind() {
    let running = Sandbox::new();
    let library = running.open(&[v1(), v2_items()]).expect("open");
    library
        .database()
        .call(|conn| {
            conn.execute(
                "INSERT INTO test_items (body) VALUES ('saved before the crash')",
                [],
            )
            .map_err(StoreError::sqlite("test"))
        })
        .expect("insert");

    // The files as a killed process leaves them: committed work still in the log.
    let crashed = Sandbox::new();
    copy_tree(running.roots.library(), crashed.roots.library());
    let log = fsutil::with_suffix(&crashed.db_path(), "-wal");
    assert!(
        fs::metadata(&log).is_ok_and(|metadata| metadata.len() > 0),
        "log holds the commit"
    );

    let reopened = crashed.open(&[v1(), v2_items()]).expect("reopen");

    assert_eq!(
        reopened.manifest().library_id,
        library.manifest().library_id
    );
    let saved: String = reopened
        .database()
        .call(|conn| {
            conn.query_row("SELECT body FROM test_items", [], |row| row.get(0))
                .map_err(StoreError::sqlite("test"))
        })
        .expect("read");
    assert_eq!(saved, "saved before the crash");
}
