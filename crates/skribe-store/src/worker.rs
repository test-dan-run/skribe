use std::sync::mpsc;
use std::thread;

use rusqlite::Connection;

use crate::StoreError;

type Job = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

/// The open database, owned by one dedicated thread. Callers hand it short units of work and
/// block for the result, so no caller thread ever holds the connection and the UI thread never
/// touches SQLite.
#[derive(Debug)]
pub(crate) struct Database {
    jobs: Option<mpsc::Sender<Job>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Database {
    pub(crate) fn start(mut conn: Connection) -> Result<Self, StoreError> {
        let (jobs, queue) = mpsc::channel::<Job>();
        let worker = thread::Builder::new()
            .name("skribe-db".to_owned())
            .spawn(move || {
                for job in queue {
                    job(&mut conn);
                }
            })
            .map_err(|source| StoreError::Io {
                action: "start the database worker for",
                path: "skribe.sqlite".into(),
                source,
            })?;
        Ok(Self {
            jobs: Some(jobs),
            worker: Some(worker),
        })
    }

    /// Runs `work` on the database thread. Keep it to one short transaction.
    pub(crate) fn call<T, F>(&self, work: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StoreError> + Send + 'static,
    {
        let (reply, result) = mpsc::sync_channel(1);
        let job: Job = Box::new(move |conn| {
            let _ = reply.send(work(conn));
        });
        let jobs = self.jobs.as_ref().ok_or(StoreError::WorkerStopped)?;
        jobs.send(job).map_err(|_| StoreError::WorkerStopped)?;
        result.recv().map_err(|_| StoreError::WorkerStopped)?
    }
}

impl Drop for Database {
    /// Finishes queued work and closes the connection before the library lock is released.
    fn drop(&mut self) {
        self.jobs.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
