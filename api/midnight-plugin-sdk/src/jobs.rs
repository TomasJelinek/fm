use crate::guard::{contain, lock};
use midnight_plugin_api::abi;
use std::{
    collections::HashMap,
    ffi::c_void,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(crate) type Task = Box<dyn FnOnce() + Send + 'static>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JobError {
    Full,
    Exhausted,
}

pub(crate) struct Group {
    count: AtomicUsize,
    limit: usize,
}

struct Permit(Arc<Group>);

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.count.fetch_sub(1, Ordering::Release);
    }
}

struct Job {
    task: Mutex<Option<Task>>,
    // Drop captures before making this instance or library appear quiescent.
    _local: Permit,
    _library: Permit,
}

#[derive(Debug)]
pub(crate) struct Registration {
    token: usize,
    accepted: bool,
}

static JOBS: OnceLock<Mutex<HashMap<usize, Arc<Job>>>> = OnceLock::new();
static IDS: AtomicUsize = AtomicUsize::new(1);
static LIBRARY: OnceLock<Arc<Group>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<usize, Arc<Job>>> {
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

impl Group {
    pub(crate) fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            count: AtomicUsize::new(0),
            limit,
        })
    }

    pub(crate) fn active(&self) -> usize {
        self.count.load(Ordering::Acquire)
    }

    fn reserve(self: &Arc<Self>) -> Result<Permit, JobError> {
        self.count
            .try_update(Ordering::Acquire, Ordering::Relaxed, |count| {
                (count < self.limit).then(|| count + 1)
            })
            .map_err(|_| JobError::Full)?;
        Ok(Permit(self.clone()))
    }

    pub(crate) fn register(self: &Arc<Self>, task: Task) -> Result<Registration, (JobError, Task)> {
        let library = match LIBRARY.get_or_init(|| Self::new(4096)).reserve() {
            Ok(permit) => permit,
            Err(error) => return Err((error, task)),
        };
        let local = match self.reserve() {
            Ok(permit) => permit,
            Err(error) => return Err((error, task)),
        };
        let token = match next_id(&IDS) {
            Ok(token) => token,
            Err(error) => return Err((error, task)),
        };
        let job = Arc::new(Job {
            task: Mutex::new(Some(task)),
            _local: local,
            _library: library,
        });
        lock(registry()).insert(token, job);
        Ok(Registration {
            token,
            accepted: false,
        })
    }
}

fn next_id(ids: &AtomicUsize) -> Result<usize, JobError> {
    ids.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
        (id != 0).then(|| id.checked_add(1)).flatten()
    })
    .map_err(|_| JobError::Exhausted)
}

impl Registration {
    #[cfg(test)]
    fn token(&self) -> usize {
        self.token
    }

    pub(crate) fn work(&self, request_id: u64, class: u32) -> abi::Work {
        abi::Work {
            // Opaque integer cookie only; neither callback dereferences it.
            context: self.token as *mut c_void,
            run: Some(run_job),
            drop: Some(drop_job),
            request_id,
            class,
            affinity: 0,
        }
    }

    pub(crate) fn accept(mut self) {
        self.accepted = true;
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if !self.accepted {
            drop_job(self.token as *mut c_void);
        }
    }
}

extern "C" fn run_job(context: *mut c_void) {
    let _ = contain(|| {
        let job = lock(registry()).get(&(context as usize)).cloned();
        if let Some(job) = job {
            let task = lock(&job.task).take();
            if let Some(task) = task {
                task();
            }
        }
    });
}

extern "C" fn drop_job(context: *mut c_void) {
    let _ = contain(|| {
        let job = lock(registry()).remove(&(context as usize));
        drop(job); // Never run a captured destructor under the registry lock.
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Barrier, mpsc};

    struct CountDrop(Arc<AtomicUsize>);

    impl Drop for CountDrop {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn accepted_work_runs_and_drops_once_and_rejection_reclaims_captures() {
        let group = Group::new(2);
        let runs = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let captured = CountDrop(drops.clone());
        let count = runs.clone();
        let pending = group
            .register(Box::new(move || {
                count.fetch_add(1, Ordering::SeqCst);
                drop(captured);
            }))
            .ok()
            .unwrap();
        let token = pending.token();
        pending.accept();
        run_job(token as *mut c_void);
        run_job(token as *mut c_void);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(group.active(), 1);
        drop_job(token as *mut c_void);
        drop_job(token as *mut c_void);
        assert_eq!(group.active(), 0);

        let captured = CountDrop(drops.clone());
        let rejected = group
            .register(Box::new(move || drop(captured)))
            .ok()
            .unwrap();
        let old = rejected.token();
        drop(rejected);
        run_job(old as *mut c_void);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert_eq!(group.active(), 0);
    }

    #[test]
    fn cancellation_without_run_releases_capacity_but_full_registration_keeps_task() {
        let group = Group::new(1);
        let drops = Arc::new(AtomicUsize::new(0));
        let captured = CountDrop(drops.clone());
        let pending = group
            .register(Box::new(move || drop(captured)))
            .ok()
            .unwrap();
        let token = pending.token();
        pending.accept();
        let captured = CountDrop(drops.clone());
        let (error, task) = group
            .register(Box::new(move || drop(captured)))
            .err()
            .unwrap();
        assert_eq!(error, JobError::Full);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(task);
        drop_job(token as *mut c_void);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert_eq!(group.active(), 0);
    }

    #[test]
    fn drop_while_running_waits_for_callback_resources_before_quiescence() {
        let group = Group::new(1);
        let ready = Arc::new(Barrier::new(2));
        let worker_ready = ready.clone();
        let (send, receive) = mpsc::channel();
        let drops = Arc::new(AtomicUsize::new(0));
        let captured = CountDrop(drops.clone());
        let pending = group
            .register(Box::new(move || {
                worker_ready.wait();
                receive.recv().unwrap();
                drop(captured);
            }))
            .ok()
            .unwrap();
        let token = pending.token();
        pending.accept();
        let thread = std::thread::spawn(move || run_job(token as *mut c_void));
        ready.wait();
        drop_job(token as *mut c_void);
        assert_eq!(group.active(), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        send.send(()).unwrap();
        thread.join().unwrap();
        assert_eq!(group.active(), 0);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn callback_panics_and_panicking_payload_destructors_do_not_cross_c_boundary() {
        struct BadPayload;
        impl Drop for BadPayload {
            fn drop(&mut self) {
                panic!("payload destructor must not run");
            }
        }
        let group = Group::new(1);
        let pending = group
            .register(Box::new(|| std::panic::panic_any(BadPayload)))
            .ok()
            .unwrap();
        let token = pending.token();
        pending.accept();
        run_job(token as *mut c_void);
        drop_job(token as *mut c_void);
        assert_eq!(group.active(), 0);
        let pending = group.register(Box::new(|| {})).ok().unwrap();
        drop(pending);
        assert_eq!(group.active(), 0);
    }

    #[test]
    fn context_ids_fail_on_exhaustion_and_never_wrap_to_an_old_job() {
        let ids = AtomicUsize::new(usize::MAX - 1);
        assert_eq!(next_id(&ids), Ok(usize::MAX - 1));
        assert_eq!(next_id(&ids), Err(JobError::Exhausted));
        assert_eq!(next_id(&ids), Err(JobError::Exhausted));
    }
}
