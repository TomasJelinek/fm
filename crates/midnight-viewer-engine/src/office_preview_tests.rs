//! Captured-parent, generated-resource and checked-release boundaries without Chromium.
#[path = "office_link_tests.rs"]
mod links;

use super::*;
use midnight_office_preview::{DocumentId, ImmutableHtml, OfficeTarget, SheetId, SlideId};
use midnight_plugin_host::{ContentResources, ContentSource, Error};
use midnight_web_viewer_host::{ResourceCancellation, ResourcePathPolicy};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

const MIB: usize = 1024 * 1024;

#[derive(Debug, Default)]
struct SourceState {
    version: AtomicU64,
    version_bytes: AtomicUsize,
    validations: AtomicUsize,
    reads: AtomicUsize,
    mutate_on_read: AtomicBool,
    native_resource_captures: AtomicUsize,
    source_releases: AtomicUsize,
    resource_releases: AtomicUsize,
    source_drops: AtomicUsize,
    resource_drops: AtomicUsize,
    fail_release: AtomicBool,
    panic_release: AtomicBool,
    events: Mutex<Vec<&'static str>>,
}

#[derive(Debug)]
struct OriginalSource(Arc<SourceState>);

impl ContentSource for OriginalSource {
    fn length(&self) -> u64 {
        8
    }
    fn display_name(&self) -> String {
        "Přehled-日本語-🦊.xlsx".into()
    }
    fn version(&self) -> Vec<u8> {
        let mut version = self
            .0
            .version
            .load(Ordering::Acquire)
            .to_le_bytes()
            .to_vec();
        version.resize(
            version
                .len()
                .max(self.0.version_bytes.load(Ordering::Acquire)),
            0,
        );
        version
    }
    fn validate(&self) -> Result<(), Error> {
        self.0.validations.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.0.reads.fetch_add(1, Ordering::AcqRel);
        let first = usize::try_from(offset).map_err(|_| Error("offset".into()))?;
        let last = first.saturating_add(length as usize).min(8);
        if self.0.mutate_on_read.swap(false, Ordering::AcqRel) {
            self.0.version.fetch_add(1, Ordering::AcqRel);
        }
        Ok(b"ORIGINAL".get(first..last).unwrap_or(&[]).to_vec())
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        match Arc::try_unwrap(self) {
            Ok(source) => {
                source.0.source_releases.fetch_add(1, Ordering::AcqRel);
                source.0.events.lock().unwrap().push("source");
                if source.0.panic_release.load(Ordering::Acquire) {
                    std::mem::forget(source);
                    panic!("controlled source-release panic");
                }
                if source.0.fail_release.load(Ordering::Acquire) {
                    std::mem::forget(source);
                    return Err(Error("controlled source-release failure".into()));
                }
                Ok(())
            }
            Err(shared) => {
                drop(shared);
                Ok(())
            }
        }
    }
}

impl Drop for OriginalSource {
    fn drop(&mut self) {
        self.0.source_drops.fetch_add(1, Ordering::AcqRel);
    }
}

#[derive(Debug)]
struct OriginalResources(Arc<SourceState>);

impl Drop for OriginalResources {
    fn drop(&mut self) {
        self.0.resource_drops.fetch_add(1, Ordering::AcqRel);
    }
}

impl ContentResources for OriginalResources {
    fn capture(&self, _: &[String]) -> Result<CapturedFile, Error> {
        self.0
            .native_resource_captures
            .fetch_add(1, Ordering::AcqRel);
        Err(Error(
            "original directory authority must not be reached".into(),
        ))
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        match Arc::try_unwrap(self) {
            Ok(resources) => {
                resources.0.resource_releases.fetch_add(1, Ordering::AcqRel);
                resources.0.events.lock().unwrap().push("resources");
                Ok(())
            }
            Err(shared) => {
                drop(shared);
                Ok(())
            }
        }
    }
}

type RenderAction = Box<dyn FnOnce() + Send>;

struct GenerationState {
    calls: AtomicUsize,
    initial_calls: AtomicUsize,
    initial: Mutex<OfficeTarget>,
    rendered: Mutex<Vec<OfficeTarget>>,
    render_error: Mutex<Option<PreviewError>>,
    sequence: AtomicU64,
    size: AtomicUsize,
    action: Mutex<Option<RenderAction>>,
    returned_target: Mutex<Option<OfficeTarget>>,
    returned_sequence: Mutex<Option<u64>>,
    paths: Mutex<Vec<String>>,
    observed_cancellation: AtomicBool,
    source: Arc<SourceState>,
}

struct ControlledGeneration(Arc<GenerationState>);

impl ExternalGeneration for ControlledGeneration {
    fn initial_target(&self) -> OfficeTarget {
        self.0.initial_calls.fetch_add(1, Ordering::AcqRel);
        *self.0.initial.lock().unwrap()
    }
    fn parse_target(&self, path: &str) -> Result<OfficeTarget, String> {
        self.0.paths.lock().unwrap().push(path.into());
        // Test membership is exactly two sheets and three explicit canonical
        // targets. Production grammar/bounds belongs to OfficeSession tests.
        let initial = *self.0.initial.lock().unwrap();
        if path == initial.path() {
            return Ok(initial);
        }
        match path {
            "/office/v1/catalog.html" => Ok(OfficeTarget::Catalog),
            "/office/v1/sheet/0/first.html" => Ok(OfficeTarget::SheetFirst { sheet: SheetId(0) }),
            "/office/v1/sheet/1/first.html" => Ok(OfficeTarget::SheetFirst { sheet: SheetId(1) }),
            "/office/v1/sheet/1/row/1048576/col/16384.html" => Ok(OfficeTarget::SheetWindow {
                sheet: SheetId(1),
                row: 1_048_576,
                column: 16_384,
            }),
            _ => Err("controlled target rejection".into()),
        }
    }
    fn render(
        &mut self,
        target: OfficeTarget,
        cancel: &Cancellation,
    ) -> Result<ImmutableHtml, PreviewError> {
        if cancel.is_cancelled() {
            return Err("generation cancelled before work".into());
        }
        self.0.calls.fetch_add(1, Ordering::AcqRel);
        self.0.rendered.lock().unwrap().push(target);
        if let Some(error) = self.0.render_error.lock().unwrap().take() {
            return Err(error);
        }
        if let Some(action) = self.0.action.lock().unwrap().take() {
            action();
        }
        self.0
            .observed_cancellation
            .store(cancel.is_cancelled(), Ordering::Release);
        let sequence = self.0.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        // Deliberately return even if the action invalidated/cancelled the
        // parent. The adapter must fence a late successful generation itself.
        Ok(ImmutableHtml {
            bytes: Arc::from(vec![b'H'; self.0.size.load(Ordering::Acquire)]),
            target: self
                .0
                .returned_target
                .lock()
                .unwrap()
                .take()
                .unwrap_or(target),
            generation: self
                .0
                .returned_sequence
                .lock()
                .unwrap()
                .take()
                .unwrap_or(sequence),
        })
    }
}

impl Drop for ControlledGeneration {
    fn drop(&mut self) {
        self.0.source.events.lock().unwrap().push("generation");
    }
}

struct Fixture {
    source: Arc<SourceState>,
    generation: Arc<GenerationState>,
}

impl Fixture {
    fn new() -> Self {
        let source = Arc::new(SourceState::default());
        let generation = Arc::new(GenerationState {
            calls: AtomicUsize::new(0),
            initial_calls: AtomicUsize::new(0),
            initial: Mutex::new(OfficeTarget::Catalog),
            rendered: Mutex::new(Vec::new()),
            render_error: Mutex::new(None),
            sequence: AtomicU64::new(0),
            size: AtomicUsize::new(32),
            action: Mutex::new(None),
            returned_target: Mutex::new(None),
            returned_sequence: Mutex::new(None),
            paths: Mutex::new(Vec::new()),
            observed_cancellation: AtomicBool::new(false),
            source: source.clone(),
        });
        Self { source, generation }
    }
    fn capture(&self) -> CapturedFile {
        CapturedFile::from_source(Arc::new(OriginalSource(self.source.clone())))
            .unwrap()
            .with_resources(Arc::new(OriginalResources(self.source.clone())))
    }
    fn prepare(&self) -> Prepared {
        self.prepare_with(Cancellation::default()).unwrap()
    }
    fn prepare_with(&self, lifetime: Cancellation) -> Result<Prepared, PreviewError> {
        prepare_external(
            self.capture(),
            lifetime,
            Box::new(ControlledGeneration(self.generation.clone())),
        )
    }
}

fn segments(path: &str) -> Vec<String> {
    path.strip_prefix('/')
        .unwrap_or(path)
        .split('/')
        .map(str::to_owned)
        .collect()
}

fn first(prepared: &Prepared, sheet: u16) -> CapturedFile {
    prepared
        .resources
        .capture(&segments(&format!("/office/v1/sheet/{sheet}/first.html")))
        .unwrap()
}

fn release(prepared: Prepared) {
    prepared.main.release_on_worker().unwrap();
    prepared.resources.release_on_worker().unwrap();
}

#[test]
fn office_generated_capture_has_own_bytes_and_no_original_directory_resources() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    assert!(prepared.main.path().as_os_str().is_empty());
    assert!(prepared.main.resources().is_none());
    assert_eq!(
        prepared.resources.path_policy(),
        ResourcePathPolicy::CanonicalRoot
    );
    assert_eq!(prepared.main.read_at(0, 32).unwrap(), vec![b'H'; 32]);
    assert_ne!(prepared.main.read_at(0, 8).unwrap(), b"ORIGINAL");
    assert!(prepared.resources.capture(&["private.txt".into()]).is_err());
    assert_eq!(
        fixture
            .source
            .native_resource_captures
            .load(Ordering::Acquire),
        0
    );
    assert_eq!(fixture.source.reads.load(Ordering::Acquire), 0);
    release(prepared);
}

#[test]
fn office_derived_identity_changes_for_parent_target_and_successful_generation() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    let first_sheet = first(&prepared, 0);
    let second_sheet = first(&prepared, 1);
    let another_first = first(&prepared, 0);
    let versions = [&prepared.main, &first_sheet, &second_sheet, &another_first]
        .map(CapturedFile::stable_version);
    for (index, version) in versions.iter().enumerate() {
        assert!(!version.is_empty());
        assert!(version.len() <= 4096);
        assert!(versions[index + 1..].iter().all(|other| version != other));
    }
    first_sheet.release_on_worker().unwrap();
    second_sheet.release_on_worker().unwrap();
    another_first.release_on_worker().unwrap();
    fixture.generation.sequence.store(0, Ordering::Release);
    let different_parent = fixture.prepare();
    assert_ne!(
        prepared.main.stable_version(),
        different_parent.main.stable_version()
    );
    release(prepared);
    release(different_parent);
}

#[test]
fn office_parent_maximum_provider_revision_does_not_overflow_derived_identity() {
    let fixture = Fixture::new();
    fixture.source.version_bytes.store(4096, Ordering::Release);
    let prepared = fixture.prepare();
    assert!(prepared.main.stable_version().len() <= 4096);
    assert_eq!(prepared.main.read_at(0, 8).unwrap(), b"HHHHHHHH");
    release(prepared);
}

#[test]
fn office_scope_rejects_noncanonical_or_foreign_targets_without_original_resources() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    let calls = fixture.generation.calls.load(Ordering::Acquire);
    for path in [
        "../private.txt",
        "office/v1/../catalog.html",
        "office/v1//catalog.html",
        "office/v1/catalog.html?x=1",
        "office/v1/catalog.html#x",
        "office/v1/%63atalog.html",
        "office/v1/sheet/00/first.html",
        "office/v1/sheet/+0/first.html",
        "office/v1/sheet/-1/first.html",
        "office/v1/sheet/2/first.html",
        "office/v1/sheet/0/first.html/extra",
        "office/v1/sheet/0/first.html ",
        "office/v1/sheet/0/row/0/col/1.html",
        "office/v1/sheet/0/row/1048577/col/1.html",
        "office/v1/sheet/0/row/1/col/16385.html",
        "office/v1/sheet/0/row/1/col/01.html",
        "office/v1/sheet/0/row/1/col/1.HTML",
        "office\\v1\\catalog.html",
        "https://example.test/office/v1/catalog.html",
        "file:/private.txt",
        "C:/private.txt",
        "//host/office/v1/catalog.html",
        "%2e%2e/private.txt",
        "office/v1/catalog.html\0",
    ] {
        assert!(
            prepared.resources.capture(&segments(path)).is_err(),
            "{path:?}"
        );
    }
    // Direct callers must not smuggle separators inside otherwise plausible segments.
    for value in [
        "sheet/0/first.html",
        "sheet\\0\\first.html",
        "sheet%2f0%2ffirst.html",
    ] {
        assert!(
            prepared
                .resources
                .capture(&["office".into(), "v1".into(), value.into()])
                .is_err()
        );
    }
    assert_eq!(fixture.generation.calls.load(Ordering::Acquire), calls);
    assert_eq!(
        fixture
            .source
            .native_resource_captures
            .load(Ordering::Acquire),
        0
    );
    release(prepared);
}

#[test]
fn office_scope_passes_canonical_root_target_unchanged_to_service() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    let path = "/office/v1/sheet/1/row/1048576/col/16384.html";
    let generated = prepared.resources.capture(&segments(path)).unwrap();
    assert_eq!(
        fixture.generation.paths.lock().unwrap().last().unwrap(),
        path
    );
    assert_eq!(generated.read_at(0, 8).unwrap(), b"HHHHHHHH");
    generated.release_on_worker().unwrap();
    release(prepared);
}

#[test]
fn office_changed_parent_blocks_resolution_before_generation_and_old_derived_reads() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    let calls = fixture.generation.calls.load(Ordering::Acquire);
    fixture.source.version.store(1, Ordering::Release);
    assert!(
        prepared
            .resources
            .capture(&segments("/office/v1/sheet/0/first.html"))
            .is_err()
    );
    assert_eq!(fixture.generation.calls.load(Ordering::Acquire), calls);
    assert!(prepared.main.revalidate().is_err());
    assert!(prepared.main.read_at(0, 8).is_err());
    fixture.source.version.store(0, Ordering::Release);
    assert!(
        prepared.main.read_at(0, 8).is_err(),
        "failed validation permanently revoked this parent"
    );
    release(prepared);
}

#[test]
fn office_parent_changed_during_generation_never_grants_successful_late_bytes() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    let parent = fixture.source.clone();
    *fixture.generation.action.lock().unwrap() = Some(Box::new(move || {
        parent.version.store(1, Ordering::Release);
    }));
    assert!(
        prepared
            .resources
            .capture(&segments("/office/v1/sheet/1/first.html"))
            .is_err()
    );
    assert!(prepared.main.read_at(0, 8).is_err());
    release(prepared);
}

#[test]
fn office_derived_reads_revalidate_parent_before_and_after_and_validate_eof() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    let before = fixture.source.validations.load(Ordering::Acquire);
    assert_eq!(prepared.main.read_at(0, 8).unwrap(), b"HHHHHHHH");
    assert!(fixture.source.validations.load(Ordering::Acquire) >= before + 2);
    fixture.source.version.store(1, Ordering::Release);
    assert!(prepared.main.read_at(prepared.main.length(), 1).is_err());
    release(prepared);
}

#[test]
fn office_close_is_observed_during_generation_without_waiting_for_ui_or_service_lock() {
    let fixture = Fixture::new();
    let close = Arc::new(AtomicBool::new(false));
    let prepared = fixture
        .prepare_with(Cancellation::observing(close.clone()))
        .unwrap();
    let (entered, entered_ready) = std::sync::mpsc::sync_channel(1);
    let (resume, resume_ready) = std::sync::mpsc::sync_channel(1);
    *fixture.generation.action.lock().unwrap() = Some(Box::new(move || {
        entered.send(()).unwrap();
        resume_ready
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
    }));
    let scope = prepared.resources.clone();
    let generation = std::thread::spawn(move || {
        let result = scope.capture(&segments("/office/v1/sheet/1/first.html"));
        scope.release_on_worker().unwrap();
        result
    });
    entered_ready
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    close.store(true, Ordering::Release);
    resume.send(()).unwrap();
    assert!(generation.join().unwrap().is_err());
    assert!(
        fixture
            .generation
            .observed_cancellation
            .load(Ordering::Acquire),
        "the real service token must observe window close while generation is in progress"
    );
    assert!(prepared.main.read_at(0, 8).is_err());
    release(prepared);
}

#[test]
fn office_host_resource_cancellation_rejects_late_success_and_never_clears_flag() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    let cancellation = ResourceCancellation::default();
    let host_flag = cancellation.flag();
    *fixture.generation.action.lock().unwrap() = Some(Box::new(move || {
        host_flag.store(true, Ordering::Release);
    }));
    assert!(
        prepared
            .resources
            .capture_with_cancellation(&segments("/office/v1/sheet/1/first.html"), &cancellation,)
            .is_err()
    );
    assert!(cancellation.is_cancelled());
    assert!(
        fixture
            .generation
            .observed_cancellation
            .load(Ordering::Acquire)
    );
    assert!(
        prepared
            .resources
            .capture_with_cancellation(&segments("/office/v1/sheet/0/first.html"), &cancellation,)
            .is_err()
    );
    release(prepared);
}

#[test]
fn office_initial_generation_observes_preexisting_close_and_checked_parent_release() {
    let fixture = Fixture::new();
    let close = Arc::new(AtomicBool::new(true));
    assert!(
        fixture
            .prepare_with(Cancellation::observing(close))
            .is_err()
    );
    assert_eq!(fixture.generation.calls.load(Ordering::Acquire), 0);
    assert_eq!(fixture.source.source_releases.load(Ordering::Acquire), 1);
    assert_eq!(fixture.source.resource_releases.load(Ordering::Acquire), 1);
}

#[test]
fn office_zero_cache_four_live_html_credits_bound_generation_and_reuse_released_capacity() {
    let fixture = Fixture::new();
    fixture.generation.size.store(MIB, Ordering::Release);
    let prepared = fixture.prepare();
    let a = first(&prepared, 0);
    let b = first(&prepared, 1);
    let c = first(&prepared, 0);
    let calls = fixture.generation.calls.load(Ordering::Acquire);
    assert!(
        prepared
            .resources
            .capture(&segments("/office/v1/sheet/0/first.html"))
            .is_err()
    );
    assert_eq!(
        fixture.generation.calls.load(Ordering::Acquire),
        calls,
        "reserve before generation"
    );
    a.release_on_worker().unwrap();
    let replacement = first(&prepared, 0);
    assert_eq!(fixture.generation.calls.load(Ordering::Acquire), calls + 1);
    assert_ne!(
        c.stable_version(),
        replacement.stable_version(),
        "no retained-window cache"
    );
    b.release_on_worker().unwrap();
    c.release_on_worker().unwrap();
    replacement.release_on_worker().unwrap();
    release(prepared);
}

#[test]
fn office_last_source_reference_owns_window_credit_until_checked_release() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    let a = first(&prepared, 0);
    let shared = a.clone();
    let b = first(&prepared, 1);
    let c = first(&prepared, 0);
    a.release_on_worker().unwrap();
    assert!(
        prepared
            .resources
            .capture(&segments("/office/v1/sheet/0/first.html"))
            .is_err()
    );
    shared.release_on_worker().unwrap();
    let replacement = first(&prepared, 0);
    b.release_on_worker().unwrap();
    c.release_on_worker().unwrap();
    replacement.release_on_worker().unwrap();
    release(prepared);
}

#[test]
fn office_oversized_window_never_publishes_and_failed_generation_returns_its_credit() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    fixture.generation.size.store(MIB + 1, Ordering::Release);
    assert!(
        prepared
            .resources
            .capture(&segments("/office/v1/sheet/0/first.html"))
            .is_err()
    );
    fixture.generation.size.store(MIB, Ordering::Release);
    let a = first(&prepared, 0);
    let b = first(&prepared, 1);
    let c = first(&prepared, 0);
    a.release_on_worker().unwrap();
    b.release_on_worker().unwrap();
    c.release_on_worker().unwrap();
    release(prepared);
}

#[test]
fn office_wrong_target_or_nonmonotonic_generation_cannot_become_derived_identity() {
    for wrong_sequence in [0, 1] {
        let fixture = Fixture::new();
        let prepared = fixture.prepare();
        *fixture.generation.returned_sequence.lock().unwrap() = Some(wrong_sequence);
        assert!(
            prepared
                .resources
                .capture(&segments("/office/v1/sheet/0/first.html"))
                .is_err()
        );
        release(prepared);
    }
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    *fixture.generation.returned_target.lock().unwrap() = Some(OfficeTarget::Catalog);
    assert!(
        prepared
            .resources
            .capture(&segments("/office/v1/sheet/1/first.html"))
            .is_err()
    );
    release(prepared);
}

#[test]
fn office_shared_scope_release_does_not_retire_service_or_original_parent() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    prepared.resources.clone().release_on_worker().unwrap();
    assert!(fixture.source.events.lock().unwrap().is_empty());
    let resource = first(&prepared, 1);
    resource.release_on_worker().unwrap();
    prepared.main.release_on_worker().unwrap();
    assert!(fixture.source.events.lock().unwrap().is_empty());
    prepared.resources.release_on_worker().unwrap();
    assert_eq!(
        *fixture.source.events.lock().unwrap(),
        ["generation", "source", "resources"]
    );
}

#[test]
fn office_derived_resource_can_outlive_scope_without_a_cycle_or_early_parent_release() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    let resource = first(&prepared, 1);
    prepared.main.release_on_worker().unwrap();
    prepared.resources.release_on_worker().unwrap();
    assert_eq!(*fixture.source.events.lock().unwrap(), ["generation"]);
    assert_eq!(resource.read_at(0, 8).unwrap(), b"HHHHHHHH");
    resource.release_on_worker().unwrap();
    assert_eq!(
        *fixture.source.events.lock().unwrap(),
        ["generation", "source", "resources"]
    );
}

#[test]
fn office_last_derived_source_propagates_unconfirmed_parent_release() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    prepared.resources.release_on_worker().unwrap();
    fixture.source.fail_release.store(true, Ordering::Release);
    let error = prepared.main.release_on_worker().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("controlled source-release failure")
    );
    assert_eq!(fixture.source.source_releases.load(Ordering::Acquire), 1);
    assert_eq!(
        fixture.source.resource_releases.load(Ordering::Acquire),
        0,
        "resource authority must remain retained when source release fails"
    );
    assert_eq!(fixture.source.source_drops.load(Ordering::Acquire), 0);
    assert_eq!(fixture.source.resource_drops.load(Ordering::Acquire), 0);
}

#[test]
fn office_parent_release_unwind_retains_original_resource_authority() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    prepared.resources.release_on_worker().unwrap();
    fixture.source.panic_release.store(true, Ordering::Release);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        prepared.main.release_on_worker()
    }));
    assert!(result.is_err());
    assert_eq!(fixture.source.resource_releases.load(Ordering::Acquire), 0);
    assert_eq!(fixture.source.source_drops.load(Ordering::Acquire), 0);
    assert_eq!(fixture.source.resource_drops.load(Ordering::Acquire), 0);
}

#[test]
fn office_service_open_failure_checks_original_parent_release_without_a_generated_page() {
    let fixture = Fixture::new();
    let result = prepare(
        fixture.capture(),
        OfficeFormat::Xlsx,
        Cancellation::default(),
    );
    assert!(result.is_err(), "ORIGINAL bytes are not an Office package");
    assert_eq!(fixture.source.source_releases.load(Ordering::Acquire), 1);
    assert_eq!(fixture.source.resource_releases.load(Ordering::Acquire), 1);
    assert_eq!(
        fixture
            .source
            .native_resource_captures
            .load(Ordering::Acquire),
        0
    );
}

#[test]
fn office_reader_bounds_source_turns_validates_eof_and_never_reads_after_close() {
    let fixture = Fixture::new();
    let closed = Arc::new(AtomicBool::new(false));
    let parent = Arc::new(Parent::new(
        fixture.capture(),
        Cancellation::observing(closed.clone()),
    ));
    let reader = CapturedReader(parent.clone());
    let mut output = vec![0; SOURCE_READ];
    assert_eq!(
        reader
            .read_at(0, &mut output, &Cancellation::default())
            .unwrap(),
        8
    );
    assert_eq!(&output[..8], b"ORIGINAL");
    assert_eq!(
        reader
            .read_at(8, &mut output, &Cancellation::default())
            .unwrap(),
        0
    );
    let reads = fixture.source.reads.load(Ordering::Acquire);
    let mut oversized = vec![0; SOURCE_READ + 1];
    assert_eq!(
        reader
            .read_at(0, &mut oversized, &Cancellation::default())
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    closed.store(true, Ordering::Release);
    assert!(
        reader
            .read_at(0, &mut output, &Cancellation::default())
            .is_err()
    );
    assert_eq!(fixture.source.reads.load(Ordering::Acquire), reads);
    drop(reader);
    parent.release_on_worker().unwrap();
    assert!(closed.load(Ordering::Acquire));
    assert_eq!(fixture.source.source_releases.load(Ordering::Acquire), 1);
}

#[test]
fn office_reader_rejects_source_revision_changed_during_read_and_revokes_parent() {
    let fixture = Fixture::new();
    let parent = Arc::new(Parent::new(fixture.capture(), Cancellation::default()));
    let reader = CapturedReader(parent.clone());
    fixture.source.mutate_on_read.store(true, Ordering::Release);
    let mut output = [0; 8];
    assert!(
        reader
            .read_at(0, &mut output, &Cancellation::default())
            .is_err()
    );
    assert_eq!(
        output, [0; 8],
        "unvalidated source bytes are never delivered to the parser"
    );
    fixture.source.version.store(0, Ordering::Release);
    assert!(
        reader
            .read_at(0, &mut output, &Cancellation::default())
            .is_err()
    );
    drop(reader);
    parent.release_on_worker().unwrap();
}

fn initial_content_targets() -> [OfficeTarget; 3] {
    // Nonzero sheet/slide IDs deliberately come from fake service metadata,
    // proving the Viewer never replaces a descriptor ID with an assumed zero.
    [
        OfficeTarget::DocumentWindow {
            document: DocumentId(0),
            block: 0,
        },
        OfficeTarget::SheetFirst { sheet: SheetId(7) },
        OfficeTarget::SlideWindow {
            slide: SlideId(3),
            block: 0,
        },
    ]
}

#[test]
fn office_initial_content_target_comes_from_service_metadata_for_each_family() {
    for target in initial_content_targets() {
        let fixture = Fixture::new();
        *fixture.generation.initial.lock().unwrap() = target;
        let prepared = fixture.prepare();
        assert_eq!(fixture.generation.initial_calls.load(Ordering::Acquire), 1);
        assert_eq!(*fixture.generation.paths.lock().unwrap(), [target.path()]);
        assert_eq!(*fixture.generation.rendered.lock().unwrap(), [target]);
        assert_eq!(prepared.main.read_at(0, 8).unwrap(), b"HHHHHHHH");
        assert_eq!(
            fixture.source.reads.load(Ordering::Acquire),
            0,
            "Viewer metadata selection must not open/read the parent"
        );
        let catalog = prepared
            .resources
            .capture(&segments("/office/v1/catalog.html"))
            .unwrap();
        assert_eq!(
            fixture.generation.rendered.lock().unwrap().last(),
            Some(&OfficeTarget::Catalog)
        );
        assert_ne!(prepared.main.stable_version(), catalog.stable_version());
        catalog.release_on_worker().unwrap();
        release(prepared);
        assert_eq!(fixture.source.source_releases.load(Ordering::Acquire), 1);
        assert_eq!(fixture.source.resource_releases.load(Ordering::Acquire), 1);
    }
}

#[test]
fn office_empty_initial_catalog_is_rendered_only_when_service_selects_it() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare();
    assert_eq!(fixture.generation.initial_calls.load(Ordering::Acquire), 1);
    assert_eq!(
        *fixture.generation.paths.lock().unwrap(),
        [OfficeTarget::Catalog.path()]
    );
    assert_eq!(
        *fixture.generation.rendered.lock().unwrap(),
        [OfficeTarget::Catalog]
    );
    assert_eq!(fixture.generation.calls.load(Ordering::Acquire), 1);
    assert_eq!(
        fixture
            .source
            .native_resource_captures
            .load(Ordering::Acquire),
        0
    );
    release(prepared);
    assert_eq!(fixture.source.source_releases.load(Ordering::Acquire), 1);
    assert_eq!(fixture.source.resource_releases.load(Ordering::Acquire), 1);
}

#[test]
fn office_initial_content_render_error_surfaces_without_catalog_fallback() {
    for target in initial_content_targets() {
        let fixture = Fixture::new();
        *fixture.generation.initial.lock().unwrap() = target;
        let failure = PreviewError {
            message: "controlled first-content failure".into(),
            details: Some("controlled first-content details".into()),
        };
        *fixture.generation.render_error.lock().unwrap() = Some(failure.clone());
        let error = match fixture.prepare_with(Cancellation::default()) {
            Err(error) => error,
            Ok(prepared) => {
                release(prepared);
                panic!("failed initial content must not become successful catalog output");
            }
        };
        assert_eq!(error, failure, "the message and its details");
        assert_eq!(fixture.generation.initial_calls.load(Ordering::Acquire), 1);
        assert_eq!(fixture.generation.calls.load(Ordering::Acquire), 1);
        assert_eq!(*fixture.generation.paths.lock().unwrap(), [target.path()]);
        assert_eq!(*fixture.generation.rendered.lock().unwrap(), [target]);
        assert_eq!(
            *fixture.source.events.lock().unwrap(),
            ["generation", "source", "resources"]
        );
        assert_eq!(fixture.source.source_releases.load(Ordering::Acquire), 1);
        assert_eq!(fixture.source.resource_releases.load(Ordering::Acquire), 1);
        assert_eq!(fixture.source.source_drops.load(Ordering::Acquire), 1);
        assert_eq!(fixture.source.resource_drops.load(Ordering::Acquire), 1);
    }
}

#[test]
fn office_xlsx_prepares_first_real_cell_before_any_resource_navigation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("first-content.xlsx");
    std::fs::write(
        &path,
        include_bytes!("office_native_fixture/two-sheet-windows.xlsx"),
    )
    .unwrap();
    let parent = CapturedFile::capture(&path).unwrap();
    let prepared = prepare(parent, OfficeFormat::Xlsx, Cancellation::default()).unwrap();
    let bytes = prepared.main.read_at(0, RESOURCE_READ).unwrap();
    let html = std::str::from_utf8(&bytes).unwrap();
    assert!(
        html.contains("NATIVE_ALPHA_A1_V1"),
        "first prepared HTML must contain real first-sheet cells"
    );
    assert!(!html.contains("NATIVE_BETA_A1_V1"));
    assert!(
        html.contains("href=\"office/v1/catalog.html\""),
        "All sheets remains available"
    );
    let catalog = prepared
        .resources
        .capture(&segments("/office/v1/catalog.html"))
        .unwrap();
    let bytes = catalog.read_at(0, RESOURCE_READ).unwrap();
    let html = std::str::from_utf8(&bytes).unwrap();
    assert!(
        html.contains("Workbook sheets")
            && html.contains("NativeAlpha")
            && html.contains("NativeBeta")
    );
    assert!(!html.contains("NATIVE_ALPHA_A1_V1"));
    catalog.release_on_worker().unwrap();
    release(prepared);
}

#[test]
fn office_family_suffixes_select_their_format() {
    for (name, expected) in [
        ("report.DOCX", OfficeFormat::Docx),
        ("macros.docm", OfficeFormat::Docx),
        ("letter.dotx", OfficeFormat::Docx),
        ("letter.dotm", OfficeFormat::Docx),
        ("deck.pptx", OfficeFormat::Pptx),
        ("deck.pptm", OfficeFormat::Pptx),
        ("show.ppsx", OfficeFormat::Pptx),
        ("show.ppsm", OfficeFormat::Pptx),
        ("theme.potx", OfficeFormat::Pptx),
        ("theme.potm", OfficeFormat::Pptx),
        ("book.xlsx", OfficeFormat::Xlsx),
        ("book.xlsm", OfficeFormat::Xlsx),
        ("book.xltx", OfficeFormat::Xlsx),
        ("book.associated", OfficeFormat::Xlsx),
        ("text.odt", OfficeFormat::Odt),
        ("text.ott", OfficeFormat::Odt),
        ("table.ods", OfficeFormat::Ods),
        ("table.ots", OfficeFormat::Ods),
        ("slides.odp", OfficeFormat::Odp),
        ("slides.otp", OfficeFormat::Odp),
    ] {
        assert_eq!(format(Path::new(name)), expected, "{name}");
    }
}

/// What `prepare` reports for a file named `name` of these bytes, which it must refuse.
fn refusal(name: &str, bytes: &[u8]) -> PreviewError {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    let parent = CapturedFile::capture(&path).unwrap();
    match prepare(parent, format(&path), Cancellation::default()) {
        Err(error) => error,
        Ok(prepared) => {
            release(prepared);
            panic!("{name} prepared");
        }
    }
}

/// A one-part ZIP package whose part is encrypted (general purpose flag bit 0), as ZIP
/// password protection stores it; the stored bytes stand for the ciphertext.
fn encrypted_package() -> Vec<u8> {
    const NAME: &[u8] = b"[Content_Types].xml";
    const DATA: &[u8] = b"ciphertext";
    // Version 2.0 needed, encrypted, stored, no time or CRC (never checked before the
    // flag), both sizes and the name's length.
    let mut common = vec![20, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    for size in [DATA.len() as u32; 2] {
        common.extend_from_slice(&size.to_le_bytes());
    }
    common.extend_from_slice(&(NAME.len() as u16).to_le_bytes());
    let mut zip = b"PK\x03\x04".to_vec();
    zip.extend_from_slice(&common);
    zip.extend_from_slice(&[0; 2]);
    zip.extend_from_slice(NAME);
    zip.extend_from_slice(DATA);
    let central = zip.len() as u32;
    // Made by 2.0; no extra field or comment, disk 0, no attributes, local header at 0.
    zip.extend_from_slice(b"PK\x01\x02\x14\x00");
    zip.extend_from_slice(&common);
    zip.extend_from_slice(&[0; 16]);
    zip.extend_from_slice(NAME);
    let size = zip.len() as u32 - central;
    // Disk 0 holds the one entry; the directory's size and offset; no comment.
    zip.extend_from_slice(b"PK\x05\x06\x00\x00\x00\x00\x01\x00\x01\x00");
    zip.extend_from_slice(&size.to_le_bytes());
    zip.extend_from_slice(&central.to_le_bytes());
    zip.extend_from_slice(&[0; 2]);
    zip
}

/// The status line gets the plain sentence (`OfficeError::message`); the notice's Copy
/// details gets the service's reason (`OfficeError::details`).
#[test]
fn office_open_refusal_names_the_document_and_keeps_the_service_reason_as_details() {
    let mut compound = vec![0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
    compound.resize(4096, 0);
    assert_eq!(
        refusal("protected.pptx", &compound),
        PreviewError {
            message: "This presentation is password-protected or saved in the older binary \
                      Office format (.doc, .xls or .ppt), which the preview cannot open."
                .into(),
            details: Some(
                "Unsupported Office content: is password-protected or saved in the older \
                 binary Office format (.doc, .xls or .ppt), which the preview cannot open"
                    .into()
            ),
        },
        "older binary format"
    );
    assert_eq!(
        refusal("protected.docx", &encrypted_package()),
        PreviewError {
            message: "This document is password-protected, so the preview cannot open it.".into(),
            details: Some("Encrypted Office package".into()),
        },
        "password-protected package"
    );
}

#[test]
fn office_refusal_details_name_the_service_reason_but_not_for_a_cancelled_preview() {
    let damaged = PreviewError::office(
        &midnight_office_preview::OfficeError::Malformed("central record bounds"),
        OfficeFormat::Xlsx,
    );
    assert_eq!(
        damaged.message,
        "This workbook is damaged or is not a valid Excel (.xlsx) file, so the preview \
         cannot show it."
    );
    assert_eq!(
        damaged.details.as_deref(),
        Some("Malformed Office package: central record bounds")
    );
    let cancelled = PreviewError::office(
        &midnight_office_preview::OfficeError::Cancelled,
        OfficeFormat::Xlsx,
    );
    assert_eq!(
        cancelled.details, None,
        "nobody waits for a cancelled preview"
    );
}
