//! One background thread that owns a piece of thread-bound state and runs jobs against it,
//! with a deadline on every job.
//!
//! UI Automation calls go to another process and block until it answers. A call that is
//! stuck cannot be cancelled, so the caller never makes one itself: it hands a job to the
//! worker thread and waits only as long as it is willing to. When the wait runs out the
//! worker is abandoned (it finishes or hangs on its own, then exits) and the next job gets a
//! fresh thread with fresh state.
//!
//! The state is created on the worker thread and never leaves it, so it does not have to be
//! `Send`. That is what lets it hold COM interfaces.

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

use crate::context::page::types::PageError;

type Job<S> = Box<dyn FnOnce(&Result<S, PageError>) + Send>;
type Init<S> = dyn Fn() -> Result<S, PageError> + Send + Sync;

/// Told to a running job when its caller has stopped waiting, so a long job can give up at
/// its next convenient point instead of working on for nobody.
#[derive(Debug, Clone, Default)]
pub(crate) struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

enum Reply<T> {
    Done(Result<T, PageError>),
    /// The thread could not create its state; it is of no use to later jobs either.
    InitFailed(PageError),
}

struct Lane<S> {
    id: u64,
    jobs: mpsc::Sender<Job<S>>,
}

pub(crate) struct Worker<S: 'static> {
    thread_name: &'static str,
    init: Arc<Init<S>>,
    lane: Mutex<Option<Lane<S>>>,
    lanes_started: AtomicU64,
}

impl<S: 'static> Worker<S> {
    /// `init` runs on each worker thread before its first job.
    pub fn new(
        thread_name: &'static str,
        init: impl Fn() -> Result<S, PageError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            thread_name,
            init: Arc::new(init),
            lane: Mutex::new(None),
            lanes_started: AtomicU64::new(0),
        }
    }

    /// Run `job` on the worker thread and wait at most `deadline` for its result.
    ///
    /// Jobs run one at a time; time spent queued behind another caller's job counts against
    /// the deadline.
    pub fn run<T, F>(&self, deadline: Duration, job: F) -> Result<T, PageError>
    where
        T: Send + 'static,
        F: FnOnce(&S, &Cancel) -> Result<T, PageError> + Send + 'static,
    {
        let cancel = Cancel::default();
        let (reply_tx, reply_rx) = mpsc::channel::<Reply<T>>();
        let job_cancel = cancel.clone();
        let mut boxed: Job<S> = Box::new(move |state| {
            let reply = match state {
                Ok(state) => Reply::Done(job(state, &job_cancel)),
                Err(error) => Reply::InitFailed(error.clone()),
            };
            // The caller may have stopped waiting; then nobody wants the result.
            let _ = reply_tx.send(reply);
        });

        // A lane whose thread has died (a job panicked) refuses the job and hands it back;
        // one fresh lane is then tried.
        let mut lane_id = 0;
        let mut sent = false;
        for _ in 0..2 {
            let (id, jobs) = self.lane()?;
            match jobs.send(boxed) {
                Ok(()) => {
                    lane_id = id;
                    sent = true;
                    break;
                }
                Err(mpsc::SendError(returned)) => {
                    self.abandon(id);
                    boxed = returned;
                }
            }
        }
        if !sent {
            return Err(unavailable("The window reader could not be started."));
        }

        match reply_rx.recv_timeout(deadline) {
            Ok(Reply::Done(result)) => result,
            Ok(Reply::InitFailed(error)) => {
                self.abandon(lane_id);
                Err(error)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                cancel.cancel();
                self.abandon(lane_id);
                Err(PageError::Timeout)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.abandon(lane_id);
                Err(unavailable("The window reader stopped unexpectedly."))
            }
        }
    }

    /// How many worker threads have been started so far.
    #[cfg(test)]
    pub fn lanes_started(&self) -> u64 {
        self.lanes_started.load(Ordering::Relaxed)
    }

    fn lane(&self) -> Result<(u64, mpsc::Sender<Job<S>>), PageError> {
        let mut lane = self
            .lane
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(lane) = lane.as_ref() {
            return Ok((lane.id, lane.jobs.clone()));
        }

        let id = self.lanes_started.fetch_add(1, Ordering::Relaxed) + 1;
        let (jobs, queue) = mpsc::channel::<Job<S>>();
        let init = Arc::clone(&self.init);
        thread::Builder::new()
            .name(self.thread_name.to_string())
            .spawn(move || {
                let state = init();
                // Ends when every sender is gone: the owner was dropped, or this lane was
                // abandoned and the callers still holding a sender have given up.
                for job in queue {
                    job(&state);
                }
            })
            .map_err(|_| unavailable("The window reader could not be started."))?;
        *lane = Some(Lane {
            id,
            jobs: jobs.clone(),
        });
        Ok((id, jobs))
    }

    /// Forget lane `id` so the next job starts a new thread. Another caller may already
    /// have replaced it; a newer lane is left alone.
    fn abandon(&self, id: u64) {
        let mut lane = self
            .lane
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if lane.as_ref().is_some_and(|lane| lane.id == id) {
            *lane = None;
        }
    }
}

fn unavailable(message: &str) -> PageError {
    PageError::Unavailable(message.to_string())
}

#[cfg(test)]
mod tests {
    use std::{sync::atomic::AtomicU32, thread::ThreadId, time::Instant};

    use super::*;

    /// State that cannot cross threads, like a COM interface.
    struct ThreadBound {
        home: ThreadId,
        _not_send: std::rc::Rc<()>,
    }

    fn worker() -> Worker<ThreadBound> {
        Worker::new("uia-test", || {
            Ok(ThreadBound {
                home: thread::current().id(),
                _not_send: std::rc::Rc::new(()),
            })
        })
    }

    const PLENTY: Duration = Duration::from_secs(10);

    #[test]
    fn a_worker_can_be_shared_between_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Worker<ThreadBound>>();
    }

    #[test]
    fn jobs_run_on_one_thread_that_is_not_the_callers() {
        let worker = worker();

        let first = worker
            .run(PLENTY, |state, _| Ok((state.home, thread::current().id())))
            .unwrap();
        let second = worker
            .run(PLENTY, |state, _| Ok((state.home, thread::current().id())))
            .unwrap();

        assert_eq!(first.0, first.1, "state is used on the thread that made it");
        assert_eq!(first, second);
        assert_ne!(first.1, thread::current().id());
        assert_eq!(worker.lanes_started(), 1);
    }

    #[test]
    fn a_jobs_error_is_returned_and_the_worker_is_kept() {
        let worker = worker();

        let failed: Result<(), _> = worker.run(PLENTY, |_, _| Err(PageError::NothingReadable));
        let after = worker.run(PLENTY, |_, _| Ok(7));

        assert_eq!(failed, Err(PageError::NothingReadable));
        assert_eq!(after, Ok(7));
        assert_eq!(worker.lanes_started(), 1);
    }

    #[test]
    fn a_slow_job_times_out_and_the_next_job_gets_a_fresh_worker() {
        let worker = worker();
        let (release, held) = mpsc::channel::<()>();
        let (finished_tx, finished) = mpsc::channel::<bool>();

        let started = Instant::now();
        let slow: Result<(), _> = worker.run(Duration::from_millis(80), move |_, cancel| {
            // Stands in for a COM call that does not come back.
            let _ = held.recv();
            let _ = finished_tx.send(cancel.is_cancelled());
            Ok(())
        });
        let waited = started.elapsed();

        assert_eq!(slow, Err(PageError::Timeout));
        assert!(
            waited >= Duration::from_millis(80),
            "returned early after {waited:?}"
        );
        assert!(waited < Duration::from_secs(2), "blocked for {waited:?}");

        // The first thread is still stuck; this must not queue behind it.
        let started = Instant::now();
        let stuck_thread = worker.run(PLENTY, |state, _| Ok(state.home));
        let fresh_thread = worker.run(PLENTY, |state, _| Ok(state.home));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(worker.lanes_started(), 2);
        assert_eq!(stuck_thread, fresh_thread, "the replacement is reused");

        // When the stuck job does come back it sees that nobody is waiting, and its late
        // result goes nowhere.
        release.send(()).unwrap();
        assert_eq!(finished.recv_timeout(PLENTY), Ok(true));
        assert_eq!(worker.run(PLENTY, |_, _| Ok(1)), Ok(1));
        assert_eq!(worker.lanes_started(), 2);
    }

    #[test]
    fn an_abandoned_thread_exits_once_its_job_returns() {
        struct Flag(Arc<AtomicBool>);
        impl Drop for Flag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let state_dropped = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&state_dropped);
        let worker = Worker::new("uia-test", move || Ok(Flag(Arc::clone(&flag))));
        let (release, held) = mpsc::channel::<()>();

        let slow: Result<(), _> = worker.run(Duration::from_millis(50), move |_, _| {
            let _ = held.recv();
            Ok(())
        });
        assert_eq!(slow, Err(PageError::Timeout));
        assert!(!state_dropped.load(Ordering::SeqCst));

        release.send(()).unwrap();
        let waiting_since = Instant::now();
        while !state_dropped.load(Ordering::SeqCst) {
            assert!(
                waiting_since.elapsed() < PLENTY,
                "the abandoned thread never exited"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_job_queued_behind_a_stuck_one_times_out_too() {
        let worker = Arc::new(worker());
        let (release, held) = mpsc::channel::<()>();
        let (entered_tx, entered) = mpsc::channel::<()>();
        let background = Arc::clone(&worker);
        let stuck = thread::spawn(move || {
            background.run(Duration::from_millis(400), move |_, _| {
                let _ = entered_tx.send(());
                let _ = held.recv();
                Ok(())
            })
        });
        entered.recv_timeout(PLENTY).unwrap();

        let queued = worker.run(Duration::from_millis(60), |_, _| Ok(1));

        assert_eq!(queued, Err(PageError::Timeout));
        assert_eq!(stuck.join().unwrap(), Err(PageError::Timeout));
        release.send(()).unwrap();
        assert_eq!(worker.run(PLENTY, |_, _| Ok(2)), Ok(2));
    }

    #[test]
    fn a_failed_start_is_reported_and_retried_on_the_next_call() {
        let attempts = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&attempts);
        let worker: Worker<u32> = Worker::new("uia-test", move || {
            let attempt = counter.fetch_add(1, Ordering::SeqCst) + 1;
            if attempt == 1 {
                Err(PageError::Unavailable("no automation".to_string()))
            } else {
                Ok(attempt)
            }
        });

        let first = worker.run(PLENTY, |state, _| Ok(*state));
        let second = worker.run(PLENTY, |state, _| Ok(*state));

        assert_eq!(
            first,
            Err(PageError::Unavailable("no automation".to_string()))
        );
        assert_eq!(second, Ok(2));
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_panicking_job_is_an_error_and_the_next_job_still_runs() {
        let worker = worker();

        let crashed: Result<(), _> = worker.run(PLENTY, |_, _| {
            panic!("job failed on purpose (expected in this test)")
        });
        let after = worker.run(PLENTY, |_, _| Ok(3));

        assert!(matches!(crashed, Err(PageError::Unavailable(_))));
        assert_eq!(after, Ok(3));
        assert_eq!(worker.lanes_started(), 2);
    }

    #[test]
    fn dropping_the_worker_stops_its_thread() {
        struct Flag(Arc<AtomicBool>);
        impl Drop for Flag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let state_dropped = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&state_dropped);
        let worker = Worker::new("uia-test", move || Ok(Flag(Arc::clone(&flag))));
        assert_eq!(worker.run(PLENTY, |_, _| Ok(())), Ok(()));

        drop(worker);

        let waiting_since = Instant::now();
        while !state_dropped.load(Ordering::SeqCst) {
            assert!(
                waiting_since.elapsed() < PLENTY,
                "the worker thread never exited"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}
