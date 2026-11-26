//! Killable persistent worker and bounded read-only broker. All entry points run on backend workers.
use crate::{
    DocumentMetadata, Frame, Metadata, PreparedImageWorker, Provider, Viewport,
    startup::{ProviderIdentity, WorkerOwner, WorkerTransport, attempt, cancelled, check_cancel},
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

/// Longest wait after `Failure` for the worker's reason or the end of its stream.
const REASON_GRACE: Duration = Duration::from_secs(1);
/// Source bytes one Open may read, 1 GiB (owner decision of 26 November 2026; was 256 MiB).
const MAX_OPEN_READ_BYTES: u64 = 1024 * 1024 * 1024;
/// Read requests one Open may make, scaled with `MAX_OPEN_READ_BYTES` (was 8192).
const MAX_OPEN_READS: usize = 32_768;

/// One persistent restricted decoder. Creation, rendering, close and Drop are backend-only.
/// Memory is capped at 1 GiB, retained RGBA at 128 MiB (the published API's
/// `MAX_DECODED_BYTES`); cumulative process CPU is 60 seconds. Open reads at most 1 GiB of
/// source in at most 32,768 requests.
/// Each operation additionally has the supplied wall-clock deadline and cancellation flag.
pub struct ImageSession {
    owner: WorkerOwner<CapturedSource>,
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
        Self::open_owned(provider, source, cancel, timeout, Box::new(()))
    }

    /// Retain the caller's native-child reservation before any possible process creation.
    pub fn open_owned(
        provider: &Provider,
        source: Arc<dyn Source>,
        cancel: &AtomicBool,
        timeout: Duration,
        owner: Box<dyn Send + 'static>,
    ) -> io::Result<Self> {
        let mut owned = WorkerOwner::empty(Some(owner)).with_source(CapturedSource::new(source));
        #[cfg(feature = "startup-diagnostics")]
        let mut startup = midnight_worker_platform::startup_diagnostics::Trace::new("image_open");
        let result = attempt(|| {
            check_cancel(cancel)?;
            owned.install_identity(ProviderIdentity::capture(provider)?);
            #[cfg(feature = "startup-diagnostics")]
            startup.mark("provider_validated");
            owned.source_mut().capture()?;
            #[cfg(feature = "startup-diagnostics")]
            startup.mark("source_validated");
            owned.start(|| WorkerTransport::spawn(provider))?;
            #[cfg(feature = "startup-diagnostics")]
            startup.mark("spawned");
            Ok(())
        });
        if let Err(error) = result {
            return Err(owned.fail(error));
        }
        Self::new(owned).open_source(
            provider,
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
        // Attach the source before cancellation or another fallible boundary can retire the child.
        let mut owned = prepared
            .into_owner()
            .with_source(CapturedSource::new(source));
        #[cfg(feature = "startup-diagnostics")]
        let mut startup =
            midnight_worker_platform::startup_diagnostics::Trace::new("image_open_prepared");
        let result = attempt(|| {
            check_cancel(cancel)?;
            if !owned.identity().catalog_matches(provider) {
                return Err(io::Error::other("Prepared image worker catalog changed"));
            }
            #[cfg(feature = "startup-diagnostics")]
            startup.mark("catalog_matched");
            owned.source_mut().capture()?;
            #[cfg(feature = "startup-diagnostics")]
            startup.mark("source_validated");
            check_cancel(cancel)?;
            Ok(())
        });
        if let Err(error) = result {
            return Err(owned.fail(error));
        }
        Self::new(owned).open_source(
            provider,
            cancel,
            timeout,
            false,
            #[cfg(feature = "startup-diagnostics")]
            &mut startup,
        )
    }

    fn new(owner: WorkerOwner<CapturedSource>) -> Self {
        Self {
            owner,
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
        cancel: &AtomicBool,
        timeout: Duration,
        read_ready: bool,
        #[cfg(feature = "startup-diagnostics")]
        startup: &mut midnight_worker_platform::startup_diagnostics::Trace,
    ) -> io::Result<Self> {
        let result = attempt(|| {
            let length = self.owner.source().length;
            let metadata = self.exchange(
                cancel,
                timeout,
                |input, output, source, identity, stopped| {
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
                    for _ in 0..MAX_OPEN_READS {
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
                                        io::Error::other(
                                            "Image worker read outside captured source",
                                        )
                                    })?;
                                read_total += u64::from(amount);
                                if read_total > MAX_OPEN_READ_BYTES {
                                    return Err(io::Error::other(
                                        "Image source-read budget exhausted",
                                    ));
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
                                    return Err(io::Error::other(
                                        "Image source returned a short read",
                                    ));
                                }
                                #[cfg(feature = "startup-diagnostics")]
                                if first_read {
                                    startup.mark("first_read_complete");
                                    first_read = false;
                                }
                                api::write_message(input, &Message::Data(bytes))?;
                            }
                            Message::Metadata(metadata) if provider.protocol == 1 => {
                                return Ok(Some((metadata, None)));
                            }
                            Message::DocumentMetadata(document) if provider.protocol == 2 => {
                                document.validate()?;
                                return Ok(Some((document.pages[0], Some(document))));
                            }
                            Message::Failure => return Ok(None),
                            _ => return Err(io::Error::other("Unexpected image-open response")),
                        }
                    }
                    Err(io::Error::other(
                        "Image source request-count budget exhausted",
                    ))
                },
            )?;
            let Some(metadata) = metadata else {
                return Err(rejected(
                    provider.protocol == 2,
                    self.failure_reason(cancel).as_ref(),
                ));
            };
            #[cfg(feature = "startup-diagnostics")]
            startup.mark("metadata");
            self.metadata = metadata.0;
            self.document = metadata.1;
            self.validate_source()?;
            #[cfg(feature = "startup-diagnostics")]
            startup.mark("complete");
            check_cancel(cancel)?;
            Ok(())
        });
        match result {
            Ok(()) => Ok(self),
            Err(error) => Err(self.owner.fail(error)),
        }
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
            let document = self.document.is_some();
            let message = if document {
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
            let frame = self.exchange(cancel, timeout, |input, output, _, _, _| {
                api::write_message(input, &message)?;
                match crate::raster_reply::read(output, viewport.width, viewport.height)? {
                    Message::Rgba { id, frame }
                        if id == request_id
                            && frame.width == viewport.width
                            && frame.height == viewport.height =>
                    {
                        Ok(Some(frame))
                    }
                    Message::Failure => Ok(None),
                    _ => Err(io::Error::other("Unexpected image-viewport response")),
                }
            })?;
            let Some(frame) = frame else {
                return Err(not_drawn(document, self.failure_reason(cancel).as_ref()));
            };
            self.validate_source()?;
            check_cancel(cancel)?;
            Ok(frame)
        })();
        if result.is_err() {
            self.failed = true;
            self.owner.transport_mut().kill();
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
            self.owner.transport_mut().kill();
            return result.and(Err(io::Error::other("Image session already ended")));
        }
        result
    }

    /// Terminate and reap on the owning backend worker; UI close handlers only signal
    /// cancellation. Positively confirm process termination and pin release; inspect errors
    /// (`cleanup_status`) before formatting.
    pub fn close_checked(mut self) -> io::Result<()> {
        self.owner.finish()
    }

    /// After `Failure`: end the session and read the reason that may follow it before
    /// stopping the worker. Workers end their stream after `Failure` (SDK 0.4.0 after the
    /// reason, older ones at once), so this normally returns within milliseconds; a
    /// worker that does neither is stopped after `REASON_GRACE` or on cancellation, and
    /// the failure keeps its plain text.
    fn failure_reason(&mut self, cancel: &AtomicBool) -> Option<api::FailureReason> {
        self.failed = true;
        let transport = self.owner.transport_mut();
        let reason = transport
            .exchange(cancel, REASON_GRACE, |_, output, _| {
                Ok(crate::raster_reply::failure_reason(output))
            })
            .ok()
            .flatten();
        transport.kill();
        reason
    }

    fn validate_source(&self) -> io::Result<()> {
        let source = self.owner.source();
        source.source.validate()?;
        if source.source.length() != source.length || source.source.version() != source.version {
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
            &ProviderIdentity,
            &AtomicBool,
        ) -> io::Result<T>
        + Send,
    ) -> io::Result<T> {
        let (transport, identity, source) = self.owner.parts();
        let source = source.expect("live captured source").source.as_ref();
        let result = transport.exchange(cancel, timeout, |input, output, stopped| {
            operation(input, output, source, identity, stopped)
        });
        if result.is_err() {
            self.failed = true;
            self.owner.transport_mut().kill();
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
    fn new(source: Arc<dyn Source>) -> Self {
        Self {
            source,
            length: 0,
            version: Vec::new(),
        }
    }

    fn capture(&mut self) -> io::Result<()> {
        self.source.validate()?;
        self.length = self.source.length();
        self.version = self.source.version();
        Ok(())
    }
}

/// The decoder refused the source: what a refusal means in words a user can act on, then
/// the decoder's own sanitized reason when the worker sent one (SDK 0.4.0 and later).
///
/// API 0.3.0 carries a reason's text but not its kind, so a limit or unsupported content
/// must not be called damage (VIEW-06 adds the kind). Until then the text decides: a reason
/// that is a complete sentence about the content ("This SVG is larger than 4 MiB, ...")
/// states why it is not shown and is the whole message; any other reason, such as a codec
/// error, continues the "damaged or incomplete" sentence.
fn rejected(document: bool, reason: Option<&api::FailureReason>) -> io::Error {
    let kind = if document { "document" } else { "image" };
    io::Error::new(
        io::ErrorKind::Unsupported,
        match reason {
            Some(reason) if statement(reason) => sentence(reason),
            Some(reason) => format!(
                "This {kind} is damaged or incomplete: {}",
                continuation(reason)
            ),
            None => {
                format!("This {kind} is damaged or incomplete, or its format is not supported.")
            }
        },
    )
}

/// A provider's complete sentence about the content: it starts with "This ".
fn statement(reason: &api::FailureReason) -> bool {
    reason.as_str().starts_with("This ")
}

/// The statement as the whole status text, ending with a full stop.
fn sentence(reason: &api::FailureReason) -> String {
    let text = reason.as_str().trim_end();
    if text.ends_with(['.', '!', '?']) {
        text.to_owned()
    } else {
        format!("{text}.")
    }
}

/// The decoder could not draw a view of a source it accepted. Images are decoded whole at
/// open, so only document pages, decoded on demand, can still turn out damaged here.
fn not_drawn(document: bool, reason: Option<&api::FailureReason>) -> io::Error {
    io::Error::other(match (document, reason) {
        (true, Some(reason)) => format!("This page could not be drawn: {}", continuation(reason)),
        (true, None) => "This page could not be drawn; the document may be damaged.".into(),
        (false, Some(reason)) => {
            format!("This image could not be drawn: {}", continuation(reason))
        }
        (false, None) => "This image could not be drawn.".into(),
    })
}

/// The reason continues the host's sentence after its colon, so a capitalized first
/// word is lowercased ("The file is empty"); an acronym such as "PDF" stays.
fn continuation(reason: &api::FailureReason) -> String {
    let mut text = reason.as_str().to_owned();
    let mut bytes = text.bytes();
    if let (Some(first), Some(second)) = (bytes.next(), bytes.next())
        && first.is_ascii_uppercase()
        && second.is_ascii_lowercase()
    {
        // Reasons are ASCII, so the first byte is a whole character.
        text[..1].make_ascii_lowercase();
    }
    text
}

#[cfg(test)]
mod failure_text_tests {
    use super::*;

    #[test]
    fn the_reason_is_the_second_part_of_the_failure_text() {
        let reason = api::FailureReason::new("The file is empty").unwrap();
        for (error, text) in [
            (
                rejected(false, Some(&reason)),
                "This image is damaged or incomplete: the file is empty",
            ),
            (
                rejected(true, Some(&reason)),
                "This document is damaged or incomplete: the file is empty",
            ),
            (
                rejected(false, None),
                "This image is damaged or incomplete, or its format is not supported.",
            ),
            (
                rejected(true, None),
                "This document is damaged or incomplete, or its format is not supported.",
            ),
        ] {
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
            assert_eq!(error.to_string(), text);
        }
        assert_eq!(
            not_drawn(true, Some(&reason)).to_string(),
            "This page could not be drawn: the file is empty"
        );
        assert_eq!(
            not_drawn(true, None).to_string(),
            "This page could not be drawn; the document may be damaged."
        );
        assert_eq!(
            not_drawn(false, Some(&reason)).to_string(),
            "This image could not be drawn: the file is empty"
        );
        assert_eq!(
            not_drawn(false, None).to_string(),
            "This image could not be drawn."
        );
        for (given, shown) in [
            ("PDF page is invalid", "PDF page is invalid"),
            ("unexpected end of file", "unexpected end of file"),
            ("X", "X"),
            ("Png: bad", "png: bad"),
        ] {
            let reason = api::FailureReason::new(given).unwrap();
            assert_eq!(continuation(&reason), shown);
        }
    }

    #[test]
    fn a_limit_or_unsupported_content_is_not_called_damage() {
        // The SVG decoder states limits and unsupported constructs as sentences; damage
        // stays a fragment after the host's own sentence.
        for (given, shown) in [
            (
                "This SVG is larger than 4 MiB, the limit for SVG images.",
                "This SVG is larger than 4 MiB, the limit for SVG images.",
            ),
            (
                "This SVG declares XML entities (a DOCTYPE internal subset), which are not \
                 supported.",
                "This SVG declares XML entities (a DOCTYPE internal subset), which are not \
                 supported.",
            ),
            (
                "This image is 40000 x 20 pixels; the limit is 32768 pixels per side",
                "This image is 40000 x 20 pixels; the limit is 32768 pixels per side.",
            ),
            (
                "The SVG is not well-formed XML: the document ends too early (line 1, column 75)",
                "This image is damaged or incomplete: the SVG is not well-formed XML: the \
                 document ends too early (line 1, column 75)",
            ),
            (
                "Thistle is not a sentence about the content",
                "This image is damaged or incomplete: thistle is not a sentence about the \
                 content",
            ),
        ] {
            let reason = api::FailureReason::new(given).unwrap();
            let error = rejected(false, Some(&reason));
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
            assert_eq!(error.to_string(), shown);
            assert!(!shown.starts_with("This SVG") || !shown.contains("damaged"));
        }
        let reason = api::FailureReason::new("This document needs a password").unwrap();
        assert_eq!(
            rejected(true, Some(&reason)).to_string(),
            "This document needs a password."
        );
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;

    /// Owner decision of 26 November 2026: the image workers' limit is 1 GiB.
    #[test]
    fn image_workers_have_one_gib_of_memory_and_of_source_reads() {
        let limits = crate::startup::IMAGE_LIMITS;
        assert_eq!(limits.memory_bytes, 1 << 30);
        assert!(limits.validate().is_ok(), "the worker SDK grants it");
        assert_eq!(MAX_OPEN_READ_BYTES, 1 << 30);
        // As before, the requests reach the byte budget at an average of 32 KiB each.
        assert_eq!(MAX_OPEN_READ_BYTES / MAX_OPEN_READS as u64, 32 * 1024);
        // The decoded RGBA bound stays the published API's.
        assert_eq!(api::MAX_DECODED_BYTES, 128 << 20);
    }
}
