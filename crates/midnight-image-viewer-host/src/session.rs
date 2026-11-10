//! Killable persistent worker and bounded read-only broker. All entry points run on backend workers.
use crate::{DocumentMetadata, Frame, Metadata, Provider, Viewport};
use midnight_image_viewer_api::{self as api, Message};
use std::{
    fs::File,
    io::{self, BufReader},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

/// Host-owned native/virtual source contract. Implementations preserve their raw provider identity.
/// Reads must be bounded and validate captured identity; blocking native calls retain their worker
/// until they actually return. Killing the decoder cannot cancel an arbitrary source OS call.
pub trait Source: Send + Sync {
    fn length(&self) -> u64;
    fn version(&self) -> Vec<u8>;
    fn validate(&self) -> io::Result<()>;
    fn read_at(&self, offset: u64, length: u32) -> io::Result<Vec<u8>>;
    fn confirmed_local_ssd(&self) -> bool {
        false
    }
}

impl Source for midnight_plugin_host::CapturedFile {
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

/// One persistent restricted decoder. Creation, rendering, close and Drop are backend-only.
/// Memory is capped at 512 MiB, retained RGBA at 128 MiB; cumulative process CPU is 60 seconds.
/// Each operation additionally has the supplied wall-clock deadline and cancellation flag.
pub struct ImageSession {
    input: File,
    output: BufReader<File>,
    process: midnight_worker_platform::Process,
    source: Arc<dyn Source>,
    length: u64,
    version: Vec<u8>,
    metadata: Metadata,
    document: Option<DocumentMetadata>,
    request_id: u64,
    failed: bool,
}

impl ImageSession {
    /// Validate the installed provider and captured source, then decode and retain original pixels.
    pub fn open(
        provider: &Provider,
        source: Arc<dyn Source>,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> io::Result<Self> {
        check_cancel(cancel)?;
        provider.validate()?;
        source.validate()?;
        let length = source.length();
        let version = source.version();
        let worker = midnight_worker_platform::spawn(
            &provider.executable,
            midnight_worker_platform::Limits {
                memory_bytes: 512 * 1024 * 1024,
                cpu_seconds: 60,
            },
        )?;
        let mut session = Self {
            input: worker.input,
            output: BufReader::new(worker.output),
            process: worker.process,
            source,
            length,
            version,
            metadata: Metadata {
                width: 1,
                height: 1,
            },
            request_id: 0,
            document: None,
            failed: false,
        };
        let metadata = session.exchange(cancel, timeout, |input, output, source, stopped| {
            midnight_worker_sdk::read_ready(output)?;
            api::write_message(
                input,
                &Message::Open {
                    source_length: length,
                },
            )?;
            let mut read_total = 0u64;
            for _ in 0..8192 {
                check_cancel(cancel)?;
                check_cancel(stopped)?;
                match api::read_message(output)? {
                    Message::Read {
                        offset,
                        length: amount,
                    } => {
                        let end = offset
                            .checked_add(u64::from(amount))
                            .filter(|end| *end <= length)
                            .ok_or_else(|| {
                                io::Error::other("Image worker read outside captured source")
                            })?;
                        read_total += u64::from(amount);
                        if read_total > 256 * 1024 * 1024 {
                            return Err(io::Error::other("Image source-read budget exhausted"));
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
                                            || stopped.load(Ordering::Acquire)
                                    },
                                )
                                .ok_or_else(cancelled)?;
                            source.validate()?;
                            let bytes = source.read_at(offset, amount)?;
                            source.validate()?;
                            bytes
                        };
                        if bytes.len() as u64 != end - offset {
                            return Err(io::Error::other("Image source returned a short read"));
                        }
                        api::write_message(input, &Message::Data(bytes))?;
                    }
                    Message::Metadata(metadata) if provider.protocol == 1 => {
                        return Ok((metadata, None));
                    }
                    Message::DocumentMetadata(document) if provider.protocol == 2 => {
                        document.validate()?;
                        return Ok((document.pages[0], Some(document)));
                    }
                    Message::Failure => return Err(rejected()),
                    _ => return Err(io::Error::other("Unexpected image-open response")),
                }
            }
            Err(io::Error::other(
                "Image source request-count budget exhausted",
            ))
        })?;
        session.metadata = metadata.0;
        session.document = metadata.1;
        session.validate_source()?;
        check_cancel(cancel)?;
        Ok(session)
    }

    pub fn metadata(&self) -> Metadata {
        self.metadata
    }

    /// Bounded document pages; ordinary images have one page.
    pub fn page_count(&self) -> usize {
        self.document
            .as_ref()
            .map_or(1, |document| document.pages.len())
    }

    pub fn is_document(&self) -> bool {
        self.document.is_some()
    }

    pub fn page_metadata(&self, page: usize) -> Option<Metadata> {
        self.document.as_ref().map_or_else(
            || (page == 0).then_some(self.metadata),
            |document| document.pages.get(page).copied(),
        )
    }

    /// Render retained original pixels into a physical viewport. Any error ends this session;
    /// failed or cancelled workers cannot publish another frame or reuse a changed source.
    pub fn render(
        &mut self,
        viewport: Viewport,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> io::Result<Frame> {
        self.render_page(0, viewport, cancel, timeout)
    }

    /// Render one validated document page, or page zero of a single image.
    pub fn render_page(
        &mut self,
        page: usize,
        viewport: Viewport,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> io::Result<Frame> {
        if self.failed {
            return Err(io::Error::other("Image session already ended"));
        }
        let result = (|| {
            check_cancel(cancel)?;
            viewport.validate()?;
            if self.page_metadata(page).is_none() {
                return Err(io::Error::other("Document page is outside the page range"));
            }
            self.validate_source()?;
            self.request_id = self
                .request_id
                .checked_add(1)
                .ok_or_else(|| io::Error::other("Image session request limit"))?;
            let request_id = self.request_id;
            let message = if self.document.is_some() {
                Message::PageViewport {
                    id: request_id,
                    page: page as u32,
                    viewport,
                }
            } else {
                Message::Viewport {
                    id: request_id,
                    viewport,
                }
            };
            let frame = self.exchange(cancel, timeout, |input, output, _, _| {
                api::write_message(input, &message)?;
                match crate::raster_reply::read(output, viewport.width, viewport.height)? {
                    Message::Rgba { id, frame }
                        if id == request_id
                            && frame.width == viewport.width
                            && frame.height == viewport.height =>
                    {
                        Ok(frame)
                    }
                    Message::Failure => Err(rejected()),
                    _ => Err(io::Error::other("Unexpected image-viewport response")),
                }
            })?;
            self.validate_source()?;
            check_cancel(cancel)?;
            Ok(frame)
        })();
        if result.is_err() {
            self.failed = true;
            let _ = self.process.kill();
        }
        result
    }

    /// Terminate and reap on the owning backend worker. UI close handlers only signal cancellation.
    pub fn validate(&mut self, cancel: &AtomicBool) -> io::Result<()> {
        let result = check_cancel(cancel)
            .and_then(|()| self.validate_source())
            .and_then(|()| check_cancel(cancel));
        if self.failed || result.is_err() {
            self.failed = true;
            let _ = self.process.kill();
            return result.and(Err(io::Error::other("Image session already ended")));
        }
        result
    }

    /// Terminate and reap on the owning backend worker. UI close handlers only signal cancellation.
    pub fn close(mut self) {
        let _ = self.process.kill();
    }

    fn validate_source(&self) -> io::Result<()> {
        self.source.validate()?;
        if self.source.length() != self.length || self.source.version() != self.version {
            return Err(io::Error::other("Image source version changed"));
        }
        Ok(())
    }

    fn exchange<T: Send>(
        &mut self,
        cancel: &AtomicBool,
        timeout: Duration,
        operation: impl FnOnce(
            &mut File,
            &mut BufReader<File>,
            &dyn Source,
            &AtomicBool,
        ) -> io::Result<T>
        + Send,
    ) -> io::Result<T> {
        check_cancel(cancel)?;
        let start = Instant::now();
        let stopped = AtomicBool::new(false);
        let input = &mut self.input;
        let output = &mut self.output;
        let source = self.source.as_ref();
        let process = &mut self.process;
        let result = std::thread::scope(|scope| {
            let (sender, receiver) = mpsc::sync_channel(1);
            let stopped = &stopped;
            let broker = scope.spawn(move || {
                let _ = sender.send(operation(input, output, source, stopped));
            });
            let result = loop {
                if cancel.load(Ordering::Acquire) {
                    break Err(cancelled());
                }
                if start.elapsed() >= timeout {
                    break Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Image viewer timed out",
                    ));
                }
                match receiver.recv_timeout(Duration::from_millis(10)) {
                    Ok(result) => break result,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        break Err(io::Error::other("Image broker stopped"));
                    }
                }
            };
            stopped.store(true, Ordering::Release);
            if result.is_err() {
                let _ = process.kill();
            }
            broker
                .join()
                .map_err(|_| io::Error::other("Image broker panicked"))?;
            result
        });
        if result.is_err() {
            self.failed = true;
            let _ = self.process.kill();
        }
        result
    }
}

fn check_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(cancelled())
    } else {
        Ok(())
    }
}
fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Image viewer cancelled")
}
fn rejected() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "Image decoder rejected the source",
    )
}
