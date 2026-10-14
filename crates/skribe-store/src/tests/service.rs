use std::fs;
use std::sync::Arc;
use std::thread;

use crate::testutil::{APP_VERSION, GARBAGE, Sandbox, read};
use crate::{LibraryAvailability, LibraryService, StoreError};

fn service(sandbox: &Sandbox) -> LibraryService {
    LibraryService::new(sandbox.roots.clone(), APP_VERSION)
}

fn expect_ready(availability: LibraryAvailability) -> Arc<crate::Library> {
    match availability {
        LibraryAvailability::Ready(library) => library,
        LibraryAvailability::Unavailable(error) => panic!("expected a ready library: {error}"),
    }
}

fn expect_unavailable(availability: LibraryAvailability) -> Arc<StoreError> {
    match availability {
        LibraryAvailability::Unavailable(error) => error,
        LibraryAvailability::Ready(_) => panic!("expected an unavailable library"),
    }
}

#[test]
fn concurrent_callers_share_one_open_library() {
    let sandbox = Sandbox::new();
    let service = Arc::new(service(&sandbox));

    let callers: Vec<_> = (0..4)
        .map(|_| {
            let service = Arc::clone(&service);
            thread::spawn(move || expect_ready(service.availability()))
        })
        .collect();
    let libraries: Vec<_> = callers
        .into_iter()
        .map(|caller| caller.join().expect("caller"))
        .collect();

    for library in &libraries {
        assert!(Arc::ptr_eq(library, &libraries[0]));
    }
}

#[test]
fn retry_opens_the_library_once_the_other_process_has_let_go() {
    let sandbox = Sandbox::new();
    let other_process = sandbox.open_latest();
    let service = service(&sandbox);

    let error = expect_unavailable(service.availability());
    assert!(
        matches!(*error, StoreError::LibraryLocked { .. }),
        "{error:?}"
    );
    // The failure is remembered rather than retried behind the caller's back.
    expect_unavailable(service.availability());

    drop(other_process);
    expect_ready(service.retry());
    expect_ready(service.availability());
}

#[test]
fn restore_is_refused_unless_a_snapshot_can_fix_the_failure() {
    let sandbox = Sandbox::new();
    sandbox.create_v1();
    let snapshot = sandbox.snapshot_closed_database(1);

    let ready = service(&sandbox);
    expect_ready(ready.availability());
    let result = ready.restore_snapshot(&snapshot.name);
    assert!(
        matches!(result, Err(StoreError::RestoreNotAvailable)),
        "{result:?}"
    );
    drop(ready);

    let other_process = sandbox.open_latest();
    let locked = service(&sandbox);
    expect_unavailable(locked.availability());
    let result = locked.restore_snapshot(&snapshot.name);
    assert!(
        matches!(result, Err(StoreError::RestoreNotAvailable)),
        "{result:?}"
    );
    drop(other_process);
    assert!(sandbox.set_aside_folders().is_empty());
}

#[test]
fn restoring_a_snapshot_makes_a_corrupt_library_available_again() {
    let sandbox = Sandbox::new();
    sandbox.create_v1();
    let snapshot = sandbox.snapshot_closed_database(1);
    fs::write(sandbox.db_path(), GARBAGE).expect("damage database");
    let service = service(&sandbox);
    let error = expect_unavailable(service.availability());
    assert!(
        matches!(*error, StoreError::DatabaseCorrupt { .. }),
        "{error:?}"
    );
    assert_eq!(service.snapshots().expect("list"), vec![snapshot.clone()]);

    let availability = service.restore_snapshot(&snapshot.name).expect("restore");

    expect_ready(availability);
    expect_ready(service.availability());
    let set_aside = sandbox.set_aside_folders();
    assert_eq!(set_aside.len(), 1);
    assert_eq!(read(&set_aside[0].join("skribe.sqlite")), GARBAGE);
}

#[test]
fn a_failed_restore_leaves_the_library_unavailable_for_its_original_reason() {
    let sandbox = Sandbox::new();
    sandbox.create_v1();
    fs::write(sandbox.db_path(), GARBAGE).expect("damage database");
    let service = service(&sandbox);
    expect_unavailable(service.availability());

    let result = service.restore_snapshot("skribe-v1-1.sqlite");

    assert!(
        matches!(result, Err(StoreError::SnapshotNotFound { .. })),
        "{result:?}"
    );
    let error = expect_unavailable(service.availability());
    assert!(
        matches!(*error, StoreError::DatabaseCorrupt { .. }),
        "{error:?}"
    );
    assert_eq!(read(&sandbox.db_path()), GARBAGE);
}
