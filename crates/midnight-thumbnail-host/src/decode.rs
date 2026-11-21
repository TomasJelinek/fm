//! Killable helper supervision and bounded read-only content brokering. Backend threads only.
use midnight_plugin_host::CapturedFile;
use midnight_thumbnail_api::{self as api, Frame, Message, Request};
use midnight_worker_platform::{ExitStatus, UnloadableLibrary, WorkerStopped};
use std::{
    fmt,
    io::{self, BufReader},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

/// A thumbnail helper whose loader could not load a library: no preview of its kind can be
/// made. The service reports the first one as a notice, so that missing previews have a
/// reason; the library is named with why (missing, or denied to the helpers' sandbox).
#[derive(Debug)]
pub struct HelperCannotStart {
    pub helper: PathBuf,
    pub status: ExitStatus,
    pub library: Option<UnloadableLibrary>,
}

impl fmt::Display for HelperCannotStart {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self.helper.file_name().map_or_else(
            || self.helper.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        match &self.library {
            Some(library) => write!(formatter, "the thumbnail helper {name} {library}"),
            None => write!(
                formatter,
                "the thumbnail helper {name} could not start: {}",
                self.status.reason()
            ),
        }
    }
}

impl std::error::Error for HelperCannotStart {}

/// A helper that ended because its loader could not load a library, with that library.
fn cannot_start(helper: &Path, status: ExitStatus) -> io::Error {
    io::Error::other(HelperCannotStart {
        helper: helper.to_owned(),
        status,
        library: midnight_worker_platform::unloadable_library(
            &midnight_worker_platform::worker_libraries(helper),
        ),
    })
}

/// Native and virtual sources implement this host-owned contract; identifiers never become paths.
pub trait Source: Send + Sync {
    fn length(&self) -> u64;
    fn version(&self) -> Vec<u8>;
    fn validate(&self) -> io::Result<()>;
    fn read_at(&self, offset: u64, length: u32) -> io::Result<Vec<u8>>;
    /// A cancellable preview read. Providers without cooperative support drain the
    /// admitted call and suppress its result; real capacity remains occupied.
    fn read_at_cooperative(
        &self,
        offset: u64,
        length: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> io::Result<Vec<u8>> {
        if cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let bytes = self.read_at(offset, length)?;
        if cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        Ok(bytes)
    }
    fn confirmed_local_ssd(&self) -> bool {
        false
    }
}

impl Source for CapturedFile {
    fn length(&self) -> u64 {
        self.length()
    }
    fn version(&self) -> Vec<u8> {
        self.stable_version()
    }
    fn validate(&self) -> io::Result<()> {
        self.revalidate().map_err(io::Error::other)
    }
    fn read_at(&self, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        self.read_at(offset, length).map_err(io::Error::other)
    }
}

/// The provider answered that it cannot decode the source (MTH1 `Failed`), as opposed to
/// failing, timing out or being cancelled.
#[derive(Debug)]
pub struct Rejected;

impl fmt::Display for Rejected {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Thumbnail decoder rejected the source")
    }
}

impl std::error::Error for Rejected {}

/// Whether the provider rejected the source itself.
pub fn rejected(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.downcast_ref::<Rejected>().is_some())
}

/// Memory for one provider process.
pub const PROVIDER_MEMORY: u64 = 512 * 1024 * 1024;
/// Memory for a platform video helper, which also maps the system's media frameworks (the
/// worker SDK's maximum).
pub const PLATFORM_MEMORY: u64 = 1024 * 1024 * 1024;

/// Supervise one decoder process on a bounded backend worker. A blocked source OS call retains
/// its worker until it actually returns; killing the decoder does not pretend to cancel that call.
pub fn decode(
    helper: &Path,
    source: &dyn Source,
    request: Request,
    cancel: &AtomicBool,
    timeout: Duration,
) -> io::Result<Frame> {
    decode_in(helper, source, request, cancel, timeout, PROVIDER_MEMORY)
}

/// [`decode`] with a process memory limit of `memory_bytes`.
pub fn decode_in(
    helper: &Path,
    source: &dyn Source,
    request: Request,
    cancel: &AtomicBool,
    timeout: Duration,
    memory_bytes: u64,
) -> io::Result<Frame> {
    decode_limited(
        helper,
        source,
        request,
        &|| cancel.load(Ordering::Acquire),
        timeout,
        memory_bytes,
    )
}

/// The same restricted decoder with caller-owned cooperative cancellation, including
/// linked lifetimes. No polling thread or replacement worker is created for cancellation.
pub fn decode_cooperative(
    helper: &Path,
    source: &dyn Source,
    request: Request,
    cancelled: &(dyn Fn() -> bool + Sync),
    timeout: Duration,
) -> io::Result<Frame> {
    decode_limited(helper, source, request, cancelled, timeout, PROVIDER_MEMORY)
}

fn decode_limited(
    helper: &Path,
    source: &dyn Source,
    request: Request,
    cancelled: &(dyn Fn() -> bool + Sync),
    timeout: Duration,
    memory_bytes: u64,
) -> io::Result<Frame> {
    if cancelled() {
        return Err(cancellation_error());
    }
    if request.source_length != source.length() {
        return Err(io::Error::other("Source length changed"));
    }
    api::rgba_length(request.width, request.height)?;
    source.validate()?;
    let version = source.version();
    let worker = midnight_worker_platform::spawn(
        helper,
        midnight_worker_platform::Limits {
            memory_bytes,
            cpu_seconds: 8,
        },
    )
    .map_err(|error| {
        // The startup message found the helper gone.
        match error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<WorkerStopped>())
        {
            Some(&WorkerStopped(status)) if status.library_not_loaded() => {
                cannot_start(helper, status)
            }
            _ => error,
        }
    })?;
    let midnight_worker_platform::Worker {
        mut input,
        output,
        mut process,
    } = worker;
    let start = Instant::now();
    let broker_stop = AtomicBool::new(false);
    let result = std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::sync_channel(1);
        let broker_stop = &broker_stop;
        let broker = scope.spawn(move || {
            let result = (|| {
                let mut output = BufReader::new(output);
                midnight_worker_sdk::read_ready(&mut output)?;
                api::write_message(&mut input, &Message::Request(request.clone()))?;
                let mut total = 0u64;
                for _ in 0..8192 {
                    if cancelled() || broker_stop.load(Ordering::Acquire) {
                        return Err(cancellation_error());
                    }
                    match api::read_message(&mut output)? {
                        Message::Read { offset, length } => {
                            let end = offset
                                .checked_add(u64::from(length))
                                .filter(|end| *end <= source.length())
                                .ok_or_else(|| io::Error::other("Read outside captured content"))?;
                            total += u64::from(length);
                            if total > 256 * 1024 * 1024 {
                                return Err(io::Error::other(
                                    "Decoder source-read budget exhausted",
                                ));
                            }
                            let bytes = {
                                let budget = midnight_work_budget::shared_io();
                                let _permit = budget
                                    .acquire(
                                        midnight_work_budget::storage_admission(
                                            source.confirmed_local_ssd(),
                                        ),
                                        || cancelled() || broker_stop.load(Ordering::Acquire),
                                    )
                                    .ok_or_else(cancellation_error)?;
                                source.read_at_cooperative(offset, length, &|| {
                                    cancelled() || broker_stop.load(Ordering::Acquire)
                                })?
                            };
                            if bytes.len() as u64 != end - offset {
                                return Err(io::Error::other("Source returned a short read"));
                            }
                            api::write_message(&mut input, &Message::Data(bytes))?;
                        }
                        Message::Frame(frame)
                            if frame.id == request.id
                                && frame.frame == request.frame
                                && frame.width <= request.width
                                && frame.height <= request.height =>
                        {
                            return Ok(frame);
                        }
                        Message::Failed => {
                            return Err(io::Error::new(io::ErrorKind::Unsupported, Rejected));
                        }
                        _ => return Err(io::Error::other("Unexpected thumbnail worker response")),
                    }
                }
                Err(io::Error::other("Decoder request-count budget exhausted"))
            })();
            let _ = sender.send(result);
        });
        let result = loop {
            if cancelled() {
                break Err(cancellation_error());
            }
            if start.elapsed() >= timeout {
                break Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Thumbnail decoder timed out",
                ));
            }
            match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(result) => break result,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Err(io::Error::other("Thumbnail broker stopped"));
                }
            }
        };
        broker_stop.store(true, Ordering::Release);
        let _ = process.kill();
        let exited = process.wait_status();
        broker
            .join()
            .map_err(|_| io::Error::other("Thumbnail broker panicked"))?;
        explain_closed_stream(result, exited, helper)
    })?;
    source.validate()?;
    if source.version() != version {
        return Err(io::Error::other("Source version changed during decode"));
    }
    Ok(result)
}

fn cancellation_error() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Thumbnail request cancelled")
}

/// A protocol stream that ended early is reported with the worker's exit: a library its
/// loader could not load ([`HelperCannotStart`]), or why else it ended (a crash, its exit
/// code or signal, also after the kill above), or a wait that could not confirm the exit.
/// A clean exit keeps the stream error.
fn explain_closed_stream<T>(
    result: io::Result<T>,
    exited: io::Result<ExitStatus>,
    helper: &Path,
) -> io::Result<T> {
    match (result, exited) {
        (Err(error), exited)
            if error.kind() == io::ErrorKind::UnexpectedEof
                && !matches!(exited, Ok(status) if status.success()) =>
        {
            if let Ok(status) = exited
                && status.library_not_loaded()
            {
                return Err(cannot_start(helper, status));
            }
            let exit = match exited {
                Ok(status) => status.reason(),
                Err(wait) => format!("its exit could not be confirmed: {wait}"),
            };
            Err(io::Error::other(format!(
                "Thumbnail worker closed its protocol stream; {exit}"
            )))
        }
        (result, _) => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_providers_own_rejection_lets_the_platform_helper_try() {
        let rejected = io::Error::new(io::ErrorKind::Unsupported, Rejected);
        assert!(super::rejected(&rejected));
        assert_eq!(
            rejected.to_string(),
            "Thumbnail decoder rejected the source"
        );
        for other in [
            io::Error::new(
                io::ErrorKind::Unsupported,
                "Thumbnail decoder rejected the source",
            ),
            io::Error::new(io::ErrorKind::TimedOut, "Thumbnail decoder timed out"),
            cancellation_error(),
        ] {
            assert!(!super::rejected(&other), "{other}");
        }
    }

    struct Untouched;
    impl Source for Untouched {
        fn length(&self) -> u64 {
            panic!("cancelled decode inspected source")
        }
        fn version(&self) -> Vec<u8> {
            panic!("cancelled decode inspected source")
        }
        fn validate(&self) -> io::Result<()> {
            panic!("cancelled decode inspected source")
        }
        fn read_at(&self, _: u64, _: u32) -> io::Result<Vec<u8>> {
            panic!("cancelled decode read source")
        }
    }
    #[test]
    fn linked_cancellation_is_checked_before_provider_or_source_access() {
        let parent = midnight_core::Cancellation::default();
        let child = parent.child();
        parent.cancel();
        let error = decode_cooperative(
            Path::new("missing-helper"),
            &Untouched,
            Request {
                id: 1,
                source_length: 0,
                width: 128,
                height: 128,
                frame: 0,
            },
            &|| child.is_cancelled(),
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    }

    fn closed_stream() -> io::Result<()> {
        Err(io::ErrorKind::UnexpectedEof.into())
    }

    #[test]
    fn a_closed_stream_is_explained_by_the_exit_code_signal_or_an_unconfirmed_wait() {
        let explained = |exited| {
            explain_closed_stream(closed_stream(), Ok(exited), Path::new("helper"))
                .unwrap_err()
                .to_string()
        };
        // Windows reports all 32 bits, such as the loader's NTSTATUS for a missing library
        // or an access violation; Linux reports the terminating signal instead, such as
        // SIGSEGV. The platform words each status (`ExitStatus::reason`).
        for status in [
            ExitStatus::Code(0xC000_0005),
            ExitStatus::Code(101),
            ExitStatus::Signal(11),
        ] {
            assert_eq!(
                explained(status),
                format!(
                    "Thumbnail worker closed its protocol stream; {}",
                    status.reason()
                )
            );
        }
        let wait = io::Error::other("wait failed");
        let error =
            explain_closed_stream(closed_stream(), Err(wait), Path::new("helper")).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Thumbnail worker closed its protocol stream; its exit could not be confirmed: \
             wait failed"
        );
    }

    #[test]
    fn other_outcomes_are_not_rewritten_by_the_exit() {
        // A clean exit leaves the stream error itself.
        let helper = Path::new("helper");
        let error =
            explain_closed_stream(closed_stream(), Ok(ExitStatus::Code(0)), helper).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        // Cancellation keeps its kind: the service uses it to skip the failure backoff.
        let killed = Ok(ExitStatus::Code(1));
        let cancelled = explain_closed_stream::<()>(Err(cancellation_error()), killed, helper);
        assert_eq!(cancelled.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert_eq!(
            explain_closed_stream(Ok(7), Ok(ExitStatus::Signal(9)), helper).unwrap(),
            7
        );
    }

    #[test]
    fn a_helper_that_cannot_load_its_libraries_names_the_library_and_why() {
        // The loader's status: STATUS_DLL_NOT_FOUND on Windows, ld.so's 127 on Linux.
        let status = if cfg!(windows) {
            ExitStatus::Code(0xC000_0135)
        } else {
            ExitStatus::Code(127)
        };
        assert!(status.library_not_loaded());
        let directory =
            std::env::temp_dir().join(format!("midnight-thumbnail-helper-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let helper = directory.join("helper.exe");
        let library = directory.join(if cfg!(windows) {
            "codec.dll"
        } else {
            "libcodec.so.1"
        });
        let error = explain_closed_stream::<()>(closed_stream(), Ok(status), &helper).unwrap_err();
        let failure = error
            .get_ref()
            .and_then(|error| error.downcast_ref::<HelperCannotStart>())
            .expect("a helper that cannot start");
        assert_eq!(failure.library, None);
        assert_eq!(
            error.to_string(),
            format!(
                "the thumbnail helper helper.exe could not start: {}",
                status.reason()
            )
        );
        // A library beside the helper that the helpers may not load is named; on Windows
        // nothing grants this one to the helpers' sandbox.
        std::fs::write(&library, b"library").unwrap();
        let error = explain_closed_stream::<()>(closed_stream(), Ok(status), &helper).unwrap_err();
        let _ = std::fs::remove_dir_all(&directory);
        if cfg!(windows) {
            assert_eq!(
                error.to_string(),
                format!(
                    "the thumbnail helper helper.exe could not load its libraries (access \
                     denied): {}",
                    library.display()
                )
            );
        }
    }
}
