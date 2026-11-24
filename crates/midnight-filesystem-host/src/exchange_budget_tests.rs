//! Hostile protocol-4 workers over a real pipe, supervised like a real worker: every
//! exchange ends within its budgets, and cancelling a listing or an unlock ends it.
use super::*;
use close::{Accounting, CloseReceipt, Supervisor, SupervisorProcess};
use midnight_plugin_host::{ContentSource, Error};
use std::{
    io::{PipeWriter, Write},
    sync::mpsc,
    thread::JoinHandle,
};

const SOURCE: u64 = 64 * 1024;

#[derive(Debug)]
struct Source {
    reads: Arc<AtomicUsize>,
}
impl ContentSource for Source {
    fn length(&self) -> u64 {
        SOURCE
    }
    fn display_name(&self) -> String {
        "hostile worker fixture".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, _: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(vec![0; length as usize])
    }
}

/// What the fake worker writes until it is killed. It never reads its input.
#[derive(Clone, Copy)]
enum Script {
    /// A `SourceRead` of `length` bytes every `pause` for request 1, never an answer.
    Reads { length: u32, pause: Duration },
    /// After `pause`, one empty final page for each of the requests 1 to `count`.
    LatePages { pause: Duration, count: u64 },
}

fn run(script: Script, mut pipe: PipeWriter, stop: &mpsc::Receiver<()>) -> io::Result<()> {
    // Killed: the sender is gone (or signalled).
    let killed = |pause: Duration| {
        !matches!(
            stop.recv_timeout(pause),
            Err(mpsc::RecvTimeoutError::Timeout)
        )
    };
    let mut send = |message: &Message| {
        api::write_message_with_protocol(&mut pipe, Protocol::V4, message)
            .and_then(|()| pipe.flush())
    };
    match script {
        Script::Reads { length, pause } => {
            while !killed(pause) {
                send(&Message::SourceRead {
                    request: 1,
                    offset: 0,
                    length,
                })?;
            }
        }
        Script::LatePages { pause, count } => {
            if killed(pause) {
                return Ok(());
            }
            for request in 1..=count {
                send(&Message::Entries {
                    request,
                    entries: Vec::new(),
                    more: false,
                })?;
            }
            let _ = stop.recv();
        }
    }
    Ok(())
}

/// The worker process: killing it ends its script, which closes its end of the pipe.
struct Worker {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    fn start(script: Script, pipe: PipeWriter) -> Self {
        let (stop, stopped) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            // A write fails once the host closed the pipe; the worker then ends.
            let _ = run(script, pipe, &stopped);
        });
        Self {
            stop: Some(stop),
            thread: Some(thread),
        }
    }
}
impl SupervisorProcess for Worker {
    fn kill(&mut self) -> io::Result<()> {
        drop(self.stop.take());
        Ok(())
    }
    fn wait_terminated(&mut self) -> io::Result<()> {
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| io::Error::other("fake worker panicked"))?;
        }
        Ok(())
    }
}

fn budget() -> Budget {
    Budget {
        served_per_byte: 4,
        served_extra: 1024 * 1024,
        time_base: Duration::from_secs(60),
        time_rate: 1 << 40,
        cancel_grace: Duration::from_secs(60),
    }
}

struct Fixture {
    mount: Option<Arc<Mount>>,
    reads: Arc<AtomicUsize>,
    supervisor: Option<JoinHandle<()>>,
    _temp: tempfile::TempDir,
}
impl Fixture {
    fn new(script: Script, budget: Budget) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let reads = Arc::new(AtomicUsize::new(0));
        let source = CapturedFile::from_source(Arc::new(Source {
            reads: reads.clone(),
        }))
        .unwrap();
        let (output, pipe) = std::io::pipe().unwrap();
        #[cfg(windows)]
        let output = File::from(std::os::windows::io::OwnedHandle::from(output));
        #[cfg(unix)]
        let output = File::from(std::os::fd::OwnedFd::from(output));
        let staging = Arc::new(Mutex::new(Some(
            crate::staging::Staging::open(&temp.path().join("spool")).unwrap(),
        )));
        let supervision = Arc::new(Supervision::with_budget(None, budget));
        let receipt = Arc::new(CloseReceipt::new());
        let owner = Supervisor::new(
            Worker::start(script, pipe),
            staging.clone(),
            Accounting::for_test(Arc::new(AtomicUsize::new(0))),
        );
        let supervised = supervision.clone();
        let supervisor_receipt = receipt.clone();
        let supervisor =
            std::thread::spawn(move || close::supervise(owner, supervised, supervisor_receipt));
        let mount = Arc::new(Mount {
            id: 31,
            provider: "hostile.fixture".into(),
            provider_version: "1".into(),
            stable_entry_ids: true,
            format: Format::Zip,
            container: temp.path().join("hostile.zip"),
            source: Some(source),
            channel: Mutex::new(Channel {
                input: tempfile::tempfile().unwrap(),
                output,
                request: 0,
            }),
            supervision,
            protocol: Protocol::V4,
            staging,
            close: receipt,
            nested: None,
        });
        Self {
            mount: Some(mount),
            reads,
            supervisor: Some(supervisor),
            _temp: temp,
        }
    }
    fn mount(&self) -> Arc<Mount> {
        self.mount.clone().unwrap()
    }
    /// Run `work` on its own thread, as a backend worker does.
    fn spawn<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Mount) -> T + Send + 'static,
    ) -> mpsc::Receiver<(T, Instant)> {
        let mount = self.mount();
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            let result = work(&mount);
            let _ = send.send((result, Instant::now()));
        });
        receive
    }
    fn wait_for(&self, what: &str, condition: impl Fn() -> bool) {
        let start = Instant::now();
        while !condition() {
            assert!(start.elapsed() < Duration::from_secs(10), "{what}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn wait_for_source_reads(&self) {
        self.wait_for("the worker read no source", || {
            self.reads.load(Ordering::SeqCst) > 0
        });
    }
    fn stopped(&self) -> bool {
        self.mount.as_ref().unwrap().supervision.stopped()
    }
    /// A later request of the mount: it reports why the worker was stopped.
    fn later_error(&self) -> FsError {
        let error = self
            .mount()
            .read(2, 0, 4, &Cancellation::default())
            .unwrap_err();
        assert_eq!(error.kind, STOPPED, "{error}");
        assert!(error.message.contains("open the archive again"), "{error}");
        error
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mount) = self.mount.take() {
            mount.supervision.request_stop(Stop::Closed);
            // Closes the host's end of the pipe, so a worker blocked writing ends too.
            drop(mount);
        }
        if let Some(supervisor) = self.supervisor.take() {
            let _ = supervisor.join();
        }
    }
}

/// The result of `work` started at `start`, which must arrive well within `limit`.
fn within<T>(
    receive: &mpsc::Receiver<(T, Instant)>,
    start: Instant,
    limit: Duration,
) -> (T, Duration) {
    let (result, end) = receive
        .recv_timeout(limit + Duration::from_secs(10))
        .expect("the request did not return");
    let elapsed = end - start;
    assert!(elapsed < limit + Duration::from_secs(5), "{elapsed:?}");
    (result, elapsed)
}

#[test]
fn endless_source_reads_fail_at_the_byte_budget_and_retire_the_worker() {
    let fixture = Fixture::new(
        Script::Reads {
            length: 16 * 1024,
            pause: Duration::ZERO,
        },
        budget(),
    );
    let start = Instant::now();
    let listed = fixture.spawn(|mount| mount.list(1, &Cancellation::default()));
    let (result, _) = within(&listed, start, Duration::ZERO);
    let error = result.unwrap_err();
    assert!(
        error
            .message
            .contains("read more than 1 MiB of its archive for one request"),
        "{error}"
    );
    // 4 × 64 KiB + 1 MiB served, then the read that exceeds it is refused.
    assert_eq!(fixture.reads.load(Ordering::SeqCst), 80);
    assert!(fixture.stopped());
    let later = fixture.later_error();
    assert!(later.message.contains("read more than 1 MiB"), "{later}");
}

#[test]
fn a_slow_drip_of_source_reads_ends_at_the_time_budget_and_frees_waiting_requests() {
    let limit = Duration::from_secs(1);
    let fixture = Fixture::new(
        Script::Reads {
            length: 1,
            pause: Duration::from_millis(20),
        },
        Budget {
            time_base: limit,
            ..budget()
        },
    );
    let start = Instant::now();
    let listed = fixture.spawn(|mount| mount.list(1, &Cancellation::default()));
    fixture.wait_for_source_reads();
    // Another owner's request waits for the channel that the listing holds.
    let read = fixture.spawn(|mount| mount.read(2, 0, 4, &Cancellation::default()));
    let (listing, elapsed) = within(&listed, start, limit);
    assert!(elapsed >= limit, "{elapsed:?}");
    let error = listing.unwrap_err();
    assert_eq!(error.kind, STOPPED, "{error}");
    assert!(
        error.message.contains("within its time budget of 1 second"),
        "{error}"
    );
    let (read, _) = within(&read, start, limit);
    let error = read.unwrap_err();
    assert_eq!(error.kind, STOPPED, "{error}");
    assert!(error.message.contains("time budget"), "{error}");
    // Each source read renewed the idle deadline; only the budget ended the exchange.
    assert!(fixture.reads.load(Ordering::SeqCst) > 10);
    fixture.later_error();
}

#[test]
fn cancelling_a_listing_stops_a_worker_that_never_answers() {
    let grace = Duration::from_millis(300);
    let fixture = Fixture::new(
        Script::Reads {
            length: 1,
            pause: Duration::from_millis(20),
        },
        Budget {
            cancel_grace: grace,
            ..budget()
        },
    );
    let cancel = Cancellation::default();
    let token = cancel.clone();
    let listed = fixture.spawn(move |mount| mount.list(1, &token));
    fixture.wait_for_source_reads();
    let start = Instant::now();
    cancel.cancel();
    let (listing, elapsed) = within(&listed, start, grace);
    assert!(elapsed >= grace, "{elapsed:?}");
    assert_eq!(listing.unwrap_err().kind, io::ErrorKind::Interrupted);
    assert!(fixture.stopped());
    let later = fixture.later_error();
    assert!(
        later.message.contains("did not end a cancelled request"),
        "{later}"
    );
}

#[test]
fn a_cancelled_listing_answered_within_the_grace_keeps_the_worker() {
    let fixture = Fixture::new(
        Script::LatePages {
            pause: Duration::from_millis(300),
            count: 2,
        },
        budget(),
    );
    let cancel = Cancellation::default();
    let token = cancel.clone();
    let listed = fixture.spawn(move |mount| mount.list(1, &token));
    let mount = fixture.mount();
    fixture.wait_for("the listing exchange did not begin", || {
        mount.supervision.lock().is_some()
    });
    cancel.cancel();
    let (listing, _) = within(&listed, Instant::now(), Duration::from_secs(1));
    assert_eq!(listing.unwrap_err().kind, io::ErrorKind::Interrupted);
    assert!(
        !fixture.stopped(),
        "a page within the grace ended the worker"
    );
    assert!(mount.list(1, &Cancellation::default()).unwrap().is_empty());
}

#[test]
fn cancelling_an_unlock_stops_a_worker_that_never_answers() {
    let grace = Duration::from_millis(300);
    let fixture = Fixture::new(
        Script::Reads {
            length: 1,
            pause: Duration::from_millis(20),
        },
        Budget {
            cancel_grace: grace,
            ..budget()
        },
    );
    let cancel = Cancellation::default();
    let token = cancel.clone();
    let unlocked = fixture.spawn(move |mount| {
        mount.unlock(&midnight_core::ArchivePassword::new("heslo".into()), &token)
    });
    fixture.wait_for_source_reads();
    let start = Instant::now();
    cancel.cancel();
    let (unlock, _) = within(&unlocked, start, grace);
    assert_eq!(unlock.unwrap_err().kind, io::ErrorKind::Interrupted);
    assert!(fixture.stopped());
    assert!(!fixture.mount().unlocked());
    fixture.later_error();
}
