//! Windows native comparison composition; all tests are explicit ignored fixtures.
//! Only settings persistence is a provider-free data sink. Policy acceptance,
//! guarded ownership, Explorer adapters, installed captures and process launch are real.
#![cfg(windows)]

use super::Backend;
use midnight_compare_engine::{
    ExternalCompareChoiceOutcome, Input, OpenError, Output, Service, Session,
};
use midnight_core::{
    Command, ComparatorValidationProof, Event, FileComparatorPreferences, FileComparatorProgram,
    FileComparatorRule, FileComparatorSnapshot, FileComparatorTarget, FileCompareLaunch,
    FileCompareSource, FilterMode, FsError, PreferencesRevision, SettingsService, UserPreferences,
    ViewerAppearance, ViewerGeometry,
};
use midnight_engine::{CommandSender, Engine};
use midnight_native_filesystem_host::InstalledProvider;
use midnight_work_budget::resident::{Owner, Spawner, Ticket};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, OpenOptions},
    io::{self, Read},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(20);
const ASYNC_WORKERS: usize = 2;
const BLOCKING_WORKERS: usize = 16;
const COMPARE_WORKERS: usize = 2;
const MAX_REPORT: u64 = 8 * 1024 * 1024;
static CASE_IDS: AtomicU64 = AtomicU64::new(0);

/// Test data sink only: real Engine code validates/compiles and accepts each save.
/// Native platform settings are intentionally never configured, so no profile is written.
#[derive(Default)]
struct SettingsData {
    preferences: Mutex<UserPreferences>,
    prepared: AtomicUsize,
    loads: AtomicUsize,
    acceptance_loads: AtomicUsize,
    unprepared_saves: AtomicUsize,
    prepared_entered: AtomicUsize,
}

impl SettingsService for SettingsData {
    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        self.loads.fetch_add(1, Ordering::AcqRel);
        eprintln!("native settings phase=load-entry");
        Ok(self.preferences.lock().unwrap().clone())
    }

    fn load_preferences_for_acceptance(&self) -> Result<UserPreferences, FsError> {
        self.acceptance_loads.fetch_add(1, Ordering::AcqRel);
        eprintln!("native settings phase=acceptance-load-entry");
        self.load_preferences()
    }

    fn save_preferences(&self, _: UserPreferences) -> Result<(), FsError> {
        self.unprepared_saves.fetch_add(1, Ordering::AcqRel);
        eprintln!("native settings phase=unexpected-unprepared-save");
        Err(FsError::from_io(
            "native comparison settings fixture",
            Path::new(""),
            io::Error::other("Engine must use its real prepared acceptance path"),
        ))
    }

    fn save_preferences_prepared(
        &self,
        preferences: UserPreferences,
        proof: ComparatorValidationProof<'_>,
    ) -> Result<(), FsError> {
        self.prepared_entered.fetch_add(1, Ordering::AcqRel);
        eprintln!("native settings phase=prepared-save-entry");
        assert!(proof.matches(&preferences.file_comparators));
        *self.preferences.lock().unwrap() = preferences;
        self.prepared.fetch_add(1, Ordering::AcqRel);
        eprintln!("native settings phase=prepared-save-complete");
        Ok(())
    }
}

impl SettingsData {
    fn diagnostic(&self) -> String {
        format!(
            "loads={} acceptance_loads={} unprepared_saves={} prepared_entered={} prepared_complete={}",
            self.loads.load(Ordering::Acquire),
            self.acceptance_loads.load(Ordering::Acquire),
            self.unprepared_saves.load(Ordering::Acquire),
            self.prepared_entered.load(Ordering::Acquire),
            self.prepared.load(Ordering::Acquire)
        )
    }
}

struct CompareDomain {
    owner: Option<Owner>,
    spawner: Spawner,
    holders: [Option<Ticket>; COMPARE_WORKERS],
    runtime: tokio::runtime::Runtime,
}

impl CompareDomain {
    fn start() -> io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(ASYNC_WORKERS)
            .max_blocking_threads(BLOCKING_WORKERS)
            .enable_all()
            .build()?;
        let (owner, spawner) = Owner::start("native-compare", COMPARE_WORKERS)?;
        Ok(Self {
            owner: Some(owner),
            spawner,
            holders: Default::default(),
            runtime,
        })
    }

    fn wait_for_holders(&mut self, deadline: Instant) -> Vec<String> {
        let deadline = tokio::time::Instant::from_std(deadline);
        let holders = &mut self.holders;
        self.runtime.block_on(async {
            let mut failures = Vec::new();
            for (index, slot) in holders.iter_mut().enumerate() {
                let Some(ticket) = slot.as_mut() else {
                    continue;
                };
                match tokio::time::timeout_at(deadline, ticket).await {
                    Ok(result) => {
                        *slot = None;
                        if let Err(error) = result {
                            failures.push(format!("holder {index}: {error:?}"));
                        }
                    }
                    Err(_) => failures.push(format!("holder {index}: cleanup timed out")),
                }
            }
            failures
        })
    }

    fn shutdown(&mut self, deadline: Instant) {
        let Some(owner) = self.owner.take() else {
            return;
        };
        let shutdown = owner.shutdown(self.runtime.handle());
        let holders = self.wait_for_holders(deadline);
        let threads = self.runtime.block_on(async {
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), shutdown).await
        });
        if !holders.is_empty() || !matches!(threads, Ok(Ok(()))) {
            let failure =
                format!("native Compare cleanup: holders={holders:?}, threads={threads:?}");
            if std::thread::panicking() {
                eprintln!("{failure}");
            } else {
                panic!("{failure}");
            }
        }
    }
}

impl Drop for CompareDomain {
    fn drop(&mut self) {
        // Also covers unwind before Harness has adopted the domain.
        self.shutdown(Instant::now() + TIMEOUT);
    }
}

struct Harness {
    service: Service,
    commands: CommandSender,
    events: tokio::sync::mpsc::Receiver<Event>,
    engine: Option<tokio::task::JoinHandle<()>>,
    settings: Arc<SettingsData>,
    revision: u64,
    compare: CompareDomain,
}

impl Harness {
    fn new() -> Self {
        let compare = CompareDomain::start().expect("start native Compare fixture domain");
        let lifetime = Engine::lifetime().expect("start native Engine lifetime");
        let filesystem = Arc::new(InstalledProvider::from_installation(
            required_directory("MIDNIGHT_NATIVE_TEST_INSTALLATION"),
            None,
        ));
        let local = Arc::new(midnight_platform::LocalFileSystem::new(None));
        let settings = Arc::new(SettingsData::default());
        let (events, observer) = tokio::sync::mpsc::channel(32);
        let mut engine = Engine::new(lifetime, filesystem.clone(), events);
        engine.configure_user_commands().unwrap().install(local);
        engine
            .configure_settings()
            .unwrap()
            .install(settings.clone());
        let (commands, receiver) = engine.command_channel(32).unwrap();
        // EXACT production composition: no test source, capture or launcher is injected.
        let service = Backend::new(&engine, commands.clone(), filesystem.clone()).configure(
            crate::comparator::service(
                compare.runtime.handle(),
                compare.spawner.clone(),
                filesystem,
            ),
        );
        let engine = compare.runtime.spawn(engine.run(receiver));
        Self {
            service,
            commands,
            events: observer,
            engine: Some(engine),
            settings,
            revision: 0,
            compare,
        }
    }

    fn accept(&mut self, program: FileComparatorProgram) -> FileComparatorSnapshot {
        self.revision = self.revision.checked_add(1).unwrap();
        let revision = PreferencesRevision(self.revision);
        let preferences = UserPreferences {
            file_comparators: FileComparatorPreferences {
                rules: vec![FileComparatorRule {
                    enabled: true,
                    mode: FilterMode::Masks,
                    expression: "*.c;*.cpp".into(),
                    case_sensitive: false,
                    target: FileComparatorTarget::External(program),
                }],
            },
            ..UserPreferences::default()
        };
        let expected_raw = preferences.file_comparators.clone();
        let commands = self.commands.clone();
        let settings = self.settings.clone();
        let mut phase = "before-command-admission";
        let mut event_count = 0_usize;
        let mut last_event = None;
        let began = Instant::now();
        eprintln!(
            "native acceptance phase={phase} revision={} engine_finished={:?} {}",
            revision.0,
            self.engine.as_ref().map(|task| task.is_finished()),
            settings.diagnostic()
        );
        let outcome = {
            let events = &mut self.events;
            let engine = self.engine.as_ref();
            self.compare.runtime.block_on(async {
                commands.send(Command::SavePreferences {
                    revision, preferences: Box::new(preferences),
                }).await.unwrap();
                phase = "command-admitted-awaiting-result";
                eprintln!("native acceptance phase={phase} revision={} engine_finished={:?} {}",
                    revision.0, engine.map(|task| task.is_finished()), settings.diagnostic());
                tokio::time::timeout(TIMEOUT, async {
                    loop {
                        let event = events.recv().await.unwrap_or_else(|| {
                            eprintln!("native acceptance phase=event-channel-closed revision={} events={event_count} engine_finished={:?} {}",
                                revision.0, engine.map(|task| task.is_finished()), settings.diagnostic());
                            panic!("Engine stopped before settings acceptance");
                        });
                        event_count += 1;
                        last_event = Some(std::mem::discriminant(&event));
                        // Discriminants and typed revisions are metadata only. Never dump
                        // preferences, source contents or native argument payloads here.
                        if event_count <= 32 {
                            eprintln!("native acceptance phase=event-observed revision={} events={event_count} kind={last_event:?}", revision.0);
                        }
                        if let Event::PreferencesSaved { revision: actual, result } = event {
                            phase = "preferences-saved-observed";
                            eprintln!("native acceptance phase={phase} expected={} actual={} ok={} {}",
                                revision.0, actual.0, result.is_ok(), settings.diagnostic());
                            assert_eq!(actual, revision);
                            break result.expect("real Engine compiler must accept the native rule");
                        }
                    }
                }).await
            })
        };
        if outcome.is_err() {
            eprintln!(
                "native acceptance phase=timeout last_phase={phase} revision={} elapsed_ms={} events={event_count} last_event={last_event:?} engine_finished={:?} {}",
                revision.0,
                began.elapsed().as_millis(),
                self.engine.as_ref().map(|task| task.is_finished()),
                settings.diagnostic()
            );
            if self.engine.as_ref().is_some_and(|task| task.is_finished()) {
                let joined = self.compare.runtime.block_on(self.engine.take().unwrap());
                match joined {
                    Ok(()) => eprintln!("native acceptance completed_engine_join=ok"),
                    Err(error) => eprintln!(
                        "native acceptance completed_engine_join=error panic={} cancelled={}",
                        error.is_panic(),
                        error.is_cancelled()
                    ),
                }
            }
        }
        let snapshot = outcome.expect("real settings acceptance timed out");
        assert_eq!(*snapshot.preferences, expected_raw);
        let live = self.service.comparator_snapshot();
        assert_eq!(live.id, snapshot.id);
        assert!(Arc::ptr_eq(&live.preferences, &snapshot.preferences));
        assert_eq!(
            self.settings.prepared.load(Ordering::Acquire),
            self.revision as usize
        );
        snapshot
    }

    fn open(
        &self,
        snapshot: &FileComparatorSnapshot,
        paths: Option<&[PathBuf; 2]>,
    ) -> Result<Session, OpenError> {
        self.service.open(
            FileCompareLaunch {
                sources: paths.map_or_else(Vec::new, |paths| {
                    paths
                        .iter()
                        .cloned()
                        .map(FileCompareSource::Focused)
                        .collect()
                }),
                disabled_plugins: Arc::from([]),
                comparators: snapshot.preferences.clone(),
                comparator_policy: snapshot.id,
            },
            ViewerAppearance {
                family: "monospace".into(),
                font_size: 14.0,
                foreground: [240; 4],
                background: [24, 24, 24, 255],
                selection: [80, 100, 180, 255],
                selection_text: [0; 4],
                accent: [120; 4],
            },
            ViewerGeometry {
                width: 400,
                height: 250,
                scale: 1.0,
            },
        )
    }

    fn open_when_available(&self, snapshot: &FileComparatorSnapshot, deadline: Instant) -> Session {
        loop {
            assert!(
                Instant::now() < deadline,
                "native credits did not return within the shared setup/cancellation deadline"
            );
            match self.open(snapshot, None) {
                Ok(session) => return session,
                Err(OpenError::PolicyChanged) => {
                    panic!("stable native snapshot unexpectedly changed")
                }
                Err(error) => {
                    assert!(
                        Instant::now() < deadline,
                        "native credits did not return: {error}"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // Every native test owns its sessions. Their local Drops cancel before
        // this guard is dropped during success or assertion unwind.
        let deadline = Instant::now() + TIMEOUT;
        let timeout = tokio::time::Instant::from_std(deadline);
        let commands = self.commands.clone();
        let engine = self.engine.take();
        let engine_failure = self.compare.runtime.block_on(async {
            let _ = tokio::time::timeout_at(timeout, commands.send(Command::Shutdown)).await;
            match engine {
                Some(mut engine) => match tokio::time::timeout_at(timeout, &mut engine).await {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(format!("native Engine shutdown failed: {error}")),
                    Err(_) => {
                        engine.abort();
                        Some("native Engine shutdown timed out; abort requested".into())
                    }
                },
                None => None,
            }
        });
        if let Some(failure) = &engine_failure {
            eprintln!("{failure}");
        }
        self.compare.shutdown(deadline);
        if let Some(failure) = engine_failure
            && !std::thread::panicking()
        {
            panic!("{failure}");
        }
    }
}

struct Fixtures {
    root: PathBuf,
    paths: [PathBuf; 2],
    cwd: PathBuf,
    report: PathBuf,
    recorder: PathBuf,
}

fn required_directory(name: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")));
    assert!(path.is_absolute(), "{name} must be absolute");
    assert!(
        path.is_dir(),
        "{name} must name the separately prepared directory"
    );
    path
}

fn raw_parent_name() -> OsString {
    let mut units: Vec<u16> = "raw-parent-".encode_utf16().collect();
    units.extend([0xd800, b'-' as u16, b'x' as u16]);
    OsString::from_wide(&units)
}

fn raw_basename() -> OsString {
    let mut units: Vec<u16> = "raw-basename-".encode_utf16().collect();
    units.extend([0xdfff, b'.' as u16, b'c' as u16]);
    OsString::from_wide(&units)
}

fn native_bytes(value: &OsStr) -> Vec<u8> {
    value.encode_wide().flat_map(u16::to_le_bytes).collect()
}

impl Fixtures {
    fn new(label: &str) -> Self {
        let work = required_directory("MIDNIGHT_NATIVE_COMPARE_WORK");
        assert!(
            work.to_str().is_some(),
            "report template location must be Unicode"
        );
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert!(
            fs::canonicalize(&work)
                .unwrap()
                .starts_with(fs::canonicalize(workspace.join("_build")).unwrap())
        );
        let sequence = CASE_IDS.fetch_add(1, Ordering::AcqRel);
        let root = work.join(format!("{label}-{}-{sequence}", std::process::id()));
        fs::create_dir(&root).expect("provide a fresh owned native run directory");
        let cwd = root.join("cwd 文档 & $x; '");
        fs::create_dir(&cwd).unwrap();
        let mut parent = root.join(raw_parent_name());
        fs::create_dir(&parent).unwrap();
        for index in 0..5 {
            parent = parent.join(format!("long-native-{index}-{}", "x".repeat(45)));
            fs::create_dir(&parent).unwrap();
        }
        let paths = [
            parent.join("before 文档 e\u{301}.cpp"),
            parent.join("after Žluťoučký é.c"),
        ];
        assert!(
            paths
                .iter()
                .all(|path| path.file_name().unwrap().to_str().is_some())
        );
        assert!(paths.iter().all(|path| path.to_str().is_none()));
        assert!(
            paths
                .iter()
                .all(|path| path.as_os_str().encode_wide().count() > 260)
        );
        // Contents are data; successful External must never interpret them as commands.
        fs::write(&paths[0], b"native comparison before\n").unwrap();
        fs::write(&paths[1], b"native comparison after\n").unwrap();
        let recorder = PathBuf::from(
            std::env::var_os("MIDNIGHT_USER_COMMAND_PROBE")
                .expect("Build supplies the precompiled existing recorder"),
        );
        assert!(recorder.is_absolute() && recorder.is_file());
        assert_eq!(
            recorder.file_name(),
            Some(OsStr::new("user-command-probe.exe"))
        );
        assert!(
            fs::canonicalize(&recorder)
                .unwrap()
                .starts_with(fs::canonicalize(workspace.join("_build")).unwrap())
        );
        Self {
            report: root.join("argv.bin"),
            root,
            paths,
            cwd,
            recorder,
        }
    }

    fn program(&self) -> FileComparatorProgram {
        FileComparatorProgram {
            executable: self.recorder.clone(),
            arguments: vec![
                self.report.to_str().unwrap().to_owned(),
                "{right}".into(),
                "{left}".into(),
                String::new(),
                "--literal={left}".into(),
                "{left}".into(),
                " & echo injected > sentinel; $x %PATH% !x! 文档 e\u{301} ".into(),
            ],
            working_directory: Some(self.cwd.clone()),
        }
    }

    fn assert_capture_readers_released(&self) {
        // Actual Windows sharing, not a destructor counter. The recorder never
        // opens either argv path. A retained content reader conflicts with share=0.
        for path in &self.paths {
            drop(
                OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(path)
                    .expect("comparison native reader remained open after terminal result"),
            );
        }
    }
}

fn terminal(session: &Session) -> Output {
    let deadline = Instant::now() + TIMEOUT;
    let mut last_status = String::new();
    loop {
        match session.mailbox.take() {
            Some(Output::Status(status)) => last_status = status,
            Some(output) => return output,
            None => {}
        }
        assert!(
            Instant::now() < deadline,
            "native comparison timed out: {last_status}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

struct Report {
    cwd: Vec<u8>,
    arguments: Vec<Vec<u8>>,
}

/// Parse the EXISTING recorder's MECMD1 format with bounds, so a partial write
/// cannot cause a false failure. This does not add a recorder/process protocol.
fn parse_report(bytes: &[u8]) -> Option<Report> {
    fn number(bytes: &mut &[u8]) -> Option<usize> {
        let value = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as usize;
        *bytes = &bytes[4..];
        Some(value)
    }
    fn record(bytes: &mut &[u8]) -> Option<Vec<u8>> {
        let length = number(bytes)?;
        let value = bytes.get(..length)?.to_vec();
        *bytes = &bytes[length..];
        Some(value)
    }
    let mut bytes = bytes.strip_prefix(b"MECMD1\0")?;
    let cwd = record(&mut bytes)?;
    let count = number(&mut bytes)?;
    if count > 64 {
        return None;
    }
    let arguments = (0..count)
        .map(|_| record(&mut bytes))
        .collect::<Option<Vec<_>>>()?;
    let environment = number(&mut bytes)?;
    if environment > 6 {
        return None;
    }
    for _ in 0..environment {
        let _ = record(&mut bytes)?;
        let _ = record(&mut bytes)?;
    }
    bytes.is_empty().then_some(Report { cwd, arguments })
}

fn recorded(path: &Path) -> Report {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match fs::File::open(path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(MAX_REPORT + 1).read_to_end(&mut bytes).unwrap();
                assert!(
                    bytes.len() as u64 <= MAX_REPORT,
                    "owned recorder report exceeded its budget"
                );
                if let Some(report) = parse_report(&bytes) {
                    return report;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("owned recorder report unreadable: {error}"),
        }
        assert!(
            Instant::now() < deadline,
            "owned recorder did not finish its MECMD1 report"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
#[ignore = "SERIAL: requires protected installed native provider, owned work dir and Build-precompiled existing argv recorder"]
fn actual_composed_external_compare_preserves_raw_parent_argv_and_cwd() {
    let fixtures = Fixtures::new("argv");
    let mut harness = Harness::new();
    let snapshot = harness.accept(fixtures.program());
    let session = harness.open(&snapshot, Some(&fixtures.paths)).unwrap();
    match terminal(&session) {
        Output::ExternalStarted { pid, .. } => assert_ne!(pid, 0),
        Output::ChooseFiles(choice) => panic!(
            "native launch did not start; typed outcome {:?}: {}",
            choice.external_outcome, choice.message
        ),
        _ => panic!("External rule fell back to built-in content"),
    }
    let report = recorded(&fixtures.report);
    assert_eq!(
        report.arguments,
        vec![
            native_bytes(fixtures.paths[1].as_os_str()),
            native_bytes(fixtures.paths[0].as_os_str()),
            Vec::new(),
            native_bytes(OsStr::new("--literal={left}")),
            native_bytes(fixtures.paths[0].as_os_str()),
            native_bytes(OsStr::new(
                " & echo injected > sentinel; $x %PATH% !x! 文档 e\u{301} "
            )),
        ]
    );
    assert_eq!(report.cwd, native_bytes(fixtures.cwd.as_os_str()));
    assert!(!fixtures.cwd.join("sentinel").exists());
    assert!(!fixtures.root.join("sentinel").exists());
    fixtures.assert_capture_readers_released();
    session.close();
}

#[test]
#[ignore = "SERIAL: requires protected installed native provider, owned work dir and Build-precompiled existing argv recorder"]
fn actual_not_started_preserves_raw_drafts_and_releases_partial_and_complete_captures() {
    for missing_source in [false, true] {
        let fixtures = Fixtures::new(if missing_source {
            "missing-input"
        } else {
            "missing-program"
        });
        let mut harness = Harness::new();
        let mut program = fixtures.program();
        let mut paths = fixtures.paths.clone();
        if missing_source {
            paths[1].set_file_name("missing Unicode 文档.c");
        } else {
            program.executable = fixtures.root.join("missing-program.exe");
        }
        let snapshot = harness.accept(program);
        let session = harness.open(&snapshot, Some(&paths)).unwrap();
        match terminal(&session) {
            Output::ChooseFiles(choice) => {
                assert_eq!(
                    choice.external_outcome,
                    Some(ExternalCompareChoiceOutcome::NotStarted)
                );
                assert!(choice.is_error);
                assert_eq!(choice.paths, paths);
            }
            Output::ExternalStarted { .. } => panic!("known missing native input/program started"),
            _ => panic!("known native refusal fell back to built-in comparison"),
        }
        fixtures.assert_capture_readers_released();
        session.close();
        drop(session);
        drop(harness);
        assert!(!fixtures.report.exists());
    }
}

struct CompareHold {
    release: Arc<(Mutex<bool>, Condvar)>,
    active: Arc<AtomicUsize>,
    deadline: Instant,
}

struct ActiveBlocker(Arc<AtomicUsize>);

impl Drop for ActiveBlocker {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl CompareHold {
    fn new(domain: &mut CompareDomain, deadline: Instant) -> Self {
        // This guard exists before any spawn or fallible wait: unwind releases
        // already running and queued blockers without relying on a timer.
        let hold = Self {
            release: Arc::new((Mutex::new(false), Condvar::new())),
            active: Arc::new(AtomicUsize::new(0)),
            deadline,
        };
        let (entered, observed) = mpsc::sync_channel(COMPARE_WORKERS);
        for holder in &mut domain.holders {
            assert!(holder.is_none(), "native Compare holder already submitted");
            let release = hold.release.clone();
            let active = hold.active.clone();
            let entered = entered.clone();
            let work = Box::new(move || {
                active.fetch_add(1, Ordering::AcqRel);
                let _active = ActiveBlocker(active);
                let _ = entered.try_send(());
                let (state, wake) = &*release;
                let state = state.lock().unwrap_or_else(|error| error.into_inner());
                drop(
                    wake.wait_while(state, |released| !*released)
                        .unwrap_or_else(|error| error.into_inner()),
                );
            });
            *holder = Some(
                domain
                    .spawner
                    .spawn(domain.runtime.handle(), work)
                    .unwrap_or_else(|refused| {
                        panic!("native Compare holder refused: {:?}", refused.reason())
                    }),
            );
        }
        drop(entered);
        for _ in 0..COMPARE_WORKERS {
            let remaining = hold
                .deadline
                .checked_duration_since(Instant::now())
                .expect("native blocking barrier exhausted its shared setup deadline");
            observed
                .recv_timeout(remaining)
                .expect("native blocking barrier did not acknowledge every held worker");
        }
        hold.assert_saturated();
        hold
    }

    fn assert_saturated(&self) {
        let (state, _) = &*self.release;
        assert!(
            !*state.lock().unwrap_or_else(|error| error.into_inner()),
            "native blocking barrier released before cancellation"
        );
        assert_eq!(
            self.active.load(Ordering::Acquire),
            COMPARE_WORKERS,
            "native blocking barrier lost a held worker"
        );
        assert!(
            Instant::now() < self.deadline,
            "native blocking barrier exhausted its shared setup/cancellation deadline"
        );
    }
}

impl Drop for CompareHold {
    fn drop(&mut self) {
        let (state, wake) = &*self.release;
        *state.lock().unwrap_or_else(|error| error.into_inner()) = true;
        wake.notify_all();
    }
}

#[test]
#[ignore = "SERIAL: requires protected installed native provider, owned work dir and Build-precompiled existing argv recorder"]
fn actual_cancel_before_worker_returns_two_guarded_session_credits_without_spawn() {
    let fixtures = Fixtures::new("cancel");
    let mut harness = Harness::new();
    let snapshot = harness.accept(fixtures.program());
    // Both real resident workers acknowledge entry before the sessions queue on
    // the same Compare spawner. Engine settings and transient work stay live.
    let deadline = Instant::now() + TIMEOUT;
    let hold = CompareHold::new(&mut harness.compare, deadline);
    hold.assert_saturated();
    let first = harness.open(&snapshot, Some(&fixtures.paths)).unwrap();
    let second = harness.open(&snapshot, Some(&fixtures.paths)).unwrap();
    assert!(matches!(
        harness.open(&snapshot, None),
        Err(OpenError::SessionLimit)
    ));
    first.close();
    second.close();
    assert!(first.send(Input::Seek(0.0)).is_err() && second.send(Input::Seek(0.0)).is_err());
    assert!(matches!(
        harness.open(&snapshot, None),
        Err(OpenError::SessionLimit)
    ));
    hold.assert_saturated();
    drop(hold);
    let holder_failures = harness.compare.wait_for_holders(deadline);
    assert!(
        holder_failures.is_empty(),
        "native Compare holders: {holder_failures:?}"
    );
    let fresh_first = harness.open_when_available(&snapshot, deadline);
    let fresh_second = harness.open_when_available(&snapshot, deadline);
    assert!(first.mailbox.take().is_none() && second.mailbox.take().is_none());
    fixtures.assert_capture_readers_released();
    assert!(
        Instant::now() < deadline,
        "native Compare cancellation exceeded its shared setup/cleanup deadline"
    );
    fresh_first.close();
    fresh_second.close();
    drop(fresh_first);
    drop(fresh_second);
    drop(first);
    drop(second);
    drop(harness);
    assert!(!fixtures.report.exists());
}

#[test]
#[ignore = "SERIAL: requires protected installed native provider, owned work dir and Build-precompiled existing argv recorder"]
fn actual_non_unicode_basename_uses_automatic_instead_of_authorizing_external() {
    let fixtures = Fixtures::new("raw-basename");
    let mut paths = fixtures.paths.clone();
    paths[0].set_file_name(raw_basename());
    assert!(paths[0].file_name().unwrap().to_str().is_none());
    fs::write(&paths[0], b"automatic text comparison\n").unwrap();
    let mut harness = Harness::new();
    let snapshot = harness.accept(fixtures.program());
    let session = harness.open(&snapshot, Some(&paths)).unwrap();
    match terminal(&session) {
        Output::Frame { frame, .. } => assert!(!frame.image),
        Output::ExternalStarted { .. } => {
            panic!("non-Unicode basename authorized an External rule")
        }
        Output::ChooseFiles(choice) => panic!("automatic native text failed: {}", choice.message),
        Output::Status(_) => unreachable!(),
    }
    session.close();
    drop(session);
    drop(harness);
    assert!(!fixtures.report.exists());
}
