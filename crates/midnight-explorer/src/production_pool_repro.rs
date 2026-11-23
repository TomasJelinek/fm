//! Integration regression for production Engine/Viewer/Compare scheduling.
//! Resident Engine/Viewer/Compare with the production transient runtime (2/16),
//! at most four windows.
//! Watch and Drag are explicitly CONTROLLED native edges; settings is a persistence
//! sink. Actual listing, prepared acceptance, report readers, captures and text
//! workers remain real. No profile/OS clipboard writes and no pressure filler jobs.
//! Catalog selection explicitly uses the same qualified C installation fixture.
#![cfg(all(test, windows))]

#[path = "production_pool_maximum_state.rs"]
mod maximum_state;

use midnight_core::*;
use midnight_engine::{CommandSender, Engine};
use midnight_native_filesystem_host::InstalledProvider;
use midnight_work_budget::resident::{Owner, Spawner};
use std::{
    collections::{HashSet, VecDeque},
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread::{self, ThreadId},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const DEADLINE: Duration = Duration::from_secs(20);
const BLOCKING_LIMIT: usize = 16;
type TestResult<T> = Result<T, String>;

fn require(value: bool, message: &str) -> TestResult<()> {
    if value { Ok(()) } else { Err(message.into()) }
}
fn geometry(width: u32) -> ViewerGeometry {
    ViewerGeometry {
        width,
        height: 240,
        scale: 1.0,
    }
}
fn appearance() -> ViewerAppearance {
    ViewerAppearance {
        family: "monospace".into(),
        font_size: 14.0,
        foreground: [240, 240, 240, 255],
        background: [24, 24, 24, 255],
        selection: [80, 100, 180, 255],
        selection_text: [0; 4],
        accent: [120, 120, 120, 255],
    }
}
fn preferences() -> UserPreferences {
    UserPreferences {
        file_comparators: FileComparatorPreferences {
            rules: vec![FileComparatorRule {
                enabled: true,
                mode: FilterMode::Masks,
                expression: "*.txt".into(),
                case_sensitive: false,
                target: FileComparatorTarget::BuiltInText,
            }],
        },
        ..UserPreferences::default()
    }
}

#[derive(Default)]
struct SettingsSink {
    value: Mutex<UserPreferences>,
    prepared_entered: AtomicUsize,
    prepared_complete: AtomicUsize,
    unprepared: AtomicUsize,
}
impl SettingsService for SettingsSink {
    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        Ok(self.value.lock().unwrap().clone())
    }
    fn load_preferences_for_acceptance(&self) -> Result<UserPreferences, FsError> {
        self.load_preferences()
    }
    fn save_preferences(&self, _: UserPreferences) -> Result<(), FsError> {
        self.unprepared.fetch_add(1, Ordering::SeqCst);
        Err(FsError::from_io(
            "repro settings",
            Path::new(""),
            io::Error::other("prepared path required"),
        ))
    }
    fn save_preferences_prepared(
        &self,
        value: UserPreferences,
        proof: ComparatorValidationProof<'_>,
    ) -> Result<(), FsError> {
        self.prepared_entered.fetch_add(1, Ordering::SeqCst);
        if !proof.matches(&value.file_comparators) {
            return Err(FsError::from_io(
                "repro settings",
                Path::new(""),
                io::Error::other("incoherent proof"),
            ));
        }
        *self.value.lock().unwrap() = value;
        self.prepared_complete.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Default)]
struct EdgeTrace {
    watches: Mutex<Vec<(PathBuf, ThreadId)>>,
    watch_drops: Mutex<Vec<(ThreadId, ThreadId)>>,
    drag_opens: Mutex<Vec<ThreadId>>,
    drag_drops: Mutex<Vec<(ThreadId, ThreadId)>>,
    drag_starts: AtomicUsize,
}
struct ControlledWatch(Arc<EdgeTrace>);
struct Subscription {
    trace: Arc<EdgeTrace>,
    created: ThreadId,
}
impl WatchSubscription for Subscription {}
impl Drop for Subscription {
    fn drop(&mut self) {
        self.trace
            .watch_drops
            .lock()
            .unwrap()
            .push((self.created, thread::current().id()));
    }
}
impl WatchService for ControlledWatch {
    fn watch_directory(
        &self,
        path: &Path,
        _: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Box<dyn WatchSubscription>, FsError> {
        let created = thread::current().id();
        self.0
            .watches
            .lock()
            .unwrap()
            .push((path.to_owned(), created));
        Ok(Box::new(Subscription {
            trace: self.0.clone(),
            created,
        }))
    }
}
struct ControlledDrag(Arc<EdgeTrace>);
struct DragSession {
    trace: Arc<EdgeTrace>,
    created: ThreadId,
}
impl FileDragDropService for ControlledDrag {
    fn open_session(
        &self,
        _: DragHost,
        _: SharedDropTargets,
        _: Arc<dyn FileDropSink>,
    ) -> Result<Box<dyn FileDragDropSession>, FsError> {
        let created = thread::current().id();
        self.0.drag_opens.lock().unwrap().push(created);
        Ok(Box::new(DragSession {
            trace: self.0.clone(),
            created,
        }))
    }
}
impl FileDragDropSession for DragSession {
    fn service_pending(&mut self) -> Result<(), FsError> {
        assert_eq!(self.created, thread::current().id());
        Ok(())
    }
    fn start_drag(&mut self, _: &FileClipboard, _: &Cancellation) -> Result<DragFinish, FsError> {
        self.trace.drag_starts.fetch_add(1, Ordering::SeqCst);
        assert_eq!(self.created, thread::current().id());
        Err(FsError::from_io(
            "repro unexpected native drag",
            Path::new(""),
            io::Error::other("only PrepareDrag and CancelDrag are allowed"),
        ))
    }
}
impl Drop for DragSession {
    fn drop(&mut self) {
        self.trace
            .drag_drops
            .lock()
            .unwrap()
            .push((self.created, thread::current().id()));
    }
}

#[derive(Debug, Default)]
struct ReadGate {
    released: Mutex<bool>,
    wake: Condvar,
}
impl ReadGate {
    fn release(&self) {
        *self.released.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.wake.notify_all();
    }
    fn wait(&self) -> bool {
        let ready = self.released.lock().unwrap_or_else(|e| e.into_inner());
        *self
            .wake
            .wait_timeout_while(ready, DEADLINE, |ready| !*ready)
            .unwrap_or_else(|e| e.into_inner())
            .0
    }
    fn wait_until_released(&self) {
        let ready = self.released.lock().unwrap_or_else(|e| e.into_inner());
        drop(
            self.wake
                .wait_while(ready, |ready| !*ready)
                .unwrap_or_else(|e| e.into_inner()),
        );
    }
}
struct ReleaseGate(Arc<ReadGate>);
impl Drop for ReleaseGate {
    fn drop(&mut self) {
        self.0.release();
    }
}
#[derive(Debug, Default)]
struct ReportTrace {
    entered: Mutex<Vec<ThreadId>>,
    dropped: Mutex<Vec<ThreadId>>,
}
#[derive(Debug)]
struct ReportFile {
    file: Mutex<File>,
    length: u64,
    gate: Option<Arc<ReadGate>>,
    trace: Arc<ReportTrace>,
}
impl GeneratedTextSource for ReportFile {
    fn length(&self) -> u64 {
        self.length
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        self.trace
            .entered
            .lock()
            .unwrap()
            .push(thread::current().id());
        if let Some(gate) = &self.gate
            && !gate.wait()
        {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "report readiness gate expired",
            ));
        }
        let mut file = self.file.lock().unwrap();
        file.seek(SeekFrom::Start(offset))?;
        let count = self.length.saturating_sub(offset).min(output.len() as u64) as usize;
        file.read_exact(&mut output[..count])?;
        Ok(count)
    }
}
impl Drop for ReportFile {
    fn drop(&mut self) {
        self.trace
            .dropped
            .lock()
            .unwrap()
            .push(thread::current().id());
    }
}

struct Fixtures {
    root: PathBuf,
    panes: [PathBuf; 2],
    files: [PathBuf; 2],
    report: PathBuf,
}
impl Fixtures {
    fn new() -> TestResult<Self> {
        let work = PathBuf::from(
            std::env::var_os("MIDNIGHT_POOL_REPRO_WORK").ok_or("set MIDNIGHT_POOL_REPRO_WORK")?,
        );
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        require(work.is_absolute(), "work directory must be absolute")?;
        require(
            fs::canonicalize(&work)
                .map_err(|e| e.to_string())?
                .starts_with(
                    fs::canonicalize(workspace.join("_build")).map_err(|e| e.to_string())?,
                ),
            "work must be inside workspace _build",
        )?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let root = work.join(format!("production-pool-{}-{nonce}", std::process::id()));
        fs::create_dir(&root).map_err(|e| e.to_string())?;
        let panes = [root.join("left"), root.join("right")];
        for pane in &panes {
            fs::create_dir(pane).map_err(|e| e.to_string())?;
        }
        let files = [
            panes[0].join("left 文档.txt"),
            panes[1].join("right 文档.txt"),
        ];
        fs::write(&files[0], b"POOL_NATIVE_LEFT\nshared text\n").map_err(|e| e.to_string())?;
        fs::write(&files[1], b"POOL_NATIVE_RIGHT\nshared text\n").map_err(|e| e.to_string())?;
        let report = root.join("report.txt");
        fs::write(&report, b"POOL_REAL_REPORT\n").map_err(|e| e.to_string())?;
        Ok(Self {
            root,
            panes,
            files,
            report,
        })
    }
    fn report(
        &self,
        gate: Option<Arc<ReadGate>>,
        trace: Arc<ReportTrace>,
    ) -> TestResult<Arc<dyn GeneratedTextSource>> {
        let file = File::open(&self.report).map_err(|e| e.to_string())?;
        let length = file.metadata().map_err(|e| e.to_string())?.len();
        Ok(Arc::new(ReportFile {
            file: Mutex::new(file),
            length,
            gate,
            trace,
        }))
    }
    fn check_readers_released(&self, deadline: Instant) -> TestResult<()> {
        for path in &self.files {
            loop {
                require(
                    Instant::now() < deadline,
                    "native reader release deadline elapsed",
                )?;
                match OpenOptions::new().read(true).share_mode(0).open(path) {
                    Ok(file) => {
                        drop(file);
                        break;
                    }
                    // The installed transport can finish its real close just after
                    // its owning worker exits. Retry only actual Windows sharing
                    // or lock violations, within the SAME whole-cleanup deadline.
                    Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => {
                        require(
                            Instant::now() < deadline,
                            &format!("retained native reader {}: {error}", path.display()),
                        )?;
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => {
                        return Err(format!(
                            "exclusive reopen {} failed: {error}",
                            path.display()
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

struct Listing {
    entries: Arc<[Entry]>,
    index: Arc<DirectoryIndex>,
    navigation: NavigationId,
}
struct Harness {
    runtime: Option<tokio::runtime::Runtime>,
    viewer_owner: Option<Owner>,
    compare_owner: Option<Owner>,
    viewer_spawner: Spawner,
    commands: CommandSender,
    events: tokio::sync::mpsc::Receiver<Event>,
    pending: VecDeque<Event>,
    engine: Option<tokio::task::JoinHandle<()>>,
    filesystem: Arc<InstalledProvider>,
    prepared_catalogs: Option<midnight_viewer_engine::PreparedCatalogs>,
    settings: Arc<SettingsSink>,
    edges: Arc<EdgeTrace>,
    reports: Arc<ReportTrace>,
    read_gate: Option<ReleaseGate>,
    host_lifetime: Cancellation,
    viewer_service: Option<midnight_viewer_engine::Service>,
    compare_service: Option<midnight_compare_engine::Service>,
    spare: Option<midnight_viewer_engine::ImageSpareRetirement>,
    viewers: Vec<midnight_viewer_engine::WindowSession>,
    compares: Vec<midnight_compare_engine::Session>,
    cleaned: bool,
}
impl Harness {
    fn new() -> TestResult<Self> {
        Self::with_services(|_, _| Ok(()))
    }

    fn with_services(
        configure: impl FnOnce(&mut Engine, &Arc<InstalledProvider>) -> TestResult<()>,
    ) -> TestResult<Self> {
        let sharded = std::env::var_os("TOKIO_UNSTABLE_SHARDED_BLOCKING_QUEUE");
        require(
            sharded.as_ref().is_none_or(|v| v.is_empty() || v == "0"),
            "FIFO startup proof requires pinned non-sharded blocking queue",
        )?;
        let installation = PathBuf::from(
            std::env::var_os("MIDNIGHT_NATIVE_TEST_INSTALLATION")
                .ok_or("set qualified MIDNIGHT_NATIVE_TEST_INSTALLATION")?,
        );
        require(
            installation.is_absolute() && installation.is_dir(),
            "invalid installed provider directory",
        )?;
        let engine_lifetime = Engine::lifetime().map_err(|e| e.to_string())?;
        let (viewer_owner, viewer_spawner) =
            Owner::start("midnight-viewer", midnight_viewer_engine::RESIDENT_WORKERS)
                .map_err(|error| format!("Viewer resident startup failed: {error}"))?;
        let (compare_owner, compare_spawner) = Owner::start("midnight-compare", 2)
            .map_err(|error| format!("Compare resident startup failed: {error}"))?;
        // Existing qualified installation fixture selects the real application
        // explicitly. This never changes the process-global executable resolver.
        // Discovery runs on this backend setup thread before any pressure exists.
        let application = installation.join("midnight-explorer.exe");
        let prepared_catalogs = midnight_viewer_engine::PreparedCatalogs::discover(&application)?;
        eprintln!(
            "controlled catalog selection: qualified application={}",
            application.display()
        );
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(BLOCKING_LIMIT)
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let filesystem = Arc::new(InstalledProvider::from_installation(installation, None));
        let settings = Arc::new(SettingsSink::default());
        let edges = Arc::new(EdgeTrace::default());
        let (events, receiver) = tokio::sync::mpsc::channel(128);
        let mut engine = Engine::new(engine_lifetime, filesystem.clone(), events);
        engine
            .configure_settings()
            .map_err(|e| e.to_string())?
            .install(settings.clone());
        engine
            .configure_watch()
            .map_err(|e| e.to_string())?
            .install(Arc::new(ControlledWatch(edges.clone())));
        engine
            .configure_drag_drop()
            .map_err(|e| e.to_string())?
            .install(Arc::new(ControlledDrag(edges.clone())));
        configure(&mut engine, &filesystem)?;
        let (commands, inbox) = engine.command_channel(32).map_err(|e| e.to_string())?;
        let compare_service =
            crate::comparator_external::Backend::new(&engine, commands.clone(), filesystem.clone())
                .configure(crate::comparator::service(
                    runtime.handle(),
                    compare_spawner,
                    filesystem.clone(),
                ));
        let engine = runtime.spawn(engine.run(inbox));
        Ok(Self {
            runtime: Some(runtime),
            viewer_owner: Some(viewer_owner),
            compare_owner: Some(compare_owner),
            viewer_spawner,
            commands,
            events: receiver,
            pending: VecDeque::new(),
            engine: Some(engine),
            filesystem,
            prepared_catalogs: Some(prepared_catalogs),
            settings,
            edges,
            reports: Arc::default(),
            read_gate: None,
            host_lifetime: Cancellation::default(),
            viewer_service: None,
            compare_service: Some(compare_service),
            spare: None,
            viewers: Vec::new(),
            compares: Vec::new(),
            cleaned: false,
        })
    }
    fn runtime(&self) -> &tokio::runtime::Runtime {
        self.runtime.as_ref().unwrap()
    }
    fn send(&self, command: Command) -> TestResult<()> {
        self.send_until(command, Instant::now() + DEADLINE)
    }
    fn send_until(&self, command: Command, deadline: Instant) -> TestResult<()> {
        self.runtime().block_on(async {
            tokio::time::timeout(
                deadline.saturating_duration_since(Instant::now()),
                self.commands.send(command),
            )
            .await
            .map_err(|_| "command admission timed out".to_string())?
            .map_err(|_| "command admission refused".to_string())
        })
    }
    fn pump(&mut self) -> TestResult<()> {
        while let Ok(event) = self.events.try_recv() {
            require(
                self.pending.len() < 256,
                "test event backlog exceeded bounded retention",
            )?;
            self.pending.push_back(event);
        }
        Ok(())
    }
    fn take(&mut self, matches: impl Fn(&Event) -> bool) -> TestResult<Option<Event>> {
        self.pump()?;
        Ok(self
            .pending
            .iter()
            .position(matches)
            .and_then(|index| self.pending.remove(index)))
    }
    fn event(&mut self, matches: impl Fn(&Event) -> bool) -> TestResult<Event> {
        self.event_until(matches, Instant::now() + DEADLINE)
    }
    fn event_until(
        &mut self,
        matches: impl Fn(&Event) -> bool,
        deadline: Instant,
    ) -> TestResult<Event> {
        loop {
            require(
                Instant::now() < deadline,
                "expected Engine event deadline elapsed",
            )?;
            if let Some(event) = self.take(&matches)? {
                return Ok(event);
            }
            require(Instant::now() < deadline, "expected Engine event timed out")?;
            require(
                self.engine.as_ref().is_some_and(|task| !task.is_finished()),
                "Engine exited while waiting for event",
            )?;
            thread::sleep(Duration::from_millis(2));
        }
    }
    fn save(&mut self, revision: u64) -> TestResult<FileComparatorSnapshot> {
        self.send(Command::SavePreferences {
            revision: PreferencesRevision(revision),
            preferences: Box::new(preferences()),
        })?;
        let Event::PreferencesSaved { result, .. } = self.event(
            |e| matches!(e, Event::PreferencesSaved { revision: r, .. } if r.0 == revision),
        )?
        else {
            unreachable!()
        };
        let snapshot = result.map_err(|e| format!("prepared save failed: {e:?}"))?;
        require(
            *snapshot.preferences == preferences().file_comparators,
            "accepted raw preferences differ",
        )?;
        let current = self.compare_service.as_ref().unwrap().comparator_snapshot();
        require(
            snapshot.id == current.id && Arc::ptr_eq(&snapshot.preferences, &current.preferences),
            "policy publication incoherent",
        )?;
        Ok(snapshot)
    }
    fn navigate(&mut self, pane: PaneId, path: &Path) -> TestResult<Listing> {
        let navigation = NavigationId(1);
        self.send(Command::Navigate {
            pane,
            navigation,
            path: path.to_owned(),
            sort: SortSpec::default(),
        })?;
        let event = self.event(|event| matches!(event,
            Event::ListingFinished { pane: p, navigation: n, .. } | Event::ListingFailed { pane: p, navigation: n, .. }
            if *p == pane && *n == navigation))?;
        let Event::ListingFinished { entries, index, .. } = event else {
            return Err("real native listing failed".into());
        };
        self.send(Command::PaneCommitted(VisibleDirectory {
            pane,
            navigation,
            path: path.to_owned(),
            entries: entries.clone(),
        }))?;
        Ok(Listing {
            entries,
            index,
            navigation,
        })
    }
    fn prepare_residents(
        &mut self,
        files: &Fixtures,
    ) -> TestResult<(FileComparatorSnapshot, [Listing; 2])> {
        let first = self.save(1)?; // Real prepared-save readiness.
        self.event(|e| matches!(e, Event::NetworkAvailability(_)))?; // Settle finite startup discovery.
        let listings = [
            self.navigate(PaneId::Left, &files.panes[0])?,
            self.navigate(PaneId::Right, &files.panes[1])?,
        ];
        let deadline = Instant::now() + DEADLINE;
        loop {
            self.pump()?;
            let calls = self.edges.watches.lock().unwrap();
            let left = calls.iter().find(|(path, _)| path == &files.panes[0]);
            let right = calls.iter().find(|(path, _)| path == &files.panes[1]);
            if let (Some((_, a)), Some((_, b))) = (left, right) {
                eprintln!("real watch registrations entered: left={a:?} right={b:?}");
                break;
            }
            drop(calls);
            require(Instant::now() < deadline, "watch callbacks did not enter")?;
            thread::sleep(Duration::from_millis(2));
        }
        let targets = Arc::new(Mutex::new(DropTargets {
            host_generation: 1,
            revision: 1,
            targets: vec![FileDropTarget {
                pane: PaneId::Left,
                navigation: listings[0].navigation,
                directory: Arc::new(files.panes[0].clone()),
                rect: DropRect {
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 100,
                },
            }],
        }));
        // Sentinel HWND is interpreted ONLY by ControlledDrag, never an OS adapter.
        self.send(Command::AttachDragDrop {
            host: DragHost {
                generation: 1,
                native: NativeDragWindow::Windows { hwnd: 1 },
                lifetime: self.host_lifetime.clone(),
            },
            targets,
        })?;
        self.event(|e| {
            matches!(
                e,
                Event::DragDrop(DragDropEvent::Ready {
                    host_generation: 1,
                    supported: true
                })
            )
        })?;
        self.send(Command::PrepareDrag {
            id: DragId(1),
            pane: PaneId::Left,
            navigation: listings[0].navigation,
            files: SelectedFiles {
                selection: Arc::default(),
                view: listings[0].index.clone(),
                fallback: Some(files.files[0].clone()),
            },
        })?;
        self.event(|e| {
            matches!(
                e,
                Event::DragDrop(DragDropEvent::Prepared {
                    id: DragId(1),
                    item_count: 1
                })
            )
        })?;
        self.send(Command::CancelDrag(DragId(1)))?;
        let catalogs = self
            .prepared_catalogs
            .take()
            .ok_or("catalog fixture already adopted")?;
        let viewer = midnight_viewer_engine::Service::with_prepared_catalogs(
            self.runtime().handle().clone(),
            self.viewer_spawner.clone(),
            crate::native_content::factory_with_resources(
                self.filesystem.clone(),
                Arc::new(midnight_platform::LocalFileSystem::new(None)),
                self.runtime().handle(),
            ),
            catalogs,
        );
        self.spare = Some(viewer.image_spare_retirement());
        self.viewer_service = Some(viewer);
        let snapshot = self.save(2)?; // The spare has been submitted. Unchanged save must reuse.
        require(
            first.id == snapshot.id && Arc::ptr_eq(&first.preferences, &snapshot.preferences),
            "unchanged save recompiled/replaced policy",
        )?;
        let gate = Arc::new(ReadGate::default());
        self.read_gate = Some(ReleaseGate(gate.clone()));
        for request in [41, 42] {
            self.send(Command::ReadDiagnosticText {
                request: DesktopRequestId(request),
                content: files.report(Some(gate.clone()), self.reports.clone())?,
                cancel: Cancellation::default(),
            })?;
        }
        let deadline = Instant::now() + DEADLINE;
        loop {
            self.pump()?;
            let entered = self.reports.entered.lock().unwrap();
            if entered.len() == 2 {
                require(
                    entered[0] != entered[1],
                    "report requests did not enter two real readers",
                )?;
                break;
            }
            drop(entered);
            require(
                Instant::now() < deadline,
                "report readers did not both enter",
            )?;
            thread::sleep(Duration::from_millis(2));
        }
        gate.release();
        self.read_gate.take();
        for request in [41, 42] {
            self.report_result(request)?;
        }
        let deadline = Instant::now() + DEADLINE;
        while self.reports.dropped.lock().unwrap().len() < 2 {
            self.pump()?;
            require(
                Instant::now() < deadline,
                "completed report request owners not retired",
            )?;
            thread::sleep(Duration::from_millis(2));
        }
        eprintln!(
            "pre-fix shared-pool reachability model: engine6+watch2+drag2+spare1+report2=13; logical service readiness complete; no report gates held"
        );
        Ok((snapshot, listings))
    }
    fn report_result(&mut self, request: u64) -> TestResult<()> {
        self.report_result_until(request, Instant::now() + DEADLINE)
    }
    fn report_result_until(&mut self, request: u64, deadline: Instant) -> TestResult<()> {
        let Event::DiagnosticTextReady { result, .. } = self.event_until(
            |e| matches!(e, Event::DiagnosticTextReady { request: r, .. } if r.0 == request),
            deadline,
        )?
        else {
            unreachable!()
        };
        let report = result.map_err(|e| format!("report failed: {e:?}"))?;
        require(
            report.text == "POOL_REAL_REPORT\n" && !report.truncated,
            "report content mismatch",
        )
    }
    fn open_viewer(&mut self, files: &Fixtures, listing: &Listing, side: usize) -> TestResult<()> {
        self.start_viewer(listing, &files.files[side])?;
        self.viewer_frame(side, 400)
    }

    fn start_viewer(&mut self, listing: &Listing, focused: &Path) -> TestResult<()> {
        let session = self.viewer_service.as_ref().unwrap().open(
            ViewerLaunch {
                associations: Arc::default(),
                preferences: Arc::default(),
                generated: None,
                archive: None,
                tree: None,
                find: None,
                entries: listing.entries.clone(),
                view: listing.index.clone(),
                focused: focused.to_owned(),
                basic: true,
                disabled_plugins: Arc::from([]),
            },
            appearance(),
            geometry(400),
        )?;
        self.viewers.push(session);
        Ok(())
    }
    fn viewer_frame(&mut self, index: usize, width: u32) -> TestResult<()> {
        self.viewer_frame_until(index, width, Instant::now() + DEADLINE)
    }
    fn viewer_frame_until(
        &mut self,
        index: usize,
        width: u32,
        deadline: Instant,
    ) -> TestResult<()> {
        loop {
            require(Instant::now() < deadline, "viewer frame deadline elapsed")?;
            self.pump()?;
            if let Some(result) = self.viewers[index].mailbox.take_open_result() {
                result?;
            }
            for output in self.viewers[index].mailbox.take() {
                match output {
                    ViewerOutput::Frame(frame) if frame.width == width => {
                        return require(
                            frame.image.is_none() && frame.accessible_text.contains("POOL_NATIVE_"),
                            "viewer text proof missing",
                        );
                    }
                    ViewerOutput::Closed => return Err("viewer closed before real frame".into()),
                    _ => {}
                }
            }
            require(Instant::now() < deadline, "viewer frame timed out")?;
            thread::sleep(Duration::from_millis(2));
        }
    }
    fn open_compare(
        &mut self,
        files: &Fixtures,
        snapshot: &FileComparatorSnapshot,
    ) -> TestResult<()> {
        require(
            self.viewers.len() + self.compares.len() < 4,
            "combined window bound exceeded",
        )?;
        self.start_compare(files, snapshot)
    }

    fn start_compare(
        &mut self,
        files: &Fixtures,
        snapshot: &FileComparatorSnapshot,
    ) -> TestResult<()> {
        let session = self
            .compare_service
            .as_ref()
            .unwrap()
            .open(
                FileCompareLaunch {
                    sources: files
                        .files
                        .iter()
                        .cloned()
                        .map(FileCompareSource::Focused)
                        .collect(),
                    disabled_plugins: Arc::from([]),
                    comparators: snapshot.preferences.clone(),
                    comparator_policy: snapshot.id,
                },
                appearance(),
                geometry(400),
            )
            .map_err(|e| e.to_string())?;
        self.compares.push(session);
        Ok(())
    }
    fn compare_ready(&self, index: usize, revision: Option<u64>) -> TestResult<bool> {
        match self.compares[index].mailbox.take() {
            Some(midnight_compare_engine::Output::Frame { frame, .. }) => {
                require(
                    !frame.image && frame.accessible_text.contains("POOL_NATIVE_"),
                    "compare text proof missing",
                )?;
                Ok(
                    frame.identity == midnight_compare_engine::IdentityVerdict::Different
                        && revision.is_none_or(|revision| frame.geometry_revision == revision),
                )
            }
            Some(midnight_compare_engine::Output::ChooseFiles(choice)) => {
                Err(format!("comparison refused: {}", choice.message))
            }
            Some(midnight_compare_engine::Output::ExternalStarted { .. }) => {
                Err("BuiltInText unexpectedly launched external".into())
            }
            _ => Ok(false),
        }
    }
    fn compare_frame(&mut self, index: usize, revision: Option<u64>) -> TestResult<()> {
        self.compare_frame_until(index, revision, Instant::now() + DEADLINE)
    }
    fn compare_frame_until(
        &mut self,
        index: usize,
        revision: Option<u64>,
        deadline: Instant,
    ) -> TestResult<()> {
        loop {
            require(Instant::now() < deadline, "compare frame deadline elapsed")?;
            self.pump()?;
            if self.compare_ready(index, revision)? {
                return Ok(());
            }
            require(Instant::now() < deadline, "compare frame timed out")?;
            thread::sleep(Duration::from_millis(2));
        }
    }
    fn close_viewers(&mut self) {
        for session in &self.viewers {
            session.close();
        }
        self.viewers.clear();
    }
    fn progress(
        &mut self,
        revision: u64,
        compare_ready: &mut bool,
        save_ready: &mut bool,
    ) -> TestResult<()> {
        if !*compare_ready {
            *compare_ready = self.compare_ready(1, None)?;
        }
        if !*save_ready
            && let Some(Event::PreferencesSaved { result, .. }) = self.take(
                |e| matches!(e, Event::PreferencesSaved { revision: r, .. } if r.0 == revision),
            )?
        {
            let snapshot = result.map_err(|e| format!("pressure save failed: {e:?}"))?;
            require(
                *snapshot.preferences == preferences().file_comparators,
                "pressure acknowledgement raw preferences differ",
            )?;
            let current = self.compare_service.as_ref().unwrap().comparator_snapshot();
            require(
                snapshot.id == current.id
                    && Arc::ptr_eq(&snapshot.preferences, &current.preferences),
                "pressure save publication incoherent",
            )?;
            *save_ready = true;
        }
        Ok(())
    }
    fn reproduce(&mut self, files: &Fixtures) -> TestResult<bool> {
        let (snapshot, listings) = self.prepare_residents(files)?;
        self.open_viewer(files, &listings[0], 0)?;
        self.open_viewer(files, &listings[1], 1)?;
        self.open_compare(files, &snapshot)?;
        self.compare_frame(0, None)?;
        let saved_before = self.settings.prepared_entered.load(Ordering::SeqCst);
        let completed_before = self.settings.prepared_complete.load(Ordering::SeqCst);
        // One absolute pressure budget includes command admission and all three
        // live-command proofs; none can postpone the start of the progress clock.
        let pressure_started = Instant::now();
        let pressure_deadline = pressure_started + DEADLINE;
        self.send_until(
            Command::SavePreferences {
                revision: PreferencesRevision(3),
                preferences: Box::new(preferences()),
            },
            pressure_deadline,
        )?;
        self.open_compare(files, &snapshot)?; // Fourth total window; second Compare is legal.
        require(
            self.viewers.len() + self.compares.len() == 4,
            "four-window test setup failed",
        )?;
        // Existing workers and Engine dispatch must still execute real work. No
        // assumption that SourceChanged/Closed means a thread has exited.
        require(
            self.viewers[0].send(ViewerInput::Resize(geometry(412))),
            "viewer resize not admitted",
        )?;
        require(
            self.compares[0]
                .send(midnight_compare_engine::Input::Resize(geometry(416), 77))
                .is_ok(),
            "compare resize not admitted",
        )?;
        self.send_until(
            Command::ReadDiagnosticText {
                request: DesktopRequestId(43),
                content: files.report(None, self.reports.clone())?,
                cancel: Cancellation::default(),
            },
            pressure_deadline,
        )?;
        self.viewer_frame_until(0, 412, pressure_deadline)?;
        self.compare_frame_until(0, Some(77), pressure_deadline)?;
        self.report_result_until(43, pressure_deadline)?;
        let mut save_ready = false;
        let mut compare_ready = false;
        while Instant::now() < pressure_deadline && !(save_ready && compare_ready) {
            self.progress(3, &mut compare_ready, &mut save_ready)?;
            thread::sleep(Duration::from_millis(2));
        }
        let failed_before_rescue = !(save_ready && compare_ready);
        let entered_now = self.settings.prepared_entered.load(Ordering::SeqCst);
        let completed_now = self.settings.prepared_complete.load(Ordering::SeqCst);
        let save_phase = if save_ready {
            "acknowledged"
        } else if entered_now == saved_before {
            "callback-not-entered (consistent with blocking-pool admission starvation)"
        } else if completed_now == completed_before {
            "callback-entered-but-not-completed"
        } else {
            "callback-completed-but-acknowledgement-missing"
        };
        eprintln!(
            "pre-rescue: progress_failed={failed_before_rescue} save_phase={save_phase} compare_ready={compare_ready} prepared_entered_before={saved_before} prepared_entered_now={entered_now} prepared_complete_before={completed_before} prepared_complete_now={completed_now} windows=4 pressure_elapsed_ms={}",
            pressure_started.elapsed().as_millis()
        );
        if failed_before_rescue {
            // Preserve the failure while releasing both Viewer windows.
            self.close_viewers();
            let deadline = Instant::now() + DEADLINE;
            while Instant::now() < deadline && !(save_ready && compare_ready) {
                self.progress(3, &mut compare_ready, &mut save_ready)?;
                thread::sleep(Duration::from_millis(2));
            }
            require(
                save_ready && compare_ready,
                "two-Viewer rescue did not restore real save and Compare progress",
            )?;
            eprintln!(
                "rescue: actual prepared acknowledgement and second Compare frame observed; original failure remains latched"
            );
        }
        let entered_after = self.settings.prepared_entered.load(Ordering::SeqCst);
        let completed_after = self.settings.prepared_complete.load(Ordering::SeqCst);
        require(
            entered_after == saved_before + 1 && completed_after == completed_before + 1,
            "pressure save did not enter and complete exactly one prepared callback",
        )?;
        require(
            self.settings.value.lock().unwrap().file_comparators == preferences().file_comparators,
            "pressure persisted raw preferences differ",
        )?;
        eprintln!(
            "pressure save verified: prepared_entered={entered_after} prepared_complete={completed_after}; exactly one prepared callback and raw publication coherent"
        );
        require(
            self.settings.unprepared.load(Ordering::SeqCst) == 0,
            "unprepared settings path used",
        )?;
        require(
            self.edges.drag_starts.load(Ordering::SeqCst) == 0,
            "unexpected native StartDrag invocation",
        )?;
        Ok(failed_before_rescue)
    }
    fn cleanup(&mut self, deadline: Instant) -> TestResult<()> {
        if self.cleaned {
            return Ok(());
        }
        self.cleaned = true;
        self.read_gate.take(); // Release real report callbacks even on setup failure.
        self.close_viewers();
        for session in &self.compares {
            session.close();
        }
        self.compares.clear();
        self.host_lifetime.cancel();
        let mut errors = Vec::new();
        if let Some(spare) = self.spare.as_ref() {
            spare.stop_spare();
            let result = self.runtime().block_on(async {
                tokio::time::timeout(
                    deadline.saturating_duration_since(Instant::now()),
                    spare.wait(),
                )
                .await
            });
            if !matches!(result, Ok(Ok(()))) {
                errors.push("image spare exit/cleanup unconfirmed".to_string());
            }
        }
        self.viewer_service.take();
        self.compare_service.take();
        let transient = self.runtime().handle().clone();
        let viewer_exit = self
            .viewer_owner
            .take()
            .expect("Viewer resident owner")
            .shutdown(&transient);
        let compare_exit = self
            .compare_owner
            .take()
            .expect("Compare resident owner")
            .shutdown(&transient);
        if let Err(error) = self.send_until(Command::Shutdown, deadline) {
            errors.push(error);
        }
        while self.engine.as_ref().is_some_and(|task| !task.is_finished())
            && Instant::now() < deadline
        {
            while self.events.try_recv().is_ok() {}
            self.pending.clear();
            thread::sleep(Duration::from_millis(2));
        }
        if let Some(task) = self.engine.take() {
            if task.is_finished() {
                if self.runtime().block_on(task).is_err() {
                    errors.push("Engine join failed".into());
                }
            } else {
                task.abort();
                errors.push("Engine did not join before cleanup deadline".into());
            }
        }
        while self.events.try_recv().is_ok() {}
        self.pending.clear();
        let (viewer, compare) = self.runtime().block_on(async {
            let deadline = tokio::time::Instant::from_std(deadline);
            tokio::join!(
                tokio::time::timeout_at(deadline, viewer_exit),
                tokio::time::timeout_at(deadline, compare_exit),
            )
        });
        for (domain, result) in [("Viewer", viewer), ("Compare", compare)] {
            match result {
                Ok(Ok(())) => eprintln!("{domain} resident pool exited and joined"),
                Ok(Err(error)) => {
                    errors.push(format!("{domain} resident pool shutdown failed: {error}"));
                }
                Err(_) => errors.push(format!(
                    "{domain} resident pool exit unconfirmed at the cleanup deadline"
                )),
            }
        }
        // CLEANUP ONLY: never contributes pressure to reproduce the defect.
        // Sixteen simultaneous acknowledgements prove transient pool capacity.
        // The gate is released on EVERY return/unwind, including timeout.
        if errors.is_empty() {
            let gate = Arc::new(ReadGate::default());
            let release = ReleaseGate(gate.clone());
            let (send, receive) = mpsc::sync_channel(BLOCKING_LIMIT);
            let mut joins = Vec::new();
            for _ in 0..BLOCKING_LIMIT {
                let gate = gate.clone();
                let send = send.clone();
                joins.push(self.runtime().spawn_blocking(move || {
                    let _ = send.send(thread::current().id());
                    gate.wait_until_released();
                }));
            }
            drop(send);
            let mut threads = Vec::new();
            for _ in 0..BLOCKING_LIMIT {
                match receive.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(id) => threads.push(id),
                    Err(_) => {
                        errors.push(
                            "cleanup pool census could not enter sixteen concurrent slots".into(),
                        );
                        break;
                    }
                }
            }
            drop(release);
            let distinct: HashSet<_> = threads.into_iter().collect();
            if distinct.len() != BLOCKING_LIMIT {
                errors.push("cleanup pool census not sixteen distinct threads".into());
            }
            let mut unconfirmed = 0;
            for join in joins {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let result = self
                    .runtime()
                    .block_on(async { tokio::time::timeout(remaining, join).await });
                if !matches!(result, Ok(Ok(()))) {
                    unconfirmed += 1;
                }
            }
            if unconfirmed != 0 {
                errors.push(format!(
                    "cleanup probes: {unconfirmed} joins unconfirmed after shared deadline/panic"
                ));
            }
        }
        let watch = self.edges.watches.lock().unwrap();
        let watch_drops = self.edges.watch_drops.lock().unwrap();
        if watch.len() != watch_drops.len()
            || watch_drops
                .iter()
                .any(|(_, dropped)| *dropped == thread::current().id())
        {
            errors.push("watch cleanup count/thread mismatch".into());
        }
        let drag = self.edges.drag_opens.lock().unwrap();
        let drag_drops = self.edges.drag_drops.lock().unwrap();
        if drag.len() != drag_drops.len() || drag_drops.iter().any(|(a, b)| a != b) {
            errors.push("drag cleanup count/thread mismatch".into());
        }
        if self.edges.drag_starts.load(Ordering::SeqCst) != 0 {
            errors.push("unexpected native StartDrag invocation".into());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.cleanup(Instant::now() + DEADLINE)
        })) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => eprintln!("production-pool cleanup FAILED: {error}"),
            Err(_) => eprintln!("production-pool cleanup PANICKED; cleanup is unconfirmed"),
        }
        drop(self.viewer_owner.take());
        drop(self.compare_owner.take());
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(2));
        }
    }
}

#[test]
#[ignore = "SERIAL: production2/16 scheduling; qualified installed native/text providers; private MIDNIGHT_POOL_REPRO_WORK; no OS clipboard"]
fn production_pool_preserves_settings_and_fourth_window_progress_with_idle_windows() {
    let files = Fixtures::new().expect("owned production pool fixtures");
    eprintln!("private repro evidence root={}", files.root.display());
    let mut harness = Harness::new().expect("production runtime composition");
    let outcome = harness.reproduce(&files);
    // One shared budget covers spare, Engine, both external resident pools,
    // transient pool and both installed reader release checks.
    let cleanup_deadline = Instant::now() + DEADLINE;
    let cleanup = harness.cleanup(cleanup_deadline);
    let released = files.check_readers_released(cleanup_deadline);
    cleanup.expect("real workers, Engine, image spare and external resident pools must exit");
    released.expect("installed capture readers must release");
    let progress_failed = outcome.expect("repro setup/live proof/recovery failed");
    assert!(
        !progress_failed,
        "PRODUCTION PROGRESS FAILURE: real prepared save or legal fourth window failed before rescue; see pre-rescue save phase and callback counters; two-Viewer recovery and bounded cleanup were verified"
    );
}
