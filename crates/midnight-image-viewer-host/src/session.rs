//! Killable persistent worker and bounded read-only broker. All entry points run on backend workers.
use crate::{
    DocumentMetadata, Frame, Metadata, PreparedImageWorker, Provider, Viewport,
    startup::{ProviderIdentity, WorkerTransport, cancelled, check_cancel},
};
use midnight_image_viewer_api::{self as api, Message};
use std::{
    fs::File,
    io::{self, BufReader},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
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
    transport: WorkerTransport,
    source: CapturedSource,
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
        #[cfg(feature = "startup-diagnostics")]
        let mut startup = midnight_worker_platform::startup_diagnostics::Trace::new("image_open");
        check_cancel(cancel)?;
        let identity = ProviderIdentity::capture(provider)?;
        #[cfg(feature = "startup-diagnostics")]
        startup.mark("provider_validated");
        let source = CapturedSource::capture(source)?;
        #[cfg(feature = "startup-diagnostics")]
        startup.mark("source_validated");
        let transport = WorkerTransport::spawn(provider)?;
        #[cfg(feature = "startup-diagnostics")]
        startup.mark("spawned");
        Self::new(transport, source).open_source(
            provider,
            identity,
            cancel,
            timeout,
            true,
            #[cfg(feature = "startup-diagnostics")]
            &mut startup,
        )
    }

    /// Consume one prepared source-free child and open a newly captured source exactly once.
    /// Invalid preparation fails before Open. Any Open, broker or decode error is terminal and
    /// must not trigger an automatic cold retry. Creation, adoption and cleanup are backend-only.
    pub fn open_prepared(
        provider: &Provider,
        prepared: PreparedImageWorker,
        source: Arc<dyn Source>,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> io::Result<Self> {
        #[cfg(feature = "startup-diagnostics")]
        let mut startup =
            midnight_worker_platform::startup_diagnostics::Trace::new("image_open_prepared");
        check_cancel(cancel)?;
        if !prepared.catalog_matches(provider) {
            return Err(io::Error::other("Prepared image worker catalog changed"));
        }
        #[cfg(feature = "startup-diagnostics")]
        startup.mark("catalog_matched");
        let source = CapturedSource::capture(source)?;
        #[cfg(feature = "startup-diagnostics")]
        startup.mark("source_validated");
        check_cancel(cancel)?;
        let (transport, identity) = prepared.into_parts();
        Self::new(transport, source).open_source(
            provider,
            identity,
            cancel,
            timeout,
            false,
            #[cfg(feature = "startup-diagnostics")]
            &mut startup,
        )
    }

    fn new(transport: WorkerTransport, source: CapturedSource) -> Self {
        Self {
            transport,
            source,
            metadata: Metadata {
                width: 1,
                height: 1,
            },
            request_id: 0,
            document: None,
            failed: false,
        }
    }

    fn open_source(
        mut self,
        provider: &Provider,
        identity: ProviderIdentity,
        cancel: &AtomicBool,
        timeout: Duration,
        read_ready: bool,
        #[cfg(feature = "startup-diagnostics")]
        startup: &mut midnight_worker_platform::startup_diagnostics::Trace,
    ) -> io::Result<Self> {
        let length = self.source.length;
        let metadata = self.exchange(cancel, timeout, |input, output, source, stopped| {
            #[cfg(feature = "startup-diagnostics")]
            startup.mark("broker_started");
            if read_ready {
                midnight_worker_sdk::read_ready(output)?;
            }
            #[cfg(feature = "startup-diagnostics")]
            startup.mark("ready");
            check_cancel(cancel)?;
            check_cancel(stopped)?;
            identity.validate_for(provider)?;
            #[cfg(feature = "startup-diagnostics")]
            startup.mark("identity_revalidated");
            check_cancel(cancel)?;
            check_cancel(stopped)?;
            api::write_message(
                input,
                &Message::Open {
                    source_length: length,
                },
            )?;
            #[cfg(feature = "startup-diagnostics")]
            startup.mark("open_sent");
            let mut read_total = 0u64;
            #[cfg(feature = "startup-diagnostics")]
            let mut first_read = true;
            for _ in 0..8192 {
                check_cancel(cancel)?;
                check_cancel(stopped)?;
                match api::read_message(output)? {
                    Message::Read {
                        offset,
                        length: amount,
                    } => {
                        #[cfg(feature = "startup-diagnostics")]
                        if first_read {
                            startup.mark("first_read_request");
                        }
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
                                    midnight_work_budget::storage_admission(
                                        source.confirmed_local_ssd(),
                                    ),
                                    || {
                                        cancel.load(Ordering::Acquire)
                                            || stopped.load(Ordering::Acquire)
                                    },
                                )
                                .ok_or_else(cancelled)?;
                            #[cfg(feature = "startup-diagnostics")]
                            if first_read {
                                startup.mark("first_read_permit");
                            }
                            source.validate()?;
                            let bytes = source.read_at(offset, amount)?;
                            source.validate()?;
                            bytes
                        };
                        if bytes.len() as u64 != end - offset {
                            return Err(io::Error::other("Image source returned a short read"));
                        }
                        #[cfg(feature = "startup-diagnostics")]
                        if first_read {
                            startup.mark("first_read_complete");
                            first_read = false;
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
        #[cfg(feature = "startup-diagnostics")]
        startup.mark("metadata");
        self.metadata = metadata.0;
        self.document = metadata.1;
        self.validate_source()?;
        #[cfg(feature = "startup-diagnostics")]
        startup.mark("complete");
        check_cancel(cancel)?;
        Ok(self)
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
            self.transport.kill();
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
            self.transport.kill();
            return result.and(Err(io::Error::other("Image session already ended")));
        }
        result
    }

    /// Terminate and reap on the owning backend worker. UI close handlers only signal cancellation.
    pub fn close(mut self) {
        self.transport.kill();
    }

    fn validate_source(&self) -> io::Result<()> {
        self.source.source.validate()?;
        if self.source.source.length() != self.source.length
            || self.source.source.version() != self.source.version
        {
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
        let source = self.source.source.as_ref();
        let result = self
            .transport
            .exchange(cancel, timeout, |input, output, stopped| {
                operation(input, output, source, stopped)
            });
        if result.is_err() {
            self.failed = true;
            self.transport.kill();
        }
        result
    }
}

struct CapturedSource {
    source: Arc<dyn Source>,
    length: u64,
    version: Vec<u8>,
}

impl CapturedSource {
    fn capture(source: Arc<dyn Source>) -> io::Result<Self> {
        source.validate()?;
        let length = source.length();
        let version = source.version();
        Ok(Self {
            source,
            length,
            version,
        })
    }
}
fn rejected() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "Image decoder rejected the source",
    )
}
