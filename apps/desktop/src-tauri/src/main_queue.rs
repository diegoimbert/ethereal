//! Plugin main-thread work queued for the process main thread, in order.
//!
//! Each job is queued here and a drain is posted to Tauri's event loop, so the main thread
//! runs the queue when it gets to it, as before. The queue also lets the main thread run the
//! jobs itself while it is blocked waiting for them: `NativeHost::shutdown` runs in the
//! `RunEvent::Exit` handler (on the main thread) and waits for the quit save, which reads
//! every plugin's state through main-thread calls (#566).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

type Job = Box<dyn FnOnce() + Send>;

#[derive(Clone)]
pub struct MainQueue {
    owner: ThreadId,
    jobs: Arc<Mutex<VecDeque<Job>>>,
}

impl MainQueue {
    /// A queue served by the thread `owner` (the process main thread).
    pub fn new(owner: ThreadId) -> Self {
        Self {
            owner,
            jobs: Arc::default(),
        }
    }

    /// Queue `job`; returns the drain to run on the owner thread.
    pub fn push(&self, job: Job) -> impl FnOnce() + Send + 'static {
        lock(&self.jobs).push_back(job);
        let jobs = self.jobs.clone();
        move || drain(&jobs)
    }

    /// Run the queued jobs if the caller is the owner thread.
    pub fn run_pending(&self) {
        if std::thread::current().id() == self.owner {
            drain(&self.jobs);
        }
    }
}

fn lock(jobs: &Mutex<VecDeque<Job>>) -> std::sync::MutexGuard<'_, VecDeque<Job>> {
    jobs.lock().unwrap_or_else(|e| e.into_inner())
}

/// Run the jobs one at a time, the lock released while each runs (a job may queue more, or
/// pump a nested run loop that drains too).
fn drain(jobs: &Mutex<VecDeque<Job>>) {
    loop {
        let job = lock(jobs).pop_front();
        match job {
            Some(job) => job(),
            None => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log_job(log: &Arc<Mutex<Vec<u32>>>, n: u32) -> Job {
        let log = log.clone();
        Box::new(move || log.lock().unwrap().push(n))
    }

    #[test]
    fn jobs_run_in_order_once_whether_drained_or_served() {
        let q = MainQueue::new(std::thread::current().id());
        let log = Arc::new(Mutex::new(Vec::new()));
        let d1 = q.push(log_job(&log, 1));
        let d2 = q.push(log_job(&log, 2));
        // The owner serves them while it waits (quitting)...
        q.run_pending();
        assert_eq!(*log.lock().unwrap(), vec![1, 2]);
        // ...and the drains posted to the event loop find nothing left.
        d1();
        d2();
        let d3 = q.push(log_job(&log, 3));
        d3();
        assert_eq!(*log.lock().unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn other_threads_never_run_the_jobs() {
        let q = MainQueue::new(std::thread::current().id());
        let log = Arc::new(Mutex::new(Vec::new()));
        let _drain = q.push(log_job(&log, 1));
        let other = q.clone();
        std::thread::spawn(move || other.run_pending())
            .join()
            .unwrap();
        assert!(log.lock().unwrap().is_empty());
        q.run_pending();
        assert_eq!(*log.lock().unwrap(), vec![1]);
    }
}
