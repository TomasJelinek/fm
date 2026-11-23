//! Controlled providers with the production Engine and installed presenter callbacks.
// Each integration crate exercises a different subset of this shared fixture.
#![allow(dead_code)]
#[path = "../../../midnight-explorer/src/command_reclaim.rs"]
mod command_reclaim;
#[path = "modeless.rs"]
pub(super) mod modeless;
#[path = "engine_startup.rs"]
mod startup;
#[path = "engine_teardown.rs"]
mod teardown;
use midnight_core::*;
use midnight_engine::{Engine, reclaim};
use midnight_ui::{
    BeginOwned, DeferredDrop, OperationPresentation, OwnedCommandScope, Presenter,
    install_with_owned, refresh, with_owned_commands,
};
use slint::{
    ComponentHandle, Model,
    platform::{WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::RefCell,
    collections::VecDeque,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{runtime::Runtime, sync::mpsc, task::JoinHandle};

#[derive(Default)]
pub struct Calls {
    pub frozen: AtomicBool,
    pub lists: AtomicUsize,
    pub saves: AtomicUsize,
    pub fail_save: AtomicBool,
    pub navigations: AtomicUsize,
    pub size_reads: Mutex<Vec<String>>,
    /// Hold the next member read for longer than a progress heartbeat, so that the
    /// running copy publishes the member it is reading.
    pub slow_read: AtomicBool,
    children: Vec<String>,
    size_fixture: bool,
}

impl Calls {
    fn size_bytes(&self, name: &str) -> Option<u64> {
        self.size_fixture.then_some(match name {
            "large" => 1_000,
            "small" => 12,
            _ => 0,
        })
    }
}

struct Files(Arc<Calls>);
impl DirectoryService for Files {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        assert!(
            !self.0.frozen.load(Ordering::SeqCst),
            "filter/sort must not enumerate"
        );
        self.0.lists.fetch_add(1, Ordering::SeqCst);
        if path.file_name().is_some_and(|name| name == "folder") {
            for (index, name) in self.0.children.iter().enumerate() {
                if !emit(Entry::new(
                    EntryId(index as u64 + 1),
                    path.join(name),
                    EntryKind::Directory,
                )) {
                    break;
                }
            }
        } else if path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "folder")
            && self.0.size_fixture
        {
            let name = path.file_name().unwrap().to_str().unwrap();
            self.0.size_reads.lock().unwrap().push(name.to_owned());
            if self.0.size_bytes(name).unwrap() > 0 {
                emit(Entry::new(
                    EntryId(50),
                    path.join("payload.bin"),
                    EntryKind::File,
                ));
            }
        } else if !path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "folder")
        {
            emit(Entry::new(
                EntryId(1),
                path.join("folder"),
                EntryKind::Directory,
            ));
            emit(Entry::new(
                EntryId(2),
                path.join("root.txt"),
                EntryKind::File,
            ));
        }
        Ok(())
    }
    fn listing_metadata_complete(&self, _: &Entry) -> bool {
        true
    }
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        if self.0.size_fixture && path.file_name().is_some_and(|name| name == "payload.bin") {
            return Ok(EntryMetadata {
                size: self.0.size_bytes(
                    path.parent()
                        .unwrap()
                        .file_name()
                        .unwrap()
                        .to_str()
                        .unwrap(),
                ),
                ..Default::default()
            });
        }
        panic!("captured metadata is complete")
    }
}
impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(vec![])
    }
}
impl StorageService for Files {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            total: 100,
            available: 50,
        })
    }
}
struct Settings(Arc<Calls>);
impl SettingsService for Settings {
    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        Ok(UserPreferences::default())
    }
    fn save_preferences(&self, _: UserPreferences) -> Result<(), FsError> {
        self.0.saves.fetch_add(1, Ordering::SeqCst);
        if self.0.fail_save.load(Ordering::SeqCst) {
            Err(FsError::from_io(
                "save settings",
                Path::new(""),
                std::io::ErrorKind::PermissionDenied.into(),
            ))
        } else {
            Ok(())
        }
    }

    fn load_preferences_for_acceptance(&self) -> Result<UserPreferences, FsError> {
        self.load_preferences()
    }

    fn save_preferences_prepared(
        &self,
        preferences: UserPreferences,
        proof: ComparatorValidationProof<'_>,
    ) -> Result<(), FsError> {
        if !proof.matches(&preferences.file_comparators) {
            return Err(FsError::from_io(
                "save settings",
                Path::new(""),
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Comparator validation proof does not match preferences.",
                ),
            ));
        }
        self.save_preferences(preferences)
    }
}

struct Archive(Arc<Calls>);
impl std::fmt::Debug for Archive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FilterArchive")
    }
}
impl ArchiveMount for Archive {
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        // This fixture owns only Calls' counters, strings and finite host
        // collections. It has no native, provider or asynchronous cleanup tail.
        if let Some(owner) = Arc::into_inner(self) {
            drop(owner);
        }
        Ok(())
    }
    fn id(&self) -> u64 {
        77
    }
    fn list(&self, directory: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        assert!(
            !self.0.frozen.load(Ordering::SeqCst),
            "filter/sort must not list archive members"
        );
        self.0.lists.fetch_add(1, Ordering::SeqCst);
        let specs: Vec<(u64, &str, EntryKind)> = if directory == 1 {
            vec![
                (2, "folder", EntryKind::Directory),
                (3, "root.txt", EntryKind::File),
            ]
        } else if directory == 2 {
            self.0
                .children
                .iter()
                .enumerate()
                .map(|(index, name)| (index as u64 + 4, name.as_str(), EntryKind::Directory))
                .collect()
        } else if self.0.size_fixture {
            let name = &self.0.children[(directory - 4) as usize];
            self.0.size_reads.lock().unwrap().push(name.clone());
            if self.0.size_bytes(name).unwrap() > 0 {
                vec![(100 + directory, "payload.bin", EntryKind::File)]
            } else {
                vec![]
            }
        } else {
            vec![]
        };
        Ok(specs
            .iter()
            .map(|(entry, name, kind)| ArchiveNode {
                id: VirtualEntryId {
                    mount: 77,
                    entry: *entry,
                },
                parent: directory,
                raw_name: Arc::from(name.as_bytes()),
                display_name: (*name).into(),
                kind: *kind,
                size: if self.0.size_fixture && directory >= 4 {
                    self.0
                        .size_bytes(&self.0.children[(directory - 4) as usize])
                } else {
                    Some(5)
                },
                modified: None,
                readable: *kind == EntryKind::File,
                access: Default::default(),
            })
            .collect())
    }
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        _: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        if self.0.size_fixture && entry >= 104 {
            if self.0.slow_read.swap(false, Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(700));
            }
            let name = &self.0.children[(entry - 104) as usize];
            let size = self.0.size_bytes(name).unwrap();
            return Ok(vec![
                42;
                size.saturating_sub(offset).min(u64::from(length))
                    as usize
            ]);
        }
        panic!("no extraction or content reads")
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ClipboardStep {
    stage: &'static str,
    stamp: Option<TreeViewStamp>,
    // Allocation identity only: no owner retained and no pointer dereferenced.
    projection_ptr: Option<usize>,
    request: Option<u64>,
    selection: Option<u64>,
    selected: Option<usize>,
    cancelled: Option<bool>,
    accepted: Option<bool>,
    ok: Option<bool>,
    active_pane: Option<i32>,
    focused_pane: Option<i32>,
    notice_generation: Option<i32>,
    modal: Option<bool>,
    blocked: Option<bool>,
    scope_claims: Option<usize>,
    scope_denials: Option<usize>,
}

impl ClipboardStep {
    fn command(command: &Command) -> Option<Self> {
        Some(match command {
            Command::Tree(tree) => match tree.as_ref() {
                TreeCommand::Commit(view) => Self {
                    stage: "commit_send",
                    stamp: Some(view.stamp),
                    projection_ptr: Some(Arc::as_ptr(view) as usize),
                    ..Self::default()
                },
                TreeCommand::Select(request) => Self {
                    stage: "select_send",
                    stamp: Some(request.view.stamp),
                    projection_ptr: Some(Arc::as_ptr(&request.view) as usize),
                    selection: Some(request.revision.0),
                    ..Self::default()
                },
                TreeCommand::Clipboard(request) => Self {
                    stage: "capture_send",
                    stamp: Some(request.view.stamp),
                    projection_ptr: Some(Arc::as_ptr(&request.view) as usize),
                    request: Some(request.request.0),
                    selection: Some(request.selection_revision.0),
                    selected: Some(request.selection.len()),
                    cancelled: Some(request.cancel.is_cancelled()),
                    ..Self::default()
                },
                _ => return None,
            },
            Command::CopyCapturedFiles {
                request, cancel, ..
            } => Self {
                stage: "publish_send",
                request: Some(request.0),
                cancelled: Some(cancel.is_cancelled()),
                ..Self::default()
            },
            _ => return None,
        })
    }

    fn event(event: &Event) -> Option<Self> {
        Some(match event {
            Event::Tree(tree) => match tree.as_ref() {
                TreeEvent::Published(publication) | TreeEvent::Reprojected { publication, .. } => {
                    Self {
                        stage: "view",
                        stamp: Some(publication.view.stamp),
                        projection_ptr: Some(Arc::as_ptr(&publication.view) as usize),
                        selection: Some(publication.selection_revision.0),
                        selected: Some(publication.selection.len()),
                        ..Self::default()
                    }
                }
                TreeEvent::SelectionChanged {
                    view,
                    revision,
                    selection,
                } => Self {
                    stage: "select_reply",
                    stamp: Some(view.stamp),
                    projection_ptr: Some(Arc::as_ptr(view) as usize),
                    selection: Some(revision.0),
                    selected: Some(selection.len()),
                    ok: Some(true),
                    ..Self::default()
                },
                TreeEvent::SelectionFailed {
                    stamp, revision, ..
                } => Self {
                    stage: "select_reply",
                    stamp: Some(*stamp),
                    selection: Some(revision.0),
                    ok: Some(false),
                    ..Self::default()
                },
                TreeEvent::Clipboard {
                    stamp,
                    request,
                    result,
                } => Self {
                    stage: "capture_reply",
                    stamp: Some(*stamp),
                    request: Some(request.0),
                    ok: Some(result.is_ok()),
                    ..Self::default()
                },
                _ => return None,
            },
            Event::DesktopActionFinished { request, result } => Self {
                stage: match result {
                    Ok(DesktopActionResult::ClipboardCopied) => "desktop_copied",
                    Err(_) => "desktop_error",
                    _ => "desktop_other",
                },
                request: Some(request.0),
                ok: Some(result.is_ok()),
                ..Self::default()
            },
            _ => return None,
        })
    }
}

#[derive(Debug)]
struct ClipboardTrace {
    started: Instant,
    parallelism: Option<usize>,
    // Each array is ordered selection, capture, publication.
    attempts: [usize; 3],
    refusals: [usize; 3],
    replies: [usize; 3],
    scope_claims: usize,
    scope_denials: usize,
    // Preserve the first chord evidence even when later stages fill the ring.
    before_chord: Option<(u128, ClipboardStep)>,
    after_chord: Option<(u128, ClipboardStep)>,
    steps: VecDeque<(u128, ClipboardStep)>,
}

impl ClipboardTrace {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            parallelism: std::thread::available_parallelism()
                .ok()
                .map(|value| value.get()),
            attempts: [0; 3],
            refusals: [0; 3],
            replies: [0; 3],
            scope_claims: 0,
            scope_denials: 0,
            before_chord: None,
            after_chord: None,
            steps: VecDeque::with_capacity(16),
        }
    }

    fn record(&mut self, step: ClipboardStep) {
        let command = match step.stage {
            "select_send" => Some(0),
            "capture_send" => Some(1),
            "publish_send" => Some(2),
            _ => None,
        };
        if let Some(index) = command {
            self.attempts[index] += 1;
            self.refusals[index] += usize::from(step.accepted == Some(false));
        }
        let reply = match step.stage {
            "select_reply" => Some(0),
            "capture_reply" => Some(1),
            "desktop_copied" | "desktop_error" | "desktop_other" => Some(2),
            _ => None,
        };
        if let Some(index) = reply {
            self.replies[index] += 1;
        }
        if self.steps.len() == 16 {
            self.steps.pop_front();
        }
        let record = (self.started.elapsed().as_millis(), step);
        match step.stage {
            "before_copy_chord" if self.before_chord.is_none() => self.before_chord = Some(record),
            "after_copy_chord" if self.after_chord.is_none() => self.after_chord = Some(record),
            _ => {}
        }
        self.steps.push_back(record);
    }
}

struct Routing {
    calls: Arc<Calls>,
    requests: Rc<RefCell<Vec<TreeReprojectRequest>>>,
    rejected: Rc<RefCell<bool>>,
    clipboard_trace: Rc<RefCell<ClipboardTrace>>,
}

impl Routing {
    fn prepare(&self, command: &mut Command) -> bool {
        if matches!(
            &*command,
            Command::Navigate { .. } | Command::ArchiveNavigate { .. }
        ) || matches!(&*command, Command::Tree(tree) if matches!(tree.as_ref(), TreeCommand::Navigate { .. }))
        {
            self.calls.navigations.fetch_add(1, Ordering::SeqCst);
        }
        // Supply a controlled provider mount for this fixture's typed saved
        // address. Presenter navigation, Engine listing and tree workers run normally.
        if let Command::ArchiveNavigate {
            action: ArchiveAction::List(location),
            ..
        } = command
            && location.address.provider == "test.archive"
            && location.mount.is_none()
        {
            location.mount = Some(Arc::new(Archive(self.calls.clone())));
            location.directory = 1;
            location.trail = vec![(1, "fixture.zip".into())];
        }
        if let Command::Tree(tree) = &*command
            && let TreeCommand::Reproject(request) = tree.as_ref()
        {
            if *self.rejected.borrow() {
                return false;
            }
            self.requests.borrow_mut().push(request.clone());
        }
        true
    }
}

struct ObservedScope {
    inner: Box<dyn OwnedCommandScope>,
    routing: Rc<Routing>,
}

impl OwnedCommandScope for ObservedScope {
    fn send(&self, mut command: Command) -> bool {
        let metadata = ClipboardStep::command(&command);
        let accepted = if !self.routing.prepare(&mut command) {
            self.inner.retire(Box::new(command));
            false
        } else {
            self.inner.send(command)
        };
        if let Some(mut metadata) = metadata {
            metadata.accepted = Some(accepted);
            self.routing.clipboard_trace.borrow_mut().record(metadata);
        }
        accepted
    }

    fn retire(&self, value: DeferredDrop) {
        self.inner.retire(value);
    }
}

pub struct Harness {
    pub ui: super::ui::TestUi,
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    pub presenter: Rc<RefCell<Presenter>>,
    pub calls: Arc<Calls>,
    pub requests: Rc<RefCell<Vec<TreeReprojectRequest>>>,
    pub reject_reproject: Rc<RefCell<bool>>,
    pub view: Option<Arc<TreeProjection>>,
    pub views: [Option<Arc<TreeProjection>>; 2],
    pub selection: Option<Arc<TreeSelectionSnapshot>>,
    pub sizes: Option<Arc<TreeSizeSnapshot>>,
    pub size_finished: usize,
    pub copy_progress: Vec<Arc<CopyProgress>>,
    /// The Jobs rows of copies, as the window showed them after each copy progress.
    pub copy_rows: Vec<midnight_ui::OperationRow>,
    copy_applied: bool,
    pub clipboard_publications: usize,
    clipboard_trace: Rc<RefCell<ClipboardTrace>>,
    pub watch_hints: [usize; 2],
    hold_maintenance_hints: bool,
    held_tree_hints: [Option<TreeViewStamp>; 2],
    held_directory_hints: [Option<NavigationId>; 2],
    /// The latest hints, refreshes and tree publications, for failure messages.
    maintenance: VecDeque<String>,
    pub send: Rc<dyn Fn(Command) -> bool>,
    pub pane: PaneId,
    runtime: Option<Runtime>,
    events: mpsc::Receiver<Event>,
    engine: Option<JoinHandle<()>>,
    commands: midnight_engine::CommandSender,
    begin_owned: BeginOwned,
    command_reclaim: command_reclaim::CommandReclaim,
    retire_queue: mpsc::Sender<reclaim::Retired>,
    watching: bool,
    committed: [Option<NavigationId>; 2],
}

struct NativeServices {
    path: PathBuf,
    directory: Arc<dyn FileSystem>,
    watch: Option<Arc<dyn WatchService>>,
    clipboard: Option<Arc<dyn ClipboardService>>,
    commands: Option<(Arc<dyn UserCommandService>, Vec<UserCommand>)>,
    hold_maintenance_hints: bool,
}

type InstallTransfer = Box<dyn FnOnce(&mut Engine)>;

fn audited_transfer<T: AuditedSynchronousTransferService + 'static>(
    transfer: Arc<T>,
) -> InstallTransfer {
    Box::new(move |engine| {
        engine
            .configure_audited_transfers()
            .unwrap()
            .install(transfer);
    })
}

impl Harness {
    pub fn new(pane: PaneId, archive: bool) -> Self {
        Self::with_children(pane, archive, &["z.txt", "a.bin"])
    }

    pub fn with_children(pane: PaneId, archive: bool, children: &[&str]) -> Self {
        Self::with_fixture(pane, archive, children, false, None, None)
    }

    pub fn with_sizes(pane: PaneId, archive: bool) -> Self {
        let h = Self::with_fixture(
            pane,
            archive,
            &["large", "small", "empty"],
            true,
            None,
            None,
        );
        h.calls.frozen.store(false, Ordering::SeqCst);
        h.calls.size_reads.lock().unwrap().clear();
        h
    }

    pub fn with_transfer<T: AuditedSynchronousTransferService + 'static>(
        pane: PaneId,
        archive: bool,
        transfer: Arc<T>,
    ) -> Self {
        let h = Self::with_fixture(
            pane,
            archive,
            &["large", "small", "empty"],
            true,
            Some(audited_transfer(transfer)),
            None,
        );
        h.calls.frozen.store(false, Ordering::SeqCst);
        h
    }

    pub fn with_native_transfer<T: AuditedSynchronousTransferService + 'static>(
        pane: PaneId,
        path: PathBuf,
        directory: Arc<dyn FileSystem>,
        transfer: Arc<T>,
    ) -> Self {
        Self::with_fixture(
            pane,
            false,
            &["large", "small", "empty"],
            true,
            Some(audited_transfer(transfer)),
            Some(NativeServices {
                path,
                directory,
                watch: None,
                clipboard: None,
                commands: None,
                hold_maintenance_hints: false,
            }),
        )
    }

    /// `hold_maintenance_hints`: see [`Self::release_maintenance_hints`]; a test
    /// that captures exact tree state for a transfer holds them until its terminal.
    pub fn with_native_watch_transfer<T: AuditedSynchronousTransferService + 'static>(
        pane: PaneId,
        path: PathBuf,
        directory: Arc<dyn FileSystem>,
        transfer: Arc<T>,
        watch: Arc<dyn WatchService>,
        hold_maintenance_hints: bool,
    ) -> Self {
        Self::with_fixture(
            pane,
            false,
            &["large", "small", "empty"],
            true,
            Some(audited_transfer(transfer)),
            Some(NativeServices {
                path,
                directory,
                watch: Some(watch),
                clipboard: None,
                commands: None,
                hold_maintenance_hints,
            }),
        )
    }

    pub fn with_native_clipboard_transfer<T: AuditedSynchronousTransferService + 'static>(
        pane: PaneId,
        path: PathBuf,
        directory: Arc<dyn FileSystem>,
        transfer: Arc<T>,
        watch: Arc<dyn WatchService>,
        clipboard: Arc<dyn ClipboardService>,
        hold_maintenance_hints: bool,
    ) -> Self {
        Self::with_fixture(
            pane,
            false,
            &["large", "small", "empty"],
            true,
            Some(audited_transfer(transfer)),
            Some(NativeServices {
                path,
                directory,
                watch: Some(watch),
                clipboard: Some(clipboard),
                commands: None,
                hold_maintenance_hints,
            }),
        )
    }

    pub fn with_native_user_commands(
        pane: PaneId,
        path: PathBuf,
        native: Arc<midnight_platform::LocalFileSystem>,
        commands: Arc<dyn UserCommandService>,
        configuration: Vec<UserCommand>,
    ) -> Self {
        Self::with_fixture(
            pane,
            false,
            &["large", "small", "empty"],
            true,
            Some(audited_transfer(native.clone())),
            Some(NativeServices {
                path,
                directory: native,
                watch: None,
                clipboard: None,
                commands: Some((commands, configuration)),
                hold_maintenance_hints: false,
            }),
        )
    }

    fn with_fixture(
        pane: PaneId,
        archive: bool,
        children: &[&str],
        size_fixture: bool,
        transfer: Option<InstallTransfer>,
        native: Option<NativeServices>,
    ) -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
        let window = midnight_ui::MainWindow::new().unwrap();
        window.set_preferences_ready(true);
        midnight_ui::install_file_keys(&window);
        window.show().unwrap();
        window
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        let owner_adapter = windows.borrow()[0].clone();
        let mut ui = super::ui::TestUi {
            window,
            native: owner_adapter,
            pixels: Vec::new(),
            width: 1340,
            height: 820,
        };
        ui.resize(1340, 820);
        let calls = Arc::new(Calls {
            children: children.iter().map(|name| (*name).to_owned()).collect(),
            size_fixture,
            ..Calls::default()
        });
        let native_listing = native.is_some();
        let NativeServices {
            path,
            directory,
            watch,
            clipboard,
            commands: user_commands,
            hold_maintenance_hints,
        } = native.unwrap_or_else(|| NativeServices {
            path: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tree-filter-bridge"),
            directory: Arc::new(Files(calls.clone())),
            watch: None,
            clipboard: None,
            commands: None,
            hold_maintenance_hints: false,
        });
        let watching = watch.is_some();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();

        let (events_send, events) = mpsc::channel(64);
        let copying = transfer.is_some();
        let mut engine = Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            directory,
            events_send,
        );
        engine
            .configure_settings()
            .unwrap()
            .install(Arc::new(Settings(calls.clone())));
        if let Some(install_transfer) = transfer {
            install_transfer(&mut engine);
        }
        if let Some(watch) = watch {
            engine.configure_watch().unwrap().install(watch);
        }
        if let Some(clipboard) = clipboard {
            engine.configure_clipboard().unwrap().install(clipboard);
        }
        let command_configuration = if let Some((service, configuration)) = user_commands {
            engine.configure_user_commands().unwrap().install(service);
            configuration
        } else {
            Vec::new()
        };
        let (commands, receive) = engine.command_channel(64).unwrap();
        let retire_queue = reclaim::start(runtime.handle(), Arc::new(|| {}));
        let command_reclaim = command_reclaim::CommandReclaim::new(retire_queue.clone());
        let engine = runtime.spawn(engine.run(receive));
        let presenter = Rc::new(RefCell::new(Presenter::new(path.clone(), path.clone())));
        let requests = Rc::new(RefCell::new(Vec::new()));
        let reject_reproject = Rc::new(RefCell::new(false));
        let clipboard_trace = Rc::new(RefCell::new(ClipboardTrace::new()));
        let routing = Rc::new(Routing {
            calls: calls.clone(),
            requests: requests.clone(),
            rejected: reject_reproject.clone(),
            clipboard_trace: clipboard_trace.clone(),
        });
        let begin = command_reclaim.factory(commands.clone());
        let observed_routing = routing.clone();
        let trace_begin = clipboard_trace.clone();
        let begin_owned: BeginOwned = Rc::new(move || {
            let Some(inner) = begin() else {
                let mut trace = trace_begin.borrow_mut();
                trace.scope_denials += 1;
                if trace.scope_denials == 1 {
                    trace.record(ClipboardStep {
                        stage: "scope_denied",
                        ..ClipboardStep::default()
                    });
                }
                return None;
            };
            trace_begin.borrow_mut().scope_claims += 1;
            Some(Box::new(ObservedScope {
                inner,
                routing: observed_routing.clone(),
            }))
        });
        let channel = commands.clone();
        let begin_controls = begin_owned.clone();
        // This fallback is for provider-free controls. Installed ownership-bearing
        // actions use begin_owned and the same real typed sender.
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |mut command| {
            if let Some(scope) = begin_controls() {
                return scope.send(command);
            }
            routing.prepare(&mut command) && channel.try_send(command).is_ok()
        });
        install_with_owned(
            &ui.window,
            presenter.clone(),
            send.clone(),
            begin_owned.clone(),
        );
        let startup_tabs = startup::active_tabs(&presenter.borrow());
        let mut h = Self {
            ui,
            windows,
            presenter,
            calls,
            requests,
            reject_reproject,
            view: None,
            views: [None, None],
            selection: None,
            sizes: None,
            size_finished: 0,
            copy_progress: Vec::new(),
            copy_rows: Vec::new(),
            copy_applied: false,
            clipboard_publications: 0,
            clipboard_trace,
            watch_hints: [0; 2],
            hold_maintenance_hints,
            held_tree_hints: [None; 2],
            held_directory_hints: [None; 2],
            maintenance: VecDeque::new(),
            send,
            pane,
            runtime: Some(runtime),
            events,
            engine: Some(engine),
            commands,
            begin_owned,
            command_reclaim,
            retire_queue,
            watching,
            committed: [None; 2],
        };
        // Startup preferences restore both tabs through the installed presenter.
        // Streaming rows are visible before disclosure is available, so wait for
        // the completed listings rather than clicking an inert loading row.
        // A completed presence probe may already replace Unknown with HasFolders.
        let mut preferences = UserPreferences {
            user_commands: Arc::new(command_configuration),
            ..Default::default()
        };
        preferences.tabs[0].paths = vec![path.clone()];
        preferences.tabs[1].paths = vec![path.clone()];
        if copying {
            // A real two-folder transfer keeps the source tree alive. Using the
            // same root in both panes legitimately refreshes that tree on success.
            preferences.tabs[pane.other().index()].paths = vec![path.join("destination")];
        }
        if archive {
            preferences.tabs[pane.index()].archives.insert(
                0,
                ArchiveAddress {
                    provider: "test.archive".into(),
                    format: Some(ArchiveFormat::Zip),
                    container: path.join("fixture.zip"),
                    source_version: vec![1],
                    components: vec![],
                },
            );
        }
        h.apply_event(Event::PreferencesLoaded(Ok(Box::new(
            midnight_core::AcceptedPreferences {
                preferences,
                comparators: midnight_core::FileComparatorSnapshot::default(),
            },
        ))));
        // The Engine's own answer to the installed preferences request is ignored
        // from now on. Wait for the tab restore as well: a restore navigation that
        // the command queue refused is sent again with a later event, and would then
        // replace a pane after these waits.
        h.until(|h| {
            if !startup::tabs_restored(&h.presenter.borrow(), &startup_tabs) {
                return false;
            }
            if native_listing {
                return !h.ui.window.get_left_loading() && !h.ui.window.get_right_loading();
            }
            [h.ui.window.get_left_rows(), h.ui.window.get_right_rows()]
                .iter()
                .all(|rows| {
                    rows.iter()
                        .any(|row| row.name == "folder" && matches!(row.tree_disclosure, 1 | 3))
                })
        });
        h.until(|h| {
            h.rows()
                .iter()
                .any(|row| row.name == "folder" && matches!(row.tree_disclosure, 1 | 3))
        });
        let identity = h
            .rows()
            .iter()
            .find(|row| row.name == "folder")
            .unwrap()
            .identity
            .clone();
        h.ui.window
            .invoke_tree_disclosure(pane.index() as i32, identity);
        // A pane's first check after acceptance can hint a change (a folder
        // created for the fixture can be updated in its parent's index later)
        // and re-read that pane meanwhile. The presenter keeps the disclosure
        // above until its tree is adopted; the caller's next click must not
        // reach a pane that is still loading either.
        h.until(|h| {
            !h.ui.window.get_left_loading()
                && !h.ui.window.get_right_loading()
                && h.view.as_ref().is_some_and(|view| {
                    (if native_listing {
                        children
                            .iter()
                            .all(|name| view.rows.iter().any(|row| row.entry.display_name == *name))
                    } else {
                        view.rows.len() == 2 + children.len()
                    }) && view.rows.iter().any(|row| row.expanded)
                        && view.rows.iter().all(|row| {
                            !matches!(
                                row.disclosure,
                                TreeDisclosure::Unknown | TreeDisclosure::Loading
                            )
                        })
                })
        });
        h.calls.frozen.store(true, Ordering::SeqCst);
        h
    }

    pub fn dialog(&mut self) -> Rc<MinimalSoftwareWindow> {
        // Settle the embedded popup using Main's current layout and drawing
        // buffer; it must not acquire the former native child geometry.
        self.ui.draw();
        let children: Vec<_> = self
            .windows
            .borrow()
            .iter()
            .skip(1)
            .filter(|window| window.window().is_visible())
            .cloned()
            .collect();
        assert_eq!(
            children.len(),
            0,
            "tree transfer confirmation must stay inside the main adapter"
        );
        assert!(
            self.ui.window.get_embedded_modal_open(),
            "the embedded confirmation must gate background application input"
        );
        assert!(
            !self.ui.window.get_dialog_input_blocked(),
            "the main input host must stay enabled for the embedded confirmation"
        );
        let form = self.ui.window.global::<OperationPresentation>();
        assert!(form.get_active() && !form.get_token().is_empty());
        assert_eq!(
            i_slint_core::window::WindowInner::from_pub(self.ui.window.window())
                .active_popups()
                .iter()
                .filter(|popup| !matches!(
                    popup.window_kind,
                    i_slint_core::window::WindowKind::ToolTip
                ))
                .count(),
            1,
            "the confirmation must be the only non-tooltip popup on Main"
        );
        self.ui.native.clone()
    }

    /// Whether an independent tool window is open (it opens after an Engine round trip).
    #[allow(dead_code)]
    pub fn has_tool_window(&self) -> bool {
        self.windows
            .borrow()
            .iter()
            .skip(1)
            .any(|window| window.window().is_visible())
    }

    pub fn tool_window(&self) -> Rc<MinimalSoftwareWindow> {
        let children: Vec<_> = self
            .windows
            .borrow()
            .iter()
            .skip(1)
            .filter(|window| window.window().is_visible())
            .cloned()
            .collect();
        assert_eq!(children.len(), 1, "expected one independent tool window");
        assert!(!self.ui.window.get_dialog_input_blocked());
        children[0].clone()
    }

    pub fn rows(&self) -> Vec<midnight_ui::FileRow> {
        let rows = if self.pane == PaneId::Left {
            self.ui.window.get_left_rows()
        } else {
            self.ui.window.get_right_rows()
        };
        rows.iter().collect()
    }

    pub fn filter_label(&self) -> String {
        if self.pane == PaneId::Left {
            self.ui.window.get_left_filter_label().to_string()
        } else {
            self.ui.window.get_right_filter_label().to_string()
        }
    }

    /// Inspection-only escape hatch: the test caller owns the returned event.
    /// Delayed/inspected events do not qualify production final-owner retirement.
    pub fn receive(&mut self) -> Event {
        self.receive_by(Instant::now() + teardown::UI_WAIT, "any Engine event")
    }

    /// Receive and apply Engine events until one matching `wanted` has been applied,
    /// within one overall [`teardown::UI_WAIT`] (a steady stream of other events
    /// cannot extend it).
    #[track_caller]
    pub fn apply_events_until(&mut self, what: &str, wanted: impl Fn(&Event) -> bool) {
        let deadline = Instant::now() + teardown::UI_WAIT;
        loop {
            let event = self.receive_by(deadline, what);
            let found = wanted(&event);
            self.apply_event(event);
            if found {
                return;
            }
        }
    }

    #[track_caller]
    fn receive_by(&mut self, deadline: Instant, what: &str) -> Event {
        loop {
            if let Some(scope) = (self.begin_owned)() {
                let event = self.events.try_recv();
                drop(scope);
                match event {
                    Ok(event) => return event,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        panic!("Engine event channel closed while waiting for {what}");
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {}
                }
            }
            assert!(
                Instant::now() < deadline,
                "{what} did not arrive within {:?}",
                teardown::UI_WAIT
            );
            self.wait_tick();
        }
    }

    pub fn apply_event(&mut self, event: Event) {
        let mut event = Some(event);
        let deadline = Instant::now() + teardown::UI_WAIT;
        let presenter = self.presenter.clone();
        let begin = self.begin_owned.clone();
        loop {
            if with_owned_commands(&presenter, &begin, |presenter, send| {
                self.apply_scoped(event.take().unwrap(), presenter, send);
            })
            .is_some()
            {
                break;
            }
            if Instant::now() >= deadline {
                // The explicit inspection caller handed ownership back; even a
                // fixture timeout must not destroy that capture on this thread.
                let _ = self
                    .runtime
                    .as_ref()
                    .expect("runtime alive until drop")
                    .block_on(teardown::retire_final(
                        &self.retire_queue,
                        vec![Box::new(event.take().unwrap())],
                    ));
                panic!("reclaimer did not accept the inspected event");
            }
            self.wait_tick();
        }
        self.after_apply();
    }

    fn drain_one(&mut self) -> bool {
        let presenter = self.presenter.clone();
        let begin = self.begin_owned.clone();
        let consumed = with_owned_commands(&presenter, &begin, |presenter, send| {
            let event = match self.events.try_recv() {
                Ok(event) => event,
                Err(mpsc::error::TryRecvError::Empty) => return false,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    panic!("Engine event channel closed before condition settled");
                }
            };
            self.apply_scoped(event, presenter, send);
            true
        }) == Some(true);
        self.command_reclaim.flush();
        if consumed {
            self.after_apply();
        }
        consumed
    }

    fn apply_scoped(
        &mut self,
        event: Event,
        presenter: &mut Presenter,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.record_maintenance(&event);
        if self.hold_maintenance_hints {
            match &event {
                Event::Tree(tree) => {
                    if let TreeEvent::Changed { stamp } = tree.as_ref() {
                        self.held_tree_hints[stamp.pane.index()] = Some(*stamp);
                        return;
                    }
                }
                Event::DirectoryChanged { pane, navigation } => {
                    self.held_directory_hints[pane.index()] = Some(*navigation);
                    return;
                }
                _ => {}
            }
        }
        if let Some(metadata) = ClipboardStep::event(&event) {
            self.clipboard_trace.borrow_mut().record(metadata);
        }
        if matches!(
            &event,
            Event::DesktopActionFinished {
                result: Ok(DesktopActionResult::ClipboardCopied),
                ..
            }
        ) {
            self.clipboard_publications += 1;
        }
        if let Event::Copy(CopyEvent::Progress(progress)) = &event {
            self.copy_progress.push(progress.clone());
            self.copy_applied = true;
        }
        if let Event::Tree(tree) = &event {
            match tree.as_ref() {
                TreeEvent::Changed { stamp } => self.watch_hints[stamp.pane.index()] += 1,
                TreeEvent::Published(publication) | TreeEvent::Reprojected { publication, .. } => {
                    let pane = publication.view.stamp.pane;
                    self.views[pane.index()] = Some(publication.view.clone());
                    if pane == self.pane {
                        self.view = Some(publication.view.clone());
                        self.selection = Some(publication.selection.clone());
                    }
                }
                TreeEvent::SelectionChanged {
                    view, selection, ..
                } if view.stamp.pane == self.pane => self.selection = Some(selection.clone()),
                TreeEvent::Sizes(TreeSizeEvent::Snapshot { sizes }) => {
                    self.sizes = Some(sizes.clone());
                }
                TreeEvent::Sizes(TreeSizeEvent::Finished { .. }) => {
                    self.size_finished += 1;
                }
                _ => {}
            }
        }
        if let Event::DirectoryChanged { pane, navigation } = event {
            self.watch_hints[pane.index()] += 1;
            presenter.refresh_if_changed(pane, navigation, send);
        } else {
            presenter.apply_with_sender(event, send);
        }
    }

    fn record_maintenance(&mut self, event: &Event) {
        fn stamp_text(stamp: TreeViewStamp) -> String {
            format!(
                "{:?} n{} s{} r{}",
                stamp.pane, stamp.navigation.0, stamp.session.0, stamp.revision.0
            )
        }
        let held = if self.hold_maintenance_hints {
            "held "
        } else {
            ""
        };
        let line = match event {
            Event::DirectoryChanged { pane, navigation } => {
                format!("{held}DirectoryChanged({pane:?} n{})", navigation.0)
            }
            Event::ListingFinished {
                pane, navigation, ..
            } => format!("ListingFinished({pane:?} n{})", navigation.0),
            Event::Tree(tree) => match tree.as_ref() {
                TreeEvent::Changed { stamp } => format!("{held}Changed({})", stamp_text(*stamp)),
                TreeEvent::Published(publication) | TreeEvent::Reprojected { publication, .. } => {
                    format!("Published({})", stamp_text(publication.view.stamp))
                }
                TreeEvent::Refreshed { stamp, result, .. } => {
                    format!("Refreshed({}, ok={})", stamp_text(*stamp), result.is_ok())
                }
                TreeEvent::RefreshRejected {
                    stamp, retryable, ..
                } => format!(
                    "RefreshRejected({}, retryable={retryable})",
                    stamp_text(*stamp)
                ),
                _ => return,
            },
            _ => return,
        };
        if self.maintenance.len() == 24 {
            self.maintenance.pop_front();
        }
        self.maintenance.push_back(line);
    }

    fn after_apply(&mut self) {
        refresh(&self.ui.window, &self.presenter.borrow());
        if std::mem::take(&mut self.copy_applied) {
            let rows = self.ui.window.get_operations();
            self.copy_rows.extend(
                rows.iter()
                    .filter(|row| row.identity.starts_with("copy-job:")),
            );
        }
        self.ui.draw();
        // The application timer samples visible rows even outside thumbnail
        // mode. Drive that real presenter path so background branch probes have
        // settled before asserting that options perform no provider I/O.
        let ranges = [
            (0, self.ui.window.get_left_rows().row_count()),
            (0, self.ui.window.get_right_rows().row_count()),
        ];
        let _ = with_owned_commands(&self.presenter, &self.begin_owned, |presenter, send| {
            let _ = presenter.thumbnail_targets(ranges, ["", ""]);
            presenter.flush_selection(send);
        });
        self.sync_visible();
    }

    fn sync_visible(&mut self) {
        if !self.watching {
            return;
        }
        for pane in [PaneId::Left, PaneId::Right] {
            let committed = &mut self.committed;
            if with_owned_commands(&self.presenter, &self.begin_owned, |presenter, send| {
                if let Some(visible) = presenter.visible_directory(pane) {
                    let navigation = visible.navigation;
                    if committed[pane.index()] != Some(navigation)
                        && send(Command::PaneCommitted(visible))
                    {
                        committed[pane.index()] = Some(navigation);
                    }
                }
            })
            .is_none()
            {
                break;
            }
        }
    }

    fn wait_tick(&self) {
        // No event owner or callback claim crosses this wait.
        self.runtime
            .as_ref()
            .expect("runtime alive until drop")
            .block_on(async {
                tokio::time::sleep(Duration::from_millis(1)).await;
            });
    }

    pub fn maintenance_hints_held(&self) -> bool {
        self.hold_maintenance_hints
    }

    pub fn release_maintenance_hints(&mut self) {
        // Disable first: replay and all later native hints take the ordinary path.
        self.hold_maintenance_hints = false;
        let tree = std::mem::take(&mut self.held_tree_hints);
        let directories = std::mem::take(&mut self.held_directory_hints);
        for pane in [PaneId::Left, PaneId::Right] {
            if let Some(stamp) = tree[pane.index()] {
                self.apply_event(Event::Tree(Box::new(TreeEvent::Changed { stamp })));
            }
            if let Some(navigation) = directories[pane.index()] {
                self.apply_event(Event::DirectoryChanged { pane, navigation });
            }
        }
    }

    pub fn clipboard_checkpoint(&self, stage: &'static str) {
        let active_pane = self.ui.window.get_active_pane();
        let focused_pane = self.ui.window.get_focused_pane();
        let notice_generation = self.ui.window.get_message_generation();
        let modal = self.ui.window.get_embedded_modal_open();
        let blocked = self.ui.window.get_dialog_input_blocked();
        let mut trace = self.clipboard_trace.borrow_mut();
        let scope_claims = trace.scope_claims;
        let scope_denials = trace.scope_denials;
        trace.record(ClipboardStep {
            stage,
            stamp: self.view.as_ref().map(|view| view.stamp),
            projection_ptr: self.view.as_ref().map(|view| Arc::as_ptr(view) as usize),
            selected: self.selection.as_ref().map(|selection| selection.len()),
            active_pane: Some(active_pane),
            focused_pane: Some(focused_pane),
            notice_generation: Some(notice_generation),
            modal: Some(modal),
            blocked: Some(blocked),
            scope_claims: Some(scope_claims),
            scope_denials: Some(scope_denials),
            ..ClipboardStep::default()
        });
    }

    #[track_caller]
    pub fn until(&mut self, condition: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + teardown::UI_WAIT;
        let mut last_retry = Instant::now();
        while Instant::now() < deadline {
            if condition(self) {
                return;
            }
            if !self.drain_one() {
                if last_retry.elapsed() >= Duration::from_millis(100) {
                    let _ = with_owned_commands(
                        &self.presenter,
                        &self.begin_owned,
                        |presenter, send| {
                            presenter.flush_selection(send);
                            presenter.flush_search(send);
                        },
                    );
                    self.sync_visible();
                    refresh(&self.ui.window, &self.presenter.borrow());
                    self.ui.draw();
                    last_retry = Instant::now();
                }
                self.wait_tick();
            }
        }
        panic!(
            "condition did not settle within {:?}: copy={:?}, clipboard_publications={}, desktop_status={}, left_error={}, right_error={}, selected={:?}, loading={:?}, folder_disclosures={:?}, notice=({},{:?},{:?}), active_pane={}, modal={}, dialog_blocked={}, maintenance={:?}, clipboard_trace={:?}",
            teardown::UI_WAIT,
            self.copy_progress.last(),
            self.clipboard_publications,
            self.ui.window.get_desktop_status(),
            self.ui.window.get_left_error(),
            self.ui.window.get_right_error(),
            self.selection.as_ref().map(|s| s.len()),
            (
                self.ui.window.get_left_loading(),
                self.ui.window.get_right_loading()
            ),
            [
                self.ui.window.get_left_rows(),
                self.ui.window.get_right_rows()
            ]
            .map(|rows| rows
                .iter()
                .filter(|row| row.name == "folder")
                .map(|row| (row.name.to_string(), row.tree_disclosure))
                .collect::<Vec<_>>()),
            self.ui.window.get_message_generation(),
            self.ui
                .window
                .get_message_title()
                .chars()
                .take(64)
                .collect::<String>(),
            self.ui
                .window
                .get_message_body()
                .chars()
                .take(256)
                .collect::<String>(),
            self.ui.window.get_active_pane(),
            self.ui.window.get_embedded_modal_open(),
            self.ui.window.get_dialog_input_blocked(),
            self.maintenance,
            self.clipboard_trace.borrow(),
        );
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.calls.frozen.store(false, Ordering::SeqCst);
        self.presenter.borrow_mut().retire_all();
        let mut retired = self.presenter.borrow_mut().take_retired();
        if let Some(pending) = self.command_reclaim.take_ready() {
            retired.extend(pending);
        }
        retired.push(Box::new(self.view.take()));
        retired.push(Box::new(std::mem::take(&mut self.views)));
        retired.push(Box::new(self.selection.take()));
        retired.push(Box::new(self.sizes.take()));
        let Some(runtime) = self.runtime.take() else {
            return;
        };
        // Never panic before the runtime is shut down: this usually runs while a
        // failed assertion unwinds, and dropping a runtime during a panic waits for
        // its blocking pool without limit.
        let mut failure = None;
        if let Some(mut engine) = self.engine.take() {
            let mut errors = Vec::new();
            let result = runtime.block_on(async {
                if let Err(error) = teardown::retire_final(&self.retire_queue, retired).await {
                    errors.push(error);
                }
                teardown::shutdown(
                    &self.commands,
                    &mut engine,
                    &mut self.events,
                    &self.retire_queue,
                    |_| {},
                )
                .await
            });
            if result.is_err() {
                engine.abort();
            }
            let (_, empty) = mpsc::channel(1);
            let receiver = std::mem::replace(&mut self.events, empty);
            if let Err(error) =
                runtime.block_on(teardown::retire_remaining(&self.retire_queue, receiver))
            {
                errors.push(error);
            }
            if result.is_err() || !errors.is_empty() {
                failure = Some(format!("{result:?}, {errors:?}"));
            }
        }
        // Bound blocking-pool work (a provider call stuck in teardown) instead of
        // waiting for it without limit.
        runtime.shutdown_timeout(teardown::RUNTIME_SHUTDOWN);
        if let Some(failure) = failure
            && !std::thread::panicking()
        {
            panic!("Engine shutdown failed: {failure}");
        }
    }
}
