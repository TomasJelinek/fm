//! Backend-only Office service adapter. Generated HTML never inherits directory authority.
use midnight_core::Cancellation;
use midnight_office_preview::{
    ImmutableHtml, OfficeFormat, OfficeSession, OfficeSource, OfficeTarget,
};
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use midnight_web_viewer_host::{ResourceCancellation, ResourcePathPolicy, Resources};
use std::{
    fmt, io,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

const SOURCE_READ: usize = 64 * 1024;
const HTML_BYTES: usize = 1024 * 1024;
const LIVE_WINDOWS: usize = 4;
const RESOURCE_READ: u32 = 256 * 1024;

pub(crate) struct Prepared {
    pub main: CapturedFile,
    pub resources: Arc<dyn Resources>,
}

/// Known suffixes retain explicit unsupported-format errors from the service.
/// A user-supplied association may give an XLSX package a different suffix; the
/// initial XLSX service still validates that package rather than trusting its name.
pub(crate) fn format(path: &Path) -> OfficeFormat {
    match path
        .extension()
        .and_then(|suffix| suffix.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "docx" => OfficeFormat::Docx,
        "pptx" => OfficeFormat::Pptx,
        "odt" => OfficeFormat::Odt,
        "ods" => OfficeFormat::Ods,
        "odp" => OfficeFormat::Odp,
        _ => OfficeFormat::Xlsx,
    }
}

/// Opens/generates on the viewer backend, before starting the browser actor.
pub(crate) fn prepare(
    file: CapturedFile,
    format: OfficeFormat,
    lifetime: Cancellation,
) -> Result<Prepared, String> {
    let mut parent = Retained::new(Arc::new(Parent::new(file, lifetime.clone())));
    let opened = (|| {
        parent.get().validate(&lifetime).map_err(text)?;
        let source = Arc::new(CapturedReader(parent.get().clone()));
        let opened = OfficeSession::open(source, format, &lifetime).map_err(text);
        parent.get().validate(&lifetime).map_err(text)?;
        opened
    })();
    match opened {
        Ok(session) => prepare_owned(parent.take(), Box::new(session), &lifetime),
        Err(error) => Err(with_cleanup(
            error,
            parent.take().release_on_worker().map_err(text),
        )),
    }
}

// This small seam controls external generation in focused boundary tests. It is
// not a public renderer API, cache framework, or a source of filesystem authority.
trait ExternalGeneration: Send {
    fn initial_target(&self) -> OfficeTarget;
    fn parse_target(&self, path: &str) -> Result<OfficeTarget, String>;
    fn render(
        &mut self,
        target: OfficeTarget,
        cancel: &Cancellation,
    ) -> Result<ImmutableHtml, String>;
}

impl ExternalGeneration for OfficeSession {
    fn initial_target(&self) -> OfficeTarget {
        OfficeSession::initial_target(self)
    }
    fn parse_target(&self, path: &str) -> Result<OfficeTarget, String> {
        OfficeSession::parse_target(self, path).map_err(text)
    }
    fn render(
        &mut self,
        target: OfficeTarget,
        cancel: &Cancellation,
    ) -> Result<ImmutableHtml, String> {
        OfficeSession::render(self, target, cancel).map_err(text)
    }
}

#[cfg(test)]
fn prepare_external(
    file: CapturedFile,
    lifetime: Cancellation,
    generation: Box<dyn ExternalGeneration>,
) -> Result<Prepared, String> {
    prepare_owned(
        Arc::new(Parent::new(file, lifetime.clone())),
        generation,
        &lifetime,
    )
}

fn prepare_owned(
    parent: Arc<Parent>,
    generation: Box<dyn ExternalGeneration>,
    lifetime: &Cancellation,
) -> Result<Prepared, String> {
    let mut scope = Retained::new(Arc::new(Scope {
        parent: Some(parent),
        generation: Mutex::new(Some(Generation {
            service: generation,
            last: 0,
        })),
        credits: Arc::default(),
    }));
    let opened = (|| {
        let initial = {
            let generation = scope
                .get()
                .generation
                .lock()
                .map_err(|_| "Office initial target lock failed")?;
            generation
                .as_ref()
                .ok_or("Office generation has been released")?
                .service
                .initial_target()
        };
        scope.get().generate(&initial.path(), lifetime)
    })();
    match opened {
        Ok(main) => Ok(Prepared {
            main,
            resources: scope.take(),
        }),
        Err(error) => Err(with_cleanup(error, scope.take().release_on_worker())),
    }
}

struct Parent {
    file: Mutex<Option<CapturedFile>>,
    length: u64,
    revision: u64,
    lifetime: Cancellation,
    cleanup_unconfirmed: AtomicBool,
}

impl Parent {
    fn new(file: CapturedFile, lifetime: Cancellation) -> Self {
        Self {
            length: file.length(),
            revision: file.revision(),
            cleanup_unconfirmed: AtomicBool::new(file.cleanup_unconfirmed()),
            file: Mutex::new(Some(file)),
            lifetime,
        }
    }

    fn check(&self, cancellation: &Cancellation) -> Result<(), Error> {
        if self.lifetime.is_cancelled() || cancellation.is_cancelled() {
            return Err(failure("Office preview cancelled or parent revoked"));
        }
        Ok(())
    }

    fn validate(&self, cancellation: &Cancellation) -> Result<(), Error> {
        self.check(cancellation)?;
        let result = self
            .file
            .lock()
            .map_err(|_| failure("Office parent lock failed"))?
            .as_ref()
            .ok_or_else(|| failure("Office parent has been released"))?
            .revalidate();
        if result.is_err() {
            // A later revision rollback cannot revive previously invalidated bytes.
            self.lifetime.cancel();
        }
        result?;
        self.check(cancellation)
    }

    fn read(
        &self,
        offset: u64,
        length: u32,
        cancellation: &Cancellation,
    ) -> Result<Vec<u8>, Error> {
        self.check(cancellation)?;
        let result = self
            .file
            .lock()
            .map_err(|_| failure("Office parent lock failed"))?
            .as_ref()
            .ok_or_else(|| failure("Office parent has been released"))?
            .read_at(offset, length);
        if result.is_err() {
            self.lifetime.cancel();
        }
        let bytes = result?;
        self.check(cancellation)?;
        Ok(bytes)
    }

    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        match Arc::try_unwrap(self) {
            Ok(parent) => {
                let mut parent = Retained::new(parent);
                let file = parent
                    .get_mut()
                    .file
                    .get_mut()
                    .map_err(|_| failure("Office parent cleanup lock failed"))?
                    .take()
                    .ok_or_else(|| failure("Office parent cleanup is already consumed"))?;
                if let Err(error) = file.release_on_worker() {
                    parent
                        .get()
                        .cleanup_unconfirmed
                        .store(true, Ordering::Release);
                    return Err(error);
                }
                drop(parent.take());
                Ok(())
            }
            Err(shared) => {
                drop(shared);
                Ok(())
            }
        }
    }
}

struct CapturedReader(Arc<Parent>);

impl OfficeSource for CapturedReader {
    fn len(&self) -> u64 {
        self.0.length
    }

    fn read_at(&self, offset: u64, output: &mut [u8], cancel: &Cancellation) -> io::Result<usize> {
        if output.len() > SOURCE_READ || offset.checked_add(output.len() as u64).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid Office source read",
            ));
        }
        let count = self
            .0
            .length
            .saturating_sub(offset)
            .min(output.len() as u64) as usize;
        if count == 0 {
            self.0.validate(cancel).map_err(io::Error::other)?;
            return Ok(0);
        }
        // CapturedFile retains the native/provider reader's own admission and
        // validates original identity/revision around this bounded source read.
        let bytes = self
            .0
            .read(offset, count as u32, cancel)
            .map_err(io::Error::other)?;
        if bytes.len() != count {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Short Office source read",
            ));
        }
        output[..count].copy_from_slice(&bytes);
        Ok(count)
    }
}

// Each credit conservatively accounts one full MiB before external generation.
// Four credits therefore bound all live/reserved HTML slices to four MiB, with
// no Vec spare capacity and no retained-window cache. Credits survive an
// unsuccessful last-source release together with its unconfirmed owner.
#[derive(Default)]
struct Credits(AtomicUsize);

impl Credits {
    fn reserve(self: &Arc<Self>) -> Result<Credit, String> {
        self.0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                (used < LIVE_WINDOWS).then(|| used + 1)
            })
            .map_err(|_| "Office preview live HTML budget exhausted".to_owned())?;
        Ok(Credit(self.clone()))
    }
}

struct Credit(Arc<Credits>);

impl Drop for Credit {
    fn drop(&mut self) {
        self.0.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct Generation {
    service: Box<dyn ExternalGeneration>,
    last: u64,
}

struct Scope {
    parent: Option<Arc<Parent>>,
    generation: Mutex<Option<Generation>>,
    credits: Arc<Credits>,
}

impl fmt::Debug for Scope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GeneratedOfficeResources")
    }
}

impl Scope {
    fn generate(&self, path: &str, cancel: &Cancellation) -> Result<CapturedFile, String> {
        let parent = self
            .parent
            .as_ref()
            .ok_or("Office parent has been released")?;
        let cancellation = parent.lifetime.linked(cancel);
        let cancel = &cancellation;
        parent.validate(cancel).map_err(text)?;
        let mut generation = self
            .generation
            .lock()
            .map_err(|_| "Office generation lock failed")?;
        let generation = generation
            .as_mut()
            .ok_or("Office generation has been released")?;
        let target = generation.service.parse_target(path)?;
        if target.path() != path {
            return Err("Office generation accepted a noncanonical target".into());
        }
        let credit = self.credits.reserve()?;
        parent.validate(cancel).map_err(text)?;
        let rendered = generation.service.render(target, cancel);
        parent.validate(cancel).map_err(text)?;
        let rendered = rendered?;
        if rendered.target != target || rendered.generation <= generation.last {
            return Err("Office generation returned inconsistent identity".into());
        }
        if rendered.bytes.is_empty() || rendered.bytes.len() > HTML_BYTES {
            return Err("Office generated HTML size limit exceeded".into());
        }
        generation.last = rendered.generation;
        let mut source = Retained::new(Arc::new(Generated {
            parent: Some(parent.clone()),
            version: identity(parent.revision, target, rendered.generation),
            bytes: rendered.bytes,
            _credit: credit,
        }));
        let capture = CapturedFile::from_source(source.get().clone()).map_err(text);
        let release = source.take().release_on_worker().map_err(text);
        match capture {
            Ok(capture) => {
                if let Err(error) = release {
                    return Err(with_cleanup(
                        error,
                        capture.release_on_worker().map_err(text),
                    ));
                }
                Ok(capture)
            }
            Err(error) => Err(with_cleanup(error, release)),
        }
    }
}

impl Resources for Scope {
    fn capture(&self, segments: &[String]) -> Result<CapturedFile, String> {
        self.generate(&target_path(segments)?, &Cancellation::default())
    }

    fn capture_with_cancellation(
        &self,
        segments: &[String],
        cancellation: &ResourceCancellation,
    ) -> Result<CapturedFile, String> {
        let cancellation = Cancellation::observing(cancellation.flag());
        self.generate(&target_path(segments)?, &cancellation)
    }

    fn path_policy(&self) -> ResourcePathPolicy {
        ResourcePathPolicy::CanonicalRoot
    }

    fn release_on_worker(self: Arc<Self>) -> Result<(), String> {
        match Arc::try_unwrap(self) {
            Ok(scope) => {
                let mut scope = Retained::new(scope);
                // Drop OfficeSession while the separately retained Parent still
                // pins its captured reader. The reader's Arc can never be last.
                let generation = scope
                    .get_mut()
                    .generation
                    .get_mut()
                    .map_err(|_| "Office generation cleanup lock failed")?
                    .take();
                drop(generation);
                let parent = scope
                    .get_mut()
                    .parent
                    .take()
                    .ok_or("Office parent cleanup consumed")?;
                parent.release_on_worker().map_err(text)?;
                drop(scope.take());
                Ok(())
            }
            Err(shared) => {
                drop(shared);
                Ok(())
            }
        }
    }
}

fn target_path(segments: &[String]) -> Result<String, String> {
    // The service's longest current target has nine path segments, <=100 bytes.
    // Validate segments before joining: a slash hidden inside one component must
    // not manufacture a canonical service URL for a direct Resources caller.
    if segments.is_empty()
        || segments.len() > 9
        || segments.iter().any(|segment| {
            segment.is_empty()
                || segment.len() > 100
                || segment == "."
                || segment == ".."
                || segment.bytes().any(|byte| {
                    !byte.is_ascii_alphanumeric() && !matches!(byte, b'.' | b'-' | b'_')
                })
        })
        || segments.iter().map(String::len).sum::<usize>() + segments.len() > 100
    {
        return Err("Invalid Office resource target".into());
    }
    Ok(format!("/{}", segments.join("/")))
}

fn identity(parent: u64, target: OfficeTarget, generation: u64) -> Vec<u8> {
    let mut version = b"midnight-office-html-v1\0".to_vec();
    version.extend_from_slice(&parent.to_le_bytes());
    version.extend_from_slice(&generation.to_le_bytes());
    version.extend_from_slice(target.path().as_bytes());
    version
}

struct Generated {
    parent: Option<Arc<Parent>>,
    bytes: Arc<[u8]>,
    version: Vec<u8>,
    _credit: Credit,
}

impl fmt::Debug for Generated {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ImmutableOfficeHtml")
    }
}

impl Generated {
    fn parent(&self) -> Result<&Parent, Error> {
        self.parent
            .as_deref()
            .ok_or_else(|| failure("Office parent has been released"))
    }
}

impl ContentSource for Generated {
    fn length(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn display_name(&self) -> String {
        "office-preview.html".into()
    }
    fn version(&self) -> Vec<u8> {
        self.version.clone()
    }
    fn validate(&self) -> Result<(), Error> {
        self.parent()?.validate(&Cancellation::default())
    }
    fn read_checked(
        &self,
        offset: u64,
        length: u32,
        expected_length: u64,
        expected_version: &[u8],
    ) -> Result<Vec<u8>, Error> {
        if expected_length != self.length() || expected_version != self.version {
            return Err(failure("Office generated content identity changed"));
        }
        self.read_at(offset, length)
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        if length == 0 || length > RESOURCE_READ || offset.checked_add(u64::from(length)).is_none()
        {
            return Err(failure("Invalid Office generated content read"));
        }
        self.validate()?;
        let first = offset.min(self.length()) as usize;
        let last = (offset.saturating_add(u64::from(length))).min(self.length()) as usize;
        let bytes = self.bytes[first..last].to_vec();
        self.validate()?;
        Ok(bytes)
    }
    fn cleanup_unconfirmed(&self) -> bool {
        self.parent
            .as_ref()
            .is_some_and(|parent| parent.cleanup_unconfirmed.load(Ordering::Acquire))
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        match Arc::try_unwrap(self) {
            Ok(source) => {
                let mut source = Retained::new(source);
                let parent = source
                    .get_mut()
                    .parent
                    .take()
                    .ok_or_else(|| failure("Office generated parent cleanup consumed"))?;
                parent.release_on_worker()?;
                // Bytes/credit are retired only after the acquired cleanup is confirmed.
                drop(source.take());
                Ok(())
            }
            Err(shared) => {
                drop(shared);
                Ok(())
            }
        }
    }
}

// A consuming child release owns any value already taken. On error/unwind this
// guard keeps all not-yet-released ownership and accounting rather than running
// an unqualified destructor. All explicit completion paths consume the guard.
struct Retained<T>(Option<T>);

impl<T> Retained<T> {
    fn new(value: T) -> Self {
        Self(Some(value))
    }
    fn get(&self) -> &T {
        self.0
            .as_ref()
            .expect("retained Office ownership remains present")
    }
    fn get_mut(&mut self) -> &mut T {
        self.0
            .as_mut()
            .expect("retained Office ownership remains present")
    }
    fn take(&mut self) -> T {
        self.0
            .take()
            .expect("retained Office ownership consumed once")
    }
}

impl<T> Drop for Retained<T> {
    fn drop(&mut self) {
        if let Some(owner) = self.0.take() {
            std::mem::forget(owner);
        }
    }
}

fn failure(message: &str) -> Error {
    Error(message.into())
}
fn text(error: impl fmt::Display) -> String {
    error.to_string()
}
fn with_cleanup(error: String, cleanup: Result<(), String>) -> String {
    match cleanup {
        Ok(()) => error,
        Err(cleanup) => format!("{error}; {cleanup}"),
    }
}

#[cfg(test)]
#[path = "office_preview_tests.rs"]
mod tests;

#[cfg(all(test, target_os = "windows"))]
#[path = "office_preview_native_tests.rs"]
mod native_tests;
