//! Ready-gated playback process adapter and bounded captured reads.

use crate::{
    video_installation::{LaunchPins, VideoInstallation, VideoWorker},
    video_transport::{RestrictedVideoPipe, VideoIo, VideoSource},
};
use midnight_plugin_host::CapturedFile;
use midnight_worker_platform::{
    ExitStatus, Process, ResourcePolicy, UnloadableLibrary, Worker, WorkerStopped,
};
use std::{
    fmt,
    fs::File,
    io::{self, BufReader, Read, Write},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// How long a launch that found its helper gone waits for the helper's exit status.
const EXIT_REPORT: Duration = Duration::from_secs(2);

/// The playback helper closed its pipe or ended, so it cannot serve the session. Its exit
/// status, once the host has confirmed the helper's termination, says why (a crash, a
/// library it could not load), and for a library the host names which one and whether it
/// is missing or denied to the helper's sandbox. The text is written for the viewer's user.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HelperStopped {
    pub status: Option<ExitStatus>,
    pub library: Option<UnloadableLibrary>,
}

impl HelperStopped {
    fn ended(status: ExitStatus) -> Self {
        Self {
            status: Some(status),
            library: None,
        }
    }
}

impl fmt::Display for HelperStopped {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.library, self.status) {
            (Some(library), _) => write!(formatter, "The video helper {library}"),
            (None, Some(status)) => {
                write!(formatter, "The video helper stopped: {}", status.reason())
            }
            (None, None) => formatter.write_str("The video helper stopped unexpectedly"),
        }
    }
}

impl std::error::Error for HelperStopped {}

/// The end of the helper's output, or a write to a pipe it no longer reads: only a helper
/// that ended closes its pipe. Other errors, such as a refused message, stay as they are.
fn helper_pipe_failure(error: io::Error) -> io::Error {
    match error.kind() {
        io::ErrorKind::UnexpectedEof | io::ErrorKind::BrokenPipe => {
            io::Error::new(error.kind(), HelperStopped::default())
        }
        _ => error,
    }
}

fn stopped_helper(error: &io::Error) -> Option<HelperStopped> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<HelperStopped>())
        .cloned()
}

/// How a helper that ended before it was ready failed: for a library its loader could not
/// load, which of the installation's `libraries` and why (missing, or denied to the helper).
fn diagnose(libraries: &[PathBuf], status: ExitStatus) -> HelperStopped {
    HelperStopped {
        status: Some(status),
        library: status
            .library_not_loaded()
            .then(|| midnight_worker_platform::unloadable_library(libraries))
            .flatten(),
    }
}

/// A launch whose startup message found the helper gone reports why it ended.
fn launch_failure(
    error: io::Error,
    diagnose: impl FnOnce(ExitStatus) -> HelperStopped,
) -> io::Error {
    match error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<WorkerStopped>())
    {
        Some(&WorkerStopped(status)) => io::Error::new(error.kind(), diagnose(status)),
        None => error,
    }
}

/// Owns the MMV2 pipe ends. A separate supervisor owns native termination and
/// the resident slot. Blocking pipe calls run on the viewer backend worker.
pub struct VideoProcess {
    input: Option<File>,
    output: Option<BufReader<File>>,
    supervisor: Arc<Supervisor>,
}

impl VideoProcess {
    /// Spawn only the pinned installed worker. Keep a clone of the captured
    /// source together with the resident permit in 'owner' until confirmed reap.
    /// Use the same cancellation allocation for all operations on this pipe.
    pub fn launch(
        installation: &VideoInstallation,
        which: VideoWorker,
        policy: ResourcePolicy,
        cancelled: Arc<AtomicBool>,
        ready_deadline: Instant,
        owner: impl Send + 'static,
    ) -> io::Result<Self> {
        let control = VideoIo {
            deadline: ready_deadline,
            cancelled: &cancelled,
        };
        control.check()?;
        let helper = installation.worker(which)?;
        let libraries = installation.libraries(which);
        let pins = installation.capture()?;
        if let Err(error) = control.check() {
            pins.release()?;
            return Err(error);
        }
        let Worker {
            input,
            output,
            process,
        } = match midnight_worker_platform::spawn_with_policy(&helper, policy) {
            Ok(worker) => worker,
            Err(error) => {
                pins.release()?;
                return Err(launch_failure(error, |status| diagnose(&libraries, status)));
            }
        };
        let pins = Arc::new(Mutex::new(Some(pins)));
        let mut ownership = ProcessOwner {
            process: Some(process),
            pins: pins.clone(),
            resident: Some(owner),
        };
        let supervisor = Arc::new(Supervisor::new(cancelled.clone()));
        let reaper = supervisor.clone();
        thread::Builder::new()
            .name("video-worker-supervisor".into())
            .spawn(move || {
                reaper.wait_for_retirement();
                reaper.complete_retirement(ownership.reap());
            })?;
        let mut pipe = Self {
            input: Some(input),
            output: Some(BufReader::new(output)),
            supervisor,
        };
        let ready = pipe.call(control, |pipe| {
            midnight_worker_sdk::read_policy_ready(pipe.output()?).map_err(helper_pipe_failure)?;
            let guard = pins.lock().unwrap_or_else(|error| error.into_inner());
            guard
                .as_ref()
                .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?
                .revalidate()
        });
        match ready {
            Ok(()) => Ok(pipe),
            Err(error) if stopped_helper(&error).is_some() => {
                // A helper that could not start (a library it needs is missing or denied to
                // it) ended before it was ready. Retire it and report why it ended.
                let receipt = pipe.retirement();
                drop(pipe);
                let _ = receipt.wait(Instant::now() + EXIT_REPORT);
                Err(match receipt.exit_status() {
                    Some(status) => io::Error::new(error.kind(), diagnose(&libraries, status)),
                    None => error,
                })
            }
            Err(error) => Err(error),
        }
    }

    /// Obtain the proof handle before transferring this pipe to VideoTransport.
    pub fn retirement(&self) -> VideoRetirement {
        VideoRetirement(self.supervisor.clone())
    }

    fn input(&mut self) -> io::Result<&mut File> {
        self.input
            .as_mut()
            .ok_or_else(|| io::ErrorKind::BrokenPipe.into())
    }

    fn output(&mut self) -> io::Result<&mut BufReader<File>> {
        self.output
            .as_mut()
            .ok_or_else(|| io::ErrorKind::BrokenPipe.into())
    }

    fn call<T>(
        &mut self,
        control: VideoIo<'_>,
        operation: impl FnOnce(&mut Self) -> io::Result<T>,
    ) -> io::Result<T> {
        let armed = self.supervisor.arm(control)?;
        let result = operation(self);
        drop(armed);
        control.check()?;
        result
    }
}

impl RestrictedVideoPipe for VideoProcess {
    fn read_exact(&mut self, bytes: &mut [u8], control: VideoIo<'_>) -> io::Result<()> {
        self.call(control, |pipe| {
            pipe.output()?
                .read_exact(bytes)
                .map_err(helper_pipe_failure)
        })
    }

    fn send(&mut self, parts: &[&[u8]], control: VideoIo<'_>) -> io::Result<()> {
        self.call(control, |pipe| {
            let input = pipe.input()?;
            let written = parts.iter().try_for_each(|part| input.write_all(part));
            written
                .and_then(|()| input.flush())
                .map_err(helper_pipe_failure)
        })
    }

    fn retire(&mut self) {
        self.supervisor.request_retirement();
        self.input = None;
        self.output = None;
    }
}

impl Drop for VideoProcess {
    fn drop(&mut self) {
        self.retire();
    }
}

/// A receipt for process exit and explicit pin/slot release. The caller closes
/// or drops its pipe before waiting. Session cancellation does not cancel proof.
pub struct VideoRetirement(Arc<Supervisor>);

impl VideoRetirement {
    /// How the helper ended, once the supervisor has confirmed its termination.
    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.0.progress().exit
    }

    /// After a confirmed retirement, an error that says the helper stopped also says how it
    /// ended (its exit status); before that, or for any other error, the error is unchanged.
    pub fn explain(&self, error: io::Error) -> io::Error {
        match (stopped_helper(&error), self.exit_status()) {
            (Some(HelperStopped { status: None, .. }), Some(status)) => {
                io::Error::new(error.kind(), HelperStopped::ended(status))
            }
            _ => error,
        }
    }

    pub fn wait(&self, deadline: Instant) -> io::Result<()> {
        let mut progress = self.0.progress();
        loop {
            if let Some(result) = &progress.retirement {
                return result
                    .as_ref()
                    .copied()
                    .map_err(|error| io::Error::new(error.kind(), error.to_string()));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Video worker retirement is still pending",
                ));
            }
            (progress, _) = self
                .0
                .changed
                .wait_timeout(progress, remaining)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

#[derive(Default)]
struct Progress {
    deadline: Option<Instant>,
    retiring: bool,
    retirement: Option<io::Result<()>>,
    /// How the helper ended, once its termination is confirmed.
    exit: Option<ExitStatus>,
}

struct Supervisor {
    cancelled: Arc<AtomicBool>,
    progress: Mutex<Progress>,
    changed: Condvar,
}

impl Supervisor {
    fn new(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            cancelled,
            progress: Mutex::new(Progress::default()),
            changed: Condvar::new(),
        }
    }

    fn progress(&self) -> MutexGuard<'_, Progress> {
        self.progress
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn arm(self: &Arc<Self>, control: VideoIo<'_>) -> io::Result<Armed> {
        if !std::ptr::eq(self.cancelled.as_ref(), control.cancelled) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Media pipe call used a different session cancellation flag",
            ));
        }
        control.check()?;
        let mut progress = self.progress();
        if progress.retiring {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        if progress.deadline.is_some() {
            return Err(io::Error::other("Overlapping calls to the media pipe"));
        }
        progress.deadline = Some(control.deadline);
        self.changed.notify_all();
        Ok(Armed(self.clone()))
    }

    fn request_retirement(&self) {
        self.progress().retiring = true;
        self.changed.notify_all();
    }

    fn complete_retirement(&self, result: io::Result<Option<ExitStatus>>) {
        let mut progress = self.progress();
        progress.exit = result.as_ref().ok().copied().flatten();
        progress.retirement = Some(result.map(|_| ()));
        drop(progress);
        self.changed.notify_all();
    }

    fn wait_for_retirement(&self) {
        let mut progress = self.progress();
        loop {
            let now = Instant::now();
            if progress.retiring
                || self.cancelled.load(Ordering::Acquire)
                || progress.deadline.is_some_and(|deadline| now >= deadline)
            {
                progress.retiring = true;
                return;
            }
            let interval = progress
                .deadline
                .map_or(Duration::from_millis(100), |deadline| {
                    deadline
                        .saturating_duration_since(now)
                        .min(Duration::from_millis(10))
                });
            (progress, _) = self
                .changed
                .wait_timeout(progress, interval)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

struct Armed(Arc<Supervisor>);

impl Drop for Armed {
    fn drop(&mut self) {
        self.0.progress().deadline = None;
        self.0.changed.notify_all();
    }
}

/// The owned process, exact pins and slot travel together even if starting
/// their supervising thread fails. Release only after native termination proof.
struct ProcessOwner<R> {
    process: Option<Process>,
    pins: Arc<Mutex<Option<LaunchPins>>>,
    resident: Option<R>,
}

impl<R> ProcessOwner<R> {
    /// Returns how the helper ended, when that is known: a confirmed termination has an
    /// exit status, and the kill does not replace the status of a helper that had already
    /// ended (one whose loader could not open a library, say).
    fn reap(&mut self) -> io::Result<Option<ExitStatus>> {
        let Some(process) = self.process.as_mut() else {
            return Ok(None);
        };
        let _ = process.kill();
        let termination = process.wait_terminated();
        let status = termination
            .is_ok()
            .then(|| process.wait_status().ok())
            .flatten();
        let pins = self
            .pins
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Err(error) = termination {
            // Preserve unconfirmed native ownership and its charged capacity.
            std::mem::forget((self.process.take(), pins, self.resident.take()));
            return Err(io::Error::new(
                error.kind(),
                format!("Video worker termination is unconfirmed. {error}"),
            ));
        }
        drop(self.process.take());
        if let Some(pins) = pins
            && let Err(error) = pins.release()
        {
            std::mem::forget(self.resident.take());
            return Err(error);
        }
        // The receipt can wake the next opener only after the capacity returns.
        drop(self.resident.take());
        Ok(status)
    }
}

impl<R> Drop for ProcessOwner<R> {
    fn drop(&mut self) {
        let _ = self.reap();
    }
}

/// A revision-checked file or provider capture retained on the backend. Keep
/// a clone in the native process owner so the capture outlives pipe retirement.
#[derive(Clone)]
pub struct CapturedVideoSource {
    capture: Arc<RetainedCapture>,
    confirmed_local_ssd: bool,
}

impl CapturedVideoSource {
    pub fn new(file: CapturedFile, confirmed_local_ssd: bool) -> Self {
        Self {
            capture: Arc::new(RetainedCapture(Some(file))),
            confirmed_local_ssd,
        }
    }

    fn file(&self) -> &CapturedFile {
        self.capture
            .0
            .as_ref()
            .expect("media capture remains owned")
    }
}

impl VideoSource for CapturedVideoSource {
    fn length(&self) -> u64 {
        self.file().length()
    }

    fn read_at(
        &mut self,
        offset: u64,
        bytes: &mut [u8],
        control: VideoIo<'_>,
    ) -> io::Result<usize> {
        control.check()?;
        let end = offset
            .checked_add(bytes.len() as u64)
            .filter(|end| *end <= self.length())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Media read outside capture")
            })?;
        let length = u32::try_from(end - offset).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "Media read length overflows")
        })?;
        let budget = midnight_work_budget::shared_io();
        let _permit = budget
            .acquire(
                midnight_work_budget::storage_admission(self.confirmed_local_ssd),
                || control.check().is_err(),
            )
            .ok_or_else(|| {
                control
                    .check()
                    .err()
                    .unwrap_or_else(|| io::ErrorKind::Interrupted.into())
            })?;
        control.check()?;
        // The file itself, not the helper, failed: say so in the session's error.
        let read = self
            .file()
            .read_at(offset, length)
            .map_err(|error| io::Error::other(format!("This video could not be read: {error}")))?;
        control.check()?;
        if read.len() > bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Media capture exceeded read limit",
            ));
        }
        bytes[..read.len()].copy_from_slice(&read);
        Ok(read.len())
    }
}

struct RetainedCapture(Option<CapturedFile>);

impl Drop for RetainedCapture {
    fn drop(&mut self) {
        if let Some(file) = self.0.take() {
            let _ = file.release_on_worker();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pipe_call_must_use_the_supplied_session_cancellation_flag() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let different = AtomicBool::new(false);
        let supervisor = Arc::new(Supervisor::new(cancelled.clone()));
        let deadline = Instant::now() + Duration::from_secs(1);
        let error = supervisor
            .arm(VideoIo {
                deadline,
                cancelled: &different,
            })
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);

        let active = supervisor
            .arm(VideoIo {
                deadline,
                cancelled: &cancelled,
            })
            .unwrap();
        assert_eq!(supervisor.progress().deadline, Some(deadline));
        drop(active);
        assert!(supervisor.progress().deadline.is_none());
    }

    #[test]
    fn retirement_signal_cannot_complete_the_receipt_without_native_proof() {
        let supervisor = Arc::new(Supervisor::new(Arc::new(AtomicBool::new(false))));
        let pending = VideoRetirement(supervisor.clone());
        supervisor.request_retirement();
        assert!(supervisor.progress().retiring);
        assert_eq!(
            pending.wait(Instant::now()).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(supervisor.progress().retirement.is_none());

        supervisor.complete_retirement(Ok(None));
        VideoRetirement(supervisor).wait(Instant::now()).unwrap();
    }

    #[test]
    fn an_unconfirmed_retirement_remains_an_error_on_later_receipts() {
        let supervisor = Arc::new(Supervisor::new(Arc::new(AtomicBool::new(false))));
        supervisor.request_retirement();
        supervisor.complete_retirement(Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Native termination is unconfirmed",
        )));
        for _ in 0..2 {
            let error = VideoRetirement(supervisor.clone())
                .wait(Instant::now())
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(error.to_string(), "Native termination is unconfirmed");
        }
    }

    #[test]
    fn the_supervisor_leaves_wait_on_expired_deadline_or_session_cancellation() {
        for cancel in [false, true] {
            let supervisor = Supervisor::new(Arc::new(AtomicBool::new(cancel)));
            if !cancel {
                supervisor.progress().deadline = Some(Instant::now());
            }
            supervisor.wait_for_retirement();
            assert!(supervisor.progress().retiring);
        }
    }

    /// The Windows loader's status for a library it could not open, or ld.so's.
    const LIBRARY_MISSING: ExitStatus = if cfg!(windows) {
        ExitStatus::Code(0xC000_0135)
    } else {
        ExitStatus::Code(127)
    };

    #[test]
    fn a_helper_that_ended_before_ready_is_reported_with_its_exit_and_not_as_a_short_read() {
        // A helper that cannot start closes its pipe: the Ready read finds its end.
        let mut ended: &[u8] = &[];
        let error = midnight_worker_sdk::read_policy_ready(&mut ended)
            .map_err(helper_pipe_failure)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(error.to_string(), "The video helper stopped unexpectedly");
        // Once the supervisor confirmed the termination, the receipt adds the exit status.
        let supervisor = Arc::new(Supervisor::new(Arc::new(AtomicBool::new(false))));
        let receipt = VideoRetirement(supervisor.clone());
        let unexplained = receipt.explain(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            HelperStopped::default(),
        ));
        assert_eq!(stopped_helper(&unexplained), Some(HelperStopped::default()));
        supervisor.request_retirement();
        supervisor.complete_retirement(Ok(Some(LIBRARY_MISSING)));
        receipt.wait(Instant::now()).unwrap();
        let explained = receipt.explain(error);
        assert_eq!(explained.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(
            stopped_helper(&explained),
            Some(HelperStopped::ended(LIBRARY_MISSING))
        );
        assert_eq!(
            explained.to_string(),
            format!("The video helper stopped: {}", LIBRARY_MISSING.reason())
        );
        #[cfg(windows)]
        assert_eq!(
            explained.to_string(),
            "The video helper stopped: a required library is missing or cannot be read \
             (STATUS_DLL_NOT_FOUND, 0xC0000135)"
        );
    }

    #[test]
    fn only_a_closed_pipe_or_a_helper_gone_at_launch_is_reported_as_a_stopped_helper() {
        for kind in [io::ErrorKind::UnexpectedEof, io::ErrorKind::BrokenPipe] {
            let error = helper_pipe_failure(io::Error::from(kind));
            assert_eq!(error.kind(), kind);
            assert_eq!(stopped_helper(&error), Some(HelperStopped::default()));
        }
        for kind in [io::ErrorKind::TimedOut, io::ErrorKind::InvalidData] {
            assert_eq!(stopped_helper(&helper_pipe_failure(kind.into())), None);
        }
        let gone = io::Error::new(io::ErrorKind::BrokenPipe, WorkerStopped(LIBRARY_MISSING));
        assert_eq!(
            stopped_helper(&launch_failure(gone, HelperStopped::ended)),
            Some(HelperStopped::ended(LIBRARY_MISSING))
        );
        let refused = launch_failure(
            io::Error::other("CreateProcessW for restricted worker"),
            HelperStopped::ended,
        );
        assert_eq!(stopped_helper(&refused), None);
        // A receipt never invents a status: an unrelated error is kept as it is.
        let supervisor = Arc::new(Supervisor::new(Arc::new(AtomicBool::new(false))));
        supervisor.complete_retirement(Ok(Some(LIBRARY_MISSING)));
        let other = VideoRetirement(supervisor).explain(io::Error::other("Media worker failed"));
        assert_eq!(other.to_string(), "Media worker failed");
    }

    /// A fresh folder for this test's files, removed when it ends.
    struct Folder(std::path::PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "midnight-media-host-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_library_the_helper_could_not_load_is_named_with_why() {
        let directory = Folder::new("libraries");
        let present = directory.0.join("present.dll");
        std::fs::write(&present, b"library").unwrap();
        let missing = directory.0.join("missing.dll");
        let stopped = diagnose(&[present.clone(), missing.clone()], LIBRARY_MISSING);
        // The present file is not granted to the workers' sandbox on Windows (nothing grants
        // it here), so it is the one reported there; on Linux the missing one is.
        let expected = if cfg!(windows) {
            format!(
                "The video helper could not load its libraries (access denied): {}",
                present.display()
            )
        } else {
            format!(
                "The video helper could not load its libraries: {} is missing",
                missing.display()
            )
        };
        assert_eq!(stopped.to_string(), expected);
        assert_eq!(
            diagnose(std::slice::from_ref(&missing), LIBRARY_MISSING).to_string(),
            format!(
                "The video helper could not load its libraries: {} is missing",
                missing.display()
            )
        );
        // Any other exit is reported by its status, without inspecting the libraries.
        let crashed = if cfg!(windows) {
            ExitStatus::Code(0xC000_0005)
        } else {
            ExitStatus::Signal(11)
        };
        let stopped = diagnose(&[missing], crashed);
        assert_eq!(stopped, HelperStopped::ended(crashed));
        assert_eq!(
            stopped.to_string(),
            format!("The video helper stopped: {}", crashed.reason())
        );
    }
}
