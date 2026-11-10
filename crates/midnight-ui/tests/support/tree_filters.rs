//! Controlled providers with the production Engine and installed presenter callbacks.
// Each integration crate exercises a different subset of this shared fixture.
#![allow(dead_code)]
use midnight_core::*;
use midnight_engine::Engine;
use midnight_ui::{Presenter, install, refresh};
use slint::Model;
use std::{
    cell::RefCell,
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
}

struct Archive(Arc<Calls>);
impl std::fmt::Debug for Archive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FilterArchive")
    }
}
impl ArchiveMount for Archive {
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

pub struct Harness {
    pub ui: super::ui::TestUi,
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
    pub watch_hints: [usize; 2],
    pub send: Rc<dyn Fn(Command) -> bool>,
    pub pane: PaneId,
    runtime: Runtime,
    events: mpsc::Receiver<Event>,
    engine: Option<JoinHandle<()>>,
    commands: mpsc::Sender<Command>,
    watching: bool,
    committed: [Option<NavigationId>; 2],
}

struct NativeServices {
    path: PathBuf,
    directory: Arc<dyn FileSystem>,
    watch: Option<Arc<dyn WatchService>>,
    clipboard: Option<Arc<dyn ClipboardService>>,
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

    pub fn with_transfer(pane: PaneId, archive: bool, transfer: Arc<dyn TransferService>) -> Self {
        let h = Self::with_fixture(
            pane,
            archive,
            &["large", "small", "empty"],
            true,
            Some(transfer),
            None,
        );
        h.calls.frozen.store(false, Ordering::SeqCst);
        h
    }

    pub fn with_native_transfer(
        pane: PaneId,
        path: PathBuf,
        directory: Arc<dyn FileSystem>,
        transfer: Arc<dyn TransferService>,
    ) -> Self {
        Self::with_fixture(
            pane,
            false,
            &["large", "small", "empty"],
            true,
            Some(transfer),
            Some(NativeServices {
                path,
                directory,
                watch: None,
                clipboard: None,
            }),
        )
    }

    pub fn with_native_watch_transfer(
        pane: PaneId,
        path: PathBuf,
        directory: Arc<dyn FileSystem>,
        transfer: Arc<dyn TransferService>,
        watch: Arc<dyn WatchService>,
    ) -> Self {
        Self::with_fixture(
            pane,
            false,
            &["large", "small", "empty"],
            true,
            Some(transfer),
            Some(NativeServices {
                path,
                directory,
                watch: Some(watch),
                clipboard: None,
            }),
        )
    }

    pub fn with_native_clipboard_transfer(
        pane: PaneId,
        path: PathBuf,
        directory: Arc<dyn FileSystem>,
        transfer: Arc<dyn TransferService>,
        watch: Arc<dyn WatchService>,
        clipboard: Arc<dyn ClipboardService>,
    ) -> Self {
        Self::with_fixture(
            pane,
            false,
            &["large", "small", "empty"],
            true,
            Some(transfer),
            Some(NativeServices {
                path,
                directory,
                watch: Some(watch),
                clipboard: Some(clipboard),
            }),
        )
    }

    fn with_fixture(
        pane: PaneId,
        archive: bool,
        children: &[&str],
        size_fixture: bool,
        transfer: Option<Arc<dyn TransferService>>,
        native: Option<NativeServices>,
    ) -> Self {
        let ui = super::ui::TestUi::new();
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
        } = native.unwrap_or_else(|| NativeServices {
            path: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tree-filter-bridge"),
            directory: Arc::new(Files(calls.clone())),
            watch: None,
            clipboard: None,
        });
        let watching = watch.is_some();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let (commands, receive) = mpsc::channel(64);
        let (events_send, events) = mpsc::channel(64);
        let copying = transfer.is_some();
        let mut engine =
            Engine::new(directory, events_send).with_settings(Arc::new(Settings(calls.clone())));
        if let Some(transfer) = transfer {
            engine = engine.with_transfers(transfer);
        }
        if let Some(watch) = watch {
            engine = engine.with_watch(watch);
        }
        if let Some(clipboard) = clipboard {
            engine = engine.with_clipboard(clipboard);
        }
        let engine = runtime.spawn(engine.run(receive));
        let presenter = Rc::new(RefCell::new(Presenter::new(path.clone(), path.clone())));
        let requests = Rc::new(RefCell::new(Vec::new()));
        let reject_reproject = Rc::new(RefCell::new(false));
        let sent_requests = requests.clone();
        let rejected = reject_reproject.clone();
        let channel = commands.clone();
        let archive_calls = calls.clone();
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |mut command| {
            if matches!(
                &command,
                Command::Navigate { .. } | Command::ArchiveNavigate { .. }
            ) || matches!(&command, Command::Tree(tree) if matches!(tree.as_ref(), TreeCommand::Navigate { .. }))
            {
                archive_calls.navigations.fetch_add(1, Ordering::SeqCst);
            }
            // Supply a controlled provider mount for this fixture's typed saved
            // address. Presenter navigation, Engine listing and tree workers run normally.
            if let Command::ArchiveNavigate {
                action: ArchiveAction::List(location),
                ..
            } = &mut command
                && location.address.provider == "test.archive"
                && location.mount.is_none()
            {
                location.mount = Some(Arc::new(Archive(archive_calls.clone())));
                location.directory = 1;
                location.trail = vec![(1, "fixture.zip".into())];
            }
            if let Command::Tree(tree) = &command
                && let TreeCommand::Reproject(request) = tree.as_ref()
            {
                if *rejected.borrow() {
                    return false;
                }
                sent_requests.borrow_mut().push(request.clone());
            }
            channel.try_send(command).is_ok()
        });
        install(&ui.window, presenter.clone(), send.clone());
        let mut h = Self {
            ui,
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
            watch_hints: [0; 2],
            send,
            pane,
            runtime,
            events,
            engine: Some(engine),
            commands,
            watching,
            committed: [None; 2],
        };
        // Startup preferences restore both tabs through the installed presenter.
        // Streaming rows are visible before disclosure is available, so wait for
        // the completed listings rather than clicking an inert loading row.
        let mut preferences = UserPreferences::default();
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
        h.apply_event(Event::PreferencesLoaded(Ok(Box::new(preferences))));
        h.until(|h| {
            if native_listing {
                return !h.ui.window.get_left_loading() && !h.ui.window.get_right_loading();
            }
            [h.ui.window.get_left_rows(), h.ui.window.get_right_rows()]
                .iter()
                .all(|rows| {
                    rows.iter()
                        .any(|row| row.name == "folder" && row.tree_disclosure == 1)
                })
        });
        h.until(|h| {
            h.rows()
                .iter()
                .any(|row| row.name == "folder" && row.tree_disclosure == 1)
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
        h.until(|h| {
            h.view.as_ref().is_some_and(|view| {
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

    pub fn receive(&mut self) -> Event {
        let result = self.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), self.events.recv()).await
        });
        result
            .unwrap_or_else(|_| {
                panic!(
                    "no Engine event: rows={:?}, left_error={}, right_error={}, view={:?}",
                    self.rows()
                        .iter()
                        .map(|row| row.name.to_string())
                        .collect::<Vec<_>>(),
                    self.ui.window.get_left_error(),
                    self.ui.window.get_right_error(),
                    self.view.as_ref().map(|view| view.stamp)
                )
            })
            .unwrap()
    }

    pub fn apply_event(&mut self, event: Event) {
        if let Event::Copy(CopyEvent::Progress(progress)) = &event {
            self.copy_progress.push(progress.clone());
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
            self.presenter
                .borrow_mut()
                .refresh_if_changed(pane, navigation, self.send.as_ref());
        } else {
            self.presenter
                .borrow_mut()
                .apply_with_sender(event, self.send.as_ref());
        }
        refresh(&self.ui.window, &self.presenter.borrow());
        self.ui.draw();
        // The application timer samples visible rows even outside thumbnail
        // mode. Drive that real presenter path so background branch probes have
        // settled before asserting that options perform no provider I/O.
        let ranges = [
            (0, self.ui.window.get_left_rows().row_count()),
            (0, self.ui.window.get_right_rows().row_count()),
        ];
        let mut presenter = self.presenter.borrow_mut();
        let _ = presenter.thumbnail_targets(ranges, ["", ""]);
        presenter.flush_selection(self.send.as_ref());
        drop(presenter);
        self.sync_visible();
    }

    fn sync_visible(&mut self) {
        if !self.watching {
            return;
        }
        for pane in [PaneId::Left, PaneId::Right] {
            if let Some(visible) = self.presenter.borrow().visible_directory(pane) {
                let navigation = visible.navigation;
                if self.committed[pane.index()] != Some(navigation)
                    && (self.send)(Command::PaneCommitted(visible))
                {
                    self.committed[pane.index()] = Some(navigation);
                }
            }
        }
    }

    #[track_caller]
    pub fn until(&mut self, condition: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if condition(self) {
                return;
            }
            let event = self.runtime.block_on(async {
                tokio::time::timeout(Duration::from_millis(100), self.events.recv()).await
            });
            match event {
                Ok(Some(event)) => self.apply_event(event),
                Ok(None) => panic!("Engine event channel closed before condition settled"),
                Err(_) => {
                    // The app's ordinary 100 ms timer retries desired input even
                    // when no provider reply is pending (e.g. Options Cancel).
                    let mut presenter = self.presenter.borrow_mut();
                    presenter.flush_selection(self.send.as_ref());
                    presenter.flush_search(self.send.as_ref());
                    drop(presenter);
                    self.sync_visible();
                    refresh(&self.ui.window, &self.presenter.borrow());
                    self.ui.draw();
                }
            }
        }
        panic!(
            "condition did not settle within five seconds: copy={:?}, left_error={}, right_error={}, selected={:?}",
            self.copy_progress.last(),
            self.ui.window.get_left_error(),
            self.ui.window.get_right_error(),
            self.selection.as_ref().map(|s| s.len())
        );
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.calls.frozen.store(false, Ordering::SeqCst);
        let _ = self.commands.try_send(Command::Shutdown);
        if let Some(engine) = self.engine.take() {
            self.runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(5), engine)
                    .await
                    .unwrap()
                    .unwrap();
            });
        }
    }
}
