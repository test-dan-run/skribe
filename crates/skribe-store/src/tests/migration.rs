use std::fs;

use crate::error::FailedMigrationOutcome;
use crate::library::InterruptedMigrationRecovery;
use crate::migrate::{MigrationMarker, MigrationReport};
use crate::testutil::{APP_VERSION, GARBAGE, Sandbox, read, v1, v2_broken, v2_items};
use crate::{Library, StoreError, db, fsutil};

fn table_exists(library: &Library, table: &'static str) -> bool {
    library
        .database()
        .call(move |conn| {
            conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1)",
                [table],
                |row| row.get(0),
            )
            .map_err(StoreError::sqlite("test"))
        })
        .expect("worker")
}

fn write_marker(sandbox: &Sandbox, snapshot: &str) {
    MigrationMarker {
        from_version: 1,
        to_version: 2,
        snapshot: snapshot.to_owned(),
        app_version: APP_VERSION.to_owned(),
        started_at_unix_ms: fsutil::now_unix_ms(),
    }
    .write(&sandbox.marker_path())
    .expect("write marker");
}

fn backup_file_names(sandbox: &Sandbox) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(sandbox.roots.backups_dir())
        .expect("list backups")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn migrating_existing_data_takes_a_verified_snapshot_first() {
    let sandbox = Sandbox::new();
    let id = sandbox.open(&[v1()]).expect("create").manifest().library_id;

    let library = sandbox.open(&[v1(), v2_items()]).expect("migrate");

    let snapshots = Library::snapshots(&sandbox.roots).expect("list snapshots");
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].schema_version, 1);
    assert_eq!(
        library.open_report().migration,
        Some(MigrationReport {
            from: 1,
            to: 2,
            snapshot: Some(snapshots[0].name.clone())
        })
    );
    // The snapshot is one complete file holding the pre-migration data.
    assert_eq!(backup_file_names(&sandbox), vec![snapshots[0].name.clone()]);
    let snapshot_path = sandbox.roots.backups_dir().join(&snapshots[0].name);
    let inspection = db::inspect(&snapshot_path).expect("snapshot is sound");
    assert_eq!(inspection.schema_version, 1);
    assert_eq!(inspection.library_id, Some(id.to_string()));

    assert_eq!(library.describe().expect("describe").schema_version, 2);
    assert_eq!(
        library.describe().expect("describe").library_id,
        id.to_string()
    );
    assert!(table_exists(&library, "test_items"));
    assert!(!sandbox.marker_path().exists());
}

#[test]
fn a_failed_migration_rolls_back_and_leaves_the_library_usable() {
    let sandbox = Sandbox::new();
    let id = sandbox.open(&[v1()]).expect("create").manifest().library_id;

    let result = sandbox.open(&[v1(), v2_broken()]);

    let Err(StoreError::MigrationFailed {
        from,
        to,
        detail,
        outcome,
        snapshot,
    }) = result
    else {
        panic!("expected a migration failure, got {result:?}");
    };
    assert_eq!((from, to), (1, 2));
    assert!(detail.contains("test_broken"), "{detail}");
    assert_eq!(outcome, FailedMigrationOutcome::RolledBack);
    let snapshot = snapshot.expect("snapshot was taken before migrating");
    assert!(sandbox.roots.backups_dir().join(&snapshot).is_file());
    assert!(!sandbox.marker_path().exists());

    // The previous build still opens the same data, without the half-applied table.
    let library = sandbox.open(&[v1()]).expect("open at schema 1");
    assert_eq!(
        library.describe().expect("describe").library_id,
        id.to_string()
    );
    assert_eq!(library.describe().expect("describe").schema_version, 1);
    assert!(!table_exists(&library, "test_items"));
}

#[test]
fn a_migration_interrupted_before_commit_is_run_again() {
    let sandbox = Sandbox::new();
    sandbox.create_v1();
    let snapshot = sandbox.snapshot_closed_database(1);
    write_marker(&sandbox, &snapshot.name);

    let library = sandbox
        .open(&[v1(), v2_items()])
        .expect("recover and migrate");

    assert_eq!(
        library.open_report().recovery,
        Some(InterruptedMigrationRecovery::RolledBack)
    );
    assert!(matches!(
        library.open_report().migration,
        Some(MigrationReport {
            from: 1,
            to: 2,
            snapshot: Some(_)
        })
    ));
    assert_eq!(library.describe().expect("describe").schema_version, 2);
    assert!(!sandbox.marker_path().exists());
}

#[test]
fn a_migration_interrupted_after_commit_only_needs_its_marker_cleared() {
    let sandbox = Sandbox::new();
    sandbox.create_v1();
    let snapshot = sandbox.snapshot_closed_database(1);
    drop(sandbox.open(&[v1(), v2_items()]).expect("migrate"));
    write_marker(&sandbox, &snapshot.name);

    let library = sandbox.open(&[v1(), v2_items()]).expect("recover");

    assert_eq!(
        library.open_report().recovery,
        Some(InterruptedMigrationRecovery::Completed)
    );
    assert_eq!(library.open_report().migration, None);
    assert!(table_exists(&library, "test_items"));
    assert!(!sandbox.marker_path().exists());
}

#[test]
fn an_interrupted_migration_with_an_unusable_database_restores_the_snapshot() {
    let sandbox = Sandbox::new();
    let id = sandbox.open(&[v1()]).expect("create").manifest().library_id;
    let snapshot = sandbox.snapshot_closed_database(1);
    write_marker(&sandbox, &snapshot.name);
    fs::write(sandbox.db_path(), GARBAGE).expect("damage database");
    let stale_wal = fsutil::with_suffix(&sandbox.db_path(), "-wal");
    fs::write(&stale_wal, b"stale write-ahead log").expect("leave a stale log");

    let library = sandbox
        .open(&[v1(), v2_items()])
        .expect("recover and migrate");

    let Some(InterruptedMigrationRecovery::RestoredFromSnapshot {
        snapshot: restored,
        set_aside: Some(set_aside),
    }) = library.open_report().recovery.clone()
    else {
        panic!(
            "expected a restore, got {:?}",
            library.open_report().recovery
        );
    };
    assert_eq!(restored, snapshot.name);
    // The unusable file and its log are kept together, byte for byte.
    assert_eq!(read(&set_aside.join("skribe.sqlite")), GARBAGE);
    assert_eq!(
        read(&set_aside.join("skribe.sqlite-wal")),
        b"stale write-ahead log"
    );

    let description = library.describe().expect("describe");
    assert_eq!(description.library_id, id.to_string());
    assert_eq!(description.schema_version, 2);
    assert!(!sandbox.marker_path().exists());
}

#[test]
fn an_interrupted_migration_without_a_usable_snapshot_stops_and_keeps_everything() {
    let sandbox = Sandbox::new();
    sandbox.create_v1();
    write_marker(&sandbox, "skribe-v1-1.sqlite");
    fs::write(sandbox.db_path(), GARBAGE).expect("damage database");

    let result = sandbox.open(&[v1(), v2_items()]);

    let Err(error) = result else {
        panic!("expected recovery to fail")
    };
    assert!(
        matches!(error, StoreError::MigrationRecoveryFailed { .. }),
        "{error:?}"
    );
    assert!(error.offers_snapshot_restore());
    assert_eq!(read(&sandbox.db_path()), GARBAGE);
    assert!(sandbox.marker_path().exists());
    assert!(sandbox.set_aside_folders().is_empty());
}

#[test]
fn an_unreadable_marker_stops_before_the_database_is_touched() {
    let sandbox = Sandbox::new();
    sandbox.create_v1();
    fs::write(sandbox.marker_path(), b"{").expect("damage marker");
    let before = read(&sandbox.db_path());

    let result = sandbox.open(&[v1(), v2_items()]);

    assert!(
        matches!(result, Err(StoreError::MigrationRecoveryFailed { .. })),
        "{result:?}"
    );
    assert_eq!(read(&sandbox.db_path()), before);
}
