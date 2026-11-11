//! Killable helper supervision and bounded read-only content brokering. Backend threads only.
use midnight_plugin_host::CapturedFile;
use midnight_thumbnail_api::{self as api, Frame, Message, Request};
use std::{
    io::{self, BufReader},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

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

/// Supervise one decoder process on a bounded backend worker. A blocked source OS call retains
/// its worker until it actually returns; killing the decoder does not pretend to cancel that call.
pub fn decode(
    helper: &Path,
    source: &dyn Source,
    request: Request,
    cancel: &AtomicBool,
    timeout: Duration,
) -> io::Result<Frame> {
    if cancel.load(Ordering::Acquire) {
        return Err(cancelled());
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
            memory_bytes: 512 * 1024 * 1024,
            cpu_seconds: 8,
        },
    )?;
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
                    if cancel.load(Ordering::Acquire) || broker_stop.load(Ordering::Acquire) {
                        return Err(cancelled());
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
                                        midnight_work_budget::storage_cost(
                                            source.confirmed_local_ssd(),
                                        ),
                                        || {
                                            cancel.load(Ordering::Acquire)
                                                || broker_stop.load(Ordering::Acquire)
                                        },
                                    )
                                    .ok_or_else(cancelled)?;
                                source.read_at_cooperative(offset, length, &|| {
                                    cancel.load(Ordering::Acquire)
                                        || broker_stop.load(Ordering::Acquire)
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
                            return Err(io::Error::new(
                                io::ErrorKind::Unsupported,
                                "Thumbnail decoder rejected the source",
                            ));
                        }
                        _ => return Err(io::Error::other("Unexpected thumbnail worker response")),
                    }
                }
                Err(io::Error::other("Decoder request-count budget exhausted"))
            })();
            let _ = sender.send(result);
        });
        let result = loop {
            if cancel.load(Ordering::Acquire) {
                break Err(cancelled());
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
        let exited = process.wait();
        broker
            .join()
            .map_err(|_| io::Error::other("Thumbnail broker panicked"))?;
        match (result, exited) {
            (Err(error), Err(exit)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                Err(io::Error::other(format!(
                    "Thumbnail worker closed its protocol stream: {exit}"
                )))
            }
            (result, _) => result,
        }
    })?;
    source.validate()?;
    if source.version() != version {
        return Err(io::Error::other("Source version changed during decode"));
    }
    Ok(result)
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Thumbnail request cancelled")
}
