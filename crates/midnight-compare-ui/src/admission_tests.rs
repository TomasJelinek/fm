//! PRIVATE typed-API draft. UNCOMPILED / NOT RUN; no canonical source was edited.
use crate::{admission, identity_tests::TestRuntime};
use midnight_compare_engine::{
    FileComparatorPinError, FileComparatorPolicySource, OpenError, Output,
    OwnedFileComparatorPolicy, Service, Session,
};
use midnight_core::{
    CompiledFileComparatorPolicy, DirectoryIndex, Entry, EntryId, EntryKind,
    ExternalCompareSessionCredit, FileComparatorPolicyStore, FileComparatorPreferences,
    FileComparatorRule, FileComparatorSnapshot, FileComparatorTarget, FileCompareLaunch,
    FileCompareSource, FilterMode, PreparedFileComparatorPolicy, SelectionSnapshot,
    ViewerAppearance, ViewerGeometry,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

// Only metadata is exercised. Any matching or content capture is a fixture failure.
struct RawPolicy(Arc<FileComparatorPreferences>);
impl CompiledFileComparatorPolicy for RawPolicy {
    fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        &self.0
    }
    fn select(&self, _: &[PathBuf; 2]) -> Option<usize> {
        panic!("unexpected matching")
    }
    fn matcher_budget_bytes(&self) -> usize {
        0
    }
}
fn publish(store: &FileComparatorPolicyStore, expression: &str) -> FileComparatorSnapshot {
    let raw = Arc::new(FileComparatorPreferences {
        rules: vec![FileComparatorRule {
            enabled: true,
            mode: FilterMode::Masks,
            expression: expression.into(),
            case_sensitive: false,
            target: FileComparatorTarget::BuiltInText,
        }],
    });
    store.publish(
        PreparedFileComparatorPolicy::new(Arc::new(RawPolicy(raw)))
            .with_id(store.prepare_id().unwrap()),
    )
}

// This admission-only owner contains no compiled matcher/retirement resource.
// Rich source tests never admit a Session; successful tests use Unspecified inputs.
struct MetadataOwner {
    snapshot: FileComparatorSnapshot,
    _credit: Arc<dyn ExternalCompareSessionCredit>,
}
impl OwnedFileComparatorPolicy for MetadataOwner {
    fn snapshot(&self) -> FileComparatorSnapshot {
        self.snapshot.clone()
    }
    fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        &self.snapshot.preferences
    }
    fn select(&self, _: &[PathBuf; 2]) -> Option<usize> {
        panic!("unexpected matching")
    }
}

#[derive(Clone, Copy)]
enum Behavior {
    Current,
    RaceOnRefresh,
    RetirementBusy,
}
struct Source {
    store: FileComparatorPolicyStore,
    behavior: Behavior,
    reads: AtomicUsize,
    pins: Mutex<Vec<FileComparatorSnapshot>>,
}
impl FileComparatorPolicySource for Source {
    fn snapshot(&self) -> FileComparatorSnapshot {
        assert_eq!(
            self.reads.fetch_add(1, Ordering::SeqCst),
            0,
            "more than one refresh"
        );
        let snapshot = self.store.snapshot();
        if matches!(self.behavior, Behavior::RaceOnRefresh) {
            // A coherent publication races the metadata read, making the retry stale too.
            publish(&self.store, "*.after-refresh");
        }
        snapshot
    }
    fn pin(
        &self,
        expected: &FileComparatorSnapshot,
        session: Arc<dyn ExternalCompareSessionCredit>,
    ) -> Result<Box<dyn OwnedFileComparatorPolicy>, FileComparatorPinError> {
        self.pins.lock().unwrap().push(expected.clone());
        if matches!(self.behavior, Behavior::RetirementBusy) {
            return Err(FileComparatorPinError::RetirementBusy);
        }
        let accepted = self
            .store
            .pin(expected)
            .map_err(|_| FileComparatorPinError::PolicyChanged)?;
        Ok(Box::new(MetadataOwner {
            snapshot: accepted.snapshot(),
            _credit: session,
        }))
    }
}

struct Fixture {
    service: Service,
    source: Arc<Source>,
    captures: Arc<AtomicUsize>,
    _runtime: TestRuntime,
}
impl Fixture {
    fn new(behavior: Behavior) -> Self {
        let runtime = TestRuntime::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .max_blocking_threads(2)
                .build()
                .unwrap(),
        );
        let source = Arc::new(Source {
            store: FileComparatorPolicyStore::default(),
            behavior,
            reads: AtomicUsize::new(0),
            pins: Mutex::new(Vec::new()),
        });
        publish(&source.store, "*.current");
        let captures = Arc::new(AtomicUsize::new(0));
        let called = captures.clone();
        let service = Service::with_native_capture(
            runtime.handle().clone(),
            runtime.spawner(),
            Arc::new(move |_, _| {
                called.fetch_add(1, Ordering::SeqCst);
                Err("Unspecified admission must not capture content".into())
            }),
        )
        .with_comparator_policies(source.clone());
        Self {
            service,
            source,
            captures,
            _runtime: runtime,
        }
    }
    fn open(&self, launch: FileCompareLaunch) -> Result<Session, OpenError> {
        admission::open(&self.service, launch, appearance(), geometry())
    }
    fn pins(&self) -> Vec<FileComparatorSnapshot> {
        self.source.pins.lock().unwrap().clone()
    }
    fn assert_no_refresh_or_capture(&self) {
        assert_eq!(self.source.reads.load(Ordering::SeqCst), 0);
        assert_eq!(self.captures.load(Ordering::SeqCst), 0);
    }
}
fn appearance() -> ViewerAppearance {
    ViewerAppearance {
        family: "monospace".into(),
        font_size: 12.0,
        foreground: [255; 4],
        background: [20, 20, 20, 255],
        selection: [40, 60, 80, 255],
        selection_text: [0; 4],
        accent: [80, 100, 120, 255],
    }
}
fn geometry() -> ViewerGeometry {
    ViewerGeometry {
        width: 320,
        height: 200,
        scale: 1.0,
    }
}
fn unresolved(snapshot: &FileComparatorSnapshot) -> FileCompareLaunch {
    FileCompareLaunch {
        sources: vec![
            FileCompareSource::Unspecified,
            FileCompareSource::Unspecified,
        ],
        disabled_plugins: Arc::from(["disabled.original".into()]),
        comparators: snapshot.preferences.clone(),
        comparator_policy: snapshot.id,
    }
}
fn same_policy(actual: &FileComparatorSnapshot, expected: &FileComparatorSnapshot) {
    assert_eq!(actual.id, expected.id);
    assert!(Arc::ptr_eq(&actual.preferences, &expected.preferences));
}
fn chooser(session: &Session) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match session.mailbox.take() {
            Some(Output::ChooseFiles(choice)) => {
                assert!(choice.paths.iter().all(|path| path.as_os_str().is_empty()));
                assert!(!choice.is_error);
                return;
            }
            Some(Output::Status(_)) | None => {}
            Some(_) => panic!("unspecified inputs must remain a provider-free chooser"),
        }
        assert!(Instant::now() < deadline, "no initial chooser output");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn current_policy_is_admitted_once_without_metadata_refresh() {
    let fixture = Fixture::new(Behavior::Current);
    let current = fixture.source.store.snapshot();
    let session = fixture.open(unresolved(&current)).unwrap();
    chooser(&session);
    let pins = fixture.pins();
    assert_eq!(pins.len(), 1);
    same_policy(&pins[0], &current);
    fixture.assert_no_refresh_or_capture();
    session.close();
}

#[test]
fn stale_policy_refreshes_both_metadata_fields_once() {
    let fixture = Fixture::new(Behavior::Current);
    let stale = FileComparatorSnapshot::default();
    let current = fixture.source.store.snapshot();
    assert_ne!(stale.id, current.id);
    assert!(!Arc::ptr_eq(&stale.preferences, &current.preferences));
    let existing = fixture.open(unresolved(&current)).unwrap();
    chooser(&existing);
    let session = fixture.open(unresolved(&stale)).unwrap();
    chooser(&session);
    let pins = fixture.pins();
    assert_eq!(pins.len(), 3);
    same_policy(&pins[0], &current);
    same_policy(&pins[1], &stale);
    same_policy(&pins[2], &current);
    assert_eq!(fixture.source.reads.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.captures.load(Ordering::SeqCst), 0);
    existing.close();
    session.close();
}

#[test]
fn second_stale_pin_preserves_original_sources_and_stops() {
    let fixture = Fixture::new(Behavior::RaceOnRefresh);
    let refreshed = fixture.source.store.snapshot();
    let mut original = unresolved(&FileComparatorSnapshot::default());
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("raw 文档 e\u{301}")
        .join("..");
    let entries: Arc<[Entry]> = Arc::from([Entry::new(
        EntryId(9),
        directory.join("original.c"),
        EntryKind::File,
    )]);
    let view = Arc::new(DirectoryIndex::from_entries(&entries));
    let selection = Arc::new(SelectionSnapshot::from_bits(view.clone(), vec![1]).unwrap());
    let visible: Arc<[EntryId]> = Arc::from([EntryId(9)]);
    original.sources = vec![
        FileCompareSource::Selected {
            entries: entries.clone(),
            view: view.clone(),
            selection: selection.clone(),
        },
        FileCompareSource::RootSameName {
            directory: directory.clone(),
            entries: entries.clone(),
            view: view.clone(),
            visible: visible.clone(),
        },
    ];
    let mut attempts = Vec::new();
    let result = admission::open_observed(
        &fixture.service,
        original.clone(),
        appearance(),
        geometry(),
        |launch| attempts.push(launch.clone()),
    );
    assert!(matches!(result, Err(OpenError::PolicyChanged)));
    assert_eq!(attempts.len(), 2);
    assert_eq!(fixture.pins().len(), 2);
    assert_eq!(fixture.source.reads.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.captures.load(Ordering::SeqCst), 0);
    same_policy(&fixture.pins()[1], &refreshed);
    for attempt in attempts {
        assert!(Arc::ptr_eq(
            &attempt.disabled_plugins,
            &original.disabled_plugins
        ));
        let [
            FileCompareSource::Selected {
                entries: a,
                view: b,
                selection: c,
            },
            FileCompareSource::RootSameName {
                directory: d,
                entries: e,
                view: f,
                visible: g,
            },
        ] = attempt.sources.as_slice()
        else {
            panic!("admission replaced the original source variants")
        };
        assert!(Arc::ptr_eq(a, &entries) && Arc::ptr_eq(b, &view) && Arc::ptr_eq(c, &selection));
        assert_eq!(d, &directory);
        assert!(Arc::ptr_eq(e, &entries) && Arc::ptr_eq(f, &view) && Arc::ptr_eq(g, &visible));
    }
}

#[test]
fn any_number_of_sessions_open_with_one_attempt_and_one_pin_each() {
    let fixture = Fixture::new(Behavior::Current);
    let current = fixture.source.store.snapshot();
    let sessions: Vec<_> = (0..6)
        .map(|_| {
            let mut attempts = 0;
            let session = admission::open_observed(
                &fixture.service,
                unresolved(&current),
                appearance(),
                geometry(),
                |_| attempts += 1,
            )
            .unwrap();
            assert_eq!(attempts, 1);
            session
        })
        .collect();
    for session in &sessions {
        chooser(session);
    }
    assert_eq!(fixture.pins().len(), 6);
    fixture.assert_no_refresh_or_capture();
    for session in &sessions {
        session.close();
    }
}

#[test]
fn unavailable_is_returned_without_refresh_or_retry() {
    let fixture = Fixture::new(Behavior::RetirementBusy);
    let mut attempts = 0;
    let result = admission::open_observed(
        &fixture.service,
        unresolved(&fixture.source.store.snapshot()),
        appearance(),
        geometry(),
        |_| attempts += 1,
    );
    assert!(matches!(result, Err(OpenError::Unavailable(_))));
    assert_eq!(attempts, 1);
    assert_eq!(fixture.pins().len(), 1);
    fixture.assert_no_refresh_or_capture();
}
