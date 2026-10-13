use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use crate::{Library, SnapshotInfo, StorageRoots, StoreError};

/// The library as callers see it once an open attempt has finished.
#[derive(Debug, Clone)]
pub enum LibraryAvailability {
    Ready(Arc<Library>),
    /// The library could not be opened. Nothing was changed beyond what the error states.
    Unavailable(Arc<StoreError>),
}

#[derive(Debug)]
enum State {
    NotOpened,
    Busy,
    Settled(LibraryAvailability),
}

/// Owns the one library an app instance uses and serialises opening, retrying and restoring.
/// Every method blocks while the library is being opened; call them off the UI thread.
#[derive(Debug)]
pub struct LibraryService {
    roots: StorageRoots,
    app_version: String,
    state: Mutex<State>,
    settled: Condvar,
}

impl LibraryService {
    pub fn new(roots: StorageRoots, app_version: impl Into<String>) -> Self {
        Self {
            roots,
            app_version: app_version.into(),
            state: Mutex::new(State::NotOpened),
            settled: Condvar::new(),
        }
    }

    pub fn roots(&self) -> &StorageRoots {
        &self.roots
    }

    /// The library's availability, opening it first if no attempt has been made and waiting
    /// if one is under way.
    pub fn availability(&self) -> LibraryAvailability {
        let mut state = self.lock();
        loop {
            match &*state {
                State::Settled(availability) => return availability.clone(),
                State::Busy => state = self.wait(state),
                State::NotOpened => {
                    *state = State::Busy;
                    drop(state);
                    return self.settle(|| Library::open(&self.roots, &self.app_version));
                }
            }
        }
    }

    /// Tries to open an unavailable library again, for failures the user can clear outside the
    /// app. A library that is already open stays open.
    pub fn retry(&self) -> LibraryAvailability {
        {
            let mut state = self.lock();
            if matches!(&*state, State::Settled(LibraryAvailability::Unavailable(_))) {
                *state = State::NotOpened;
            }
        }
        self.availability()
    }

    /// Restores the named snapshot over an unavailable library's database and reopens it.
    /// Refused unless the current failure is one a snapshot can recover from. If the restore
    /// itself fails, the library stays unavailable for its original reason.
    pub fn restore_snapshot(&self, name: &str) -> Result<LibraryAvailability, StoreError> {
        let original = {
            let mut state = self.lock();
            while matches!(&*state, State::Busy) {
                state = self.wait(state);
            }
            let State::Settled(LibraryAvailability::Unavailable(error)) = &*state else {
                return Err(StoreError::RestoreNotAvailable);
            };
            if !error.offers_snapshot_restore() {
                return Err(StoreError::RestoreNotAvailable);
            }
            let original = Arc::clone(error);
            *state = State::Busy;
            original
        };

        let mut restore_error = None;
        let availability = self.settle(|| match Library::restore_snapshot(&self.roots, name) {
            Ok(_) => Library::open(&self.roots, &self.app_version),
            Err(error) => {
                restore_error = Some(error);
                Err(StoreError::RestoreNotAvailable)
            }
        });
        match restore_error {
            None => Ok(availability),
            Some(error) => {
                *self.lock() = State::Settled(LibraryAvailability::Unavailable(original));
                Err(error)
            }
        }
    }

    pub fn snapshots(&self) -> Result<Vec<SnapshotInfo>, StoreError> {
        Library::snapshots(&self.roots)
    }

    /// Runs `open` with the state marked busy, then publishes the outcome. If `open` panics
    /// the state returns to not-opened so waiting callers retry instead of blocking forever.
    fn settle(&self, open: impl FnOnce() -> Result<Library, StoreError>) -> LibraryAvailability {
        struct Reset<'a>(&'a LibraryService);
        impl Drop for Reset<'_> {
            fn drop(&mut self) {
                let mut state = self.0.lock();
                if matches!(&*state, State::Busy) {
                    *state = State::NotOpened;
                }
                self.0.settled.notify_all();
            }
        }

        let reset = Reset(self);
        let availability = match open() {
            Ok(library) => LibraryAvailability::Ready(Arc::new(library)),
            Err(error) => LibraryAvailability::Unavailable(Arc::new(error)),
        };
        *self.lock() = State::Settled(availability.clone());
        drop(reset);
        availability
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&self, guard: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.settled
            .wait(guard)
            .unwrap_or_else(PoisonError::into_inner)
    }
}
