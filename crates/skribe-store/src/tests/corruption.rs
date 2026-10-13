use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};

use crate::testutil::{GARBAGE, Sandbox, read, v1, v2_items};
use crate::{Library, StoreError};

#[test]
fn a_file_that_is_not_a_database_is_reported_and_left_untouched() {
    let sandbox = Sandbox::new();
    sandbox.create_v1();
    fs::write(sandbox.db_path(), GARBAGE).expect("damage database");

    let result = sandbox.open(&[v1()]);

    let Err(error) = result else {
        panic!("expected corruption to be reported")
    };
    assert!(
        matches!(error, StoreError::DatabaseCorrupt { .. }),
        "{error:?}"
    );
    assert!(error.offers_snapshot_restore());
    assert_eq!(read(&sandbox.db_path()), GARBAGE);
    assert!(sandbox.set_aside_folders().is_empty());
}

#[test]
fn damaged_pages_fail_the_integrity_check_and_are_left_untouched() {
    let sandbox = Sandbox::new();
    let library = sandbox.open(&[v1(), v2_items()]).expect("create");
    library
        .database()
        .call(|conn| {
            let tx = conn.transaction().map_err(StoreError::sqlite("test"))?;
            for index in 0..2_000 {
                tx.execute(
                    "INSERT INTO test_items (body) VALUES (?1)",
                    [format!("item {index} {}", "x".repeat(200))],
                )
                .map_err(StoreError::sqlite("test"))?;
            }
            tx.commit().map_err(StoreError::sqlite("test"))
        })
        .expect("insert rows");
    drop(library);

    // Overwrite pages in the middle of the table, leaving the header readable.
    let mut file = OpenOptions::new()
        .write(true)
        .open(sandbox.db_path())
        .expect("open file");
    file.seek(SeekFrom::Start(4096 * 3)).expect("seek");
    file.write_all(&[0xA5; 4096 * 4]).expect("overwrite pages");
    drop(file);
    let damaged = read(&sandbox.db_path());

    let result = sandbox.open(&[v1(), v2_items()]);

    assert!(
        matches!(result, Err(StoreError::DatabaseCorrupt { .. })),
        "{result:?}"
    );
    assert_eq!(read(&sandbox.db_path()), damaged);
}

#[test]
fn restoring_a_snapshot_keeps_the_damaged_file_and_reopens_the_library() {
    let sandbox = Sandbox::new();
    let id = sandbox.open(&[v1()]).expect("create").manifest().library_id;
    let snapshot = sandbox.snapshot_closed_database(1);
    fs::write(sandbox.db_path(), GARBAGE).expect("damage database");

    let report = Library::restore_snapshot(&sandbox.roots, &snapshot.name).expect("restore");

    assert_eq!(report.snapshot, snapshot.name);
    let set_aside = report.set_aside.expect("the damaged file was moved aside");
    assert_eq!(read(&set_aside.join("skribe.sqlite")), GARBAGE);
    // The snapshot itself stays in backups/ for another restore.
    assert!(sandbox.roots.backups_dir().join(&snapshot.name).is_file());

    let library = sandbox.open(&[v1()]).expect("open restored library");
    assert_eq!(
        library.describe().expect("describe").library_id,
        id.to_string()
    );
}

#[test]
fn restore_rejects_names_that_are_not_snapshots_in_the_backups_folder() {
    let sandbox = Sandbox::new();
    sandbox.create_v1();
    let before = read(&sandbox.db_path());

    for name in [
        "../db/skribe.sqlite",
        "skribe.sqlite",
        "/etc/passwd",
        "skribe-v1-1.sqlite/..",
    ] {
        let result = Library::restore_snapshot(&sandbox.roots, name);
        assert!(
            matches!(result, Err(StoreError::SnapshotNameInvalid)),
            "{name}: {result:?}"
        );
    }
    let missing = Library::restore_snapshot(&sandbox.roots, "skribe-v1-1.sqlite");
    assert!(
        matches!(missing, Err(StoreError::SnapshotNotFound { .. })),
        "{missing:?}"
    );

    assert_eq!(read(&sandbox.db_path()), before);
    assert!(sandbox.set_aside_folders().is_empty());
}

#[test]
fn restore_rejects_a_snapshot_from_another_library_or_a_damaged_one() {
    let ours = Sandbox::new();
    let theirs = Sandbox::new();
    ours.create_v1();
    theirs.create_v1();
    let foreign = theirs.snapshot_closed_database(1);
    fs::copy(
        theirs.roots.backups_dir().join(&foreign.name),
        ours.roots.backups_dir().join(&foreign.name),
    )
    .expect("plant foreign snapshot");
    fs::write(ours.roots.backups_dir().join("skribe-v1-2.sqlite"), GARBAGE).expect("plant garbage");
    fs::write(ours.db_path(), GARBAGE).expect("damage database");

    for name in [foreign.name.as_str(), "skribe-v1-2.sqlite"] {
        let result = Library::restore_snapshot(&ours.roots, name);
        assert!(
            matches!(result, Err(StoreError::SnapshotInvalid { .. })),
            "{name}: {result:?}"
        );
    }

    assert_eq!(read(&ours.db_path()), GARBAGE);
    assert!(ours.set_aside_folders().is_empty());
}
