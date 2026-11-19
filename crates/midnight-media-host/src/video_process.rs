//! Ready-gated playback process adapter and bounded captured reads.

use crate::{
    video_installation::{LaunchPins, VideoInstallation},
    video_transport::{RestrictedVideoPipe, VideoIo, VideoSource},
};
use midnight_plugin_host::CapturedFile;
use midnight_worker_platform::{Process, ResourcePolicy, Worker};
use std::{
    fs::File,
    io::{self, BufReader, Read, Write},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

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
        let pins = installation.capture()?;
        if let Err(error) = control.check() {
            pins.release()?;
            return Err(error);
        }
        let Worker {
            input,
            output,
            process,
        } = match midnight_worker_platform::spawn_with_policy(&installation.worker(), policy) {
            Ok(worker) => worker,
            Err(error) => {
                pins.release()?;
                return Err(error);
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
        pipe.call(control, |pipe| {
            midnight_worker_sdk::read_policy_ready(pipe.output()?)?;
            let guard = pins.lock().unwrap_or_else(|error| error.into_inner());
            guard
                .as_ref()
                .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?
                .revalidate()
        })?;
        Ok(pipe)
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
        self.call(control, |pipe| pipe.output()?.read_exact(bytes))
    }

    fn send(&mut self, parts: &[&[u8]], control: VideoIo<'_>) -> io::Result<()> {
        self.call(control, |pipe| {
            let input = pipe.input()?;
            for part in parts {
                input.write_all(part)?;
            }
            input.flush()
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
    pub fn wait(self, deadline: Instant) -> io::Result<()> {
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

    fn complete_retirement(&self, result: io::Result<()>) {
        self.progress().retirement = Some(result);
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
    fn reap(&mut self) -> io::Result<()> {
        let Some(process) = self.process.as_mut() else {
            return Ok(());
        };
        let _ = process.kill();
        let termination = process.wait_terminated();
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
        Ok(())
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
        let read = self
            .file()
            .read_at(offset, length)
            .map_err(io::Error::other)?;
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

        supervisor.complete_retirement(Ok(()));
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
}
