#![forbid(unsafe_code)]

mod desktop;
mod folder_sizes;
mod fonts;
mod preferences;
pub mod reclaim;
mod scan_cache;
mod search;
mod selection;
pub mod snapshot;
mod volumes;
mod watch;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use fm_core::{
    Cancellation, Command, DesktopServices, Entry, EntryKind, Event, FileSystem, FolderSize,
    FolderSizeScanId, IconKey, NavigationId, PaneId, SettingsService, SortSpec, WatchService,
    sort_entries,
};
use tokio::{
    sync::{Semaphore, mpsc},
    task::{AbortHandle, JoinSet},
};

const BATCH_SIZE: usize = 128;

#[derive(Clone)]
struct Outbox {
    sender: mpsc::Sender<Event>,
    wakeup: Arc<dyn Fn() + Send + Sync>,
}

impl Outbox {
    async fn send(&self, event: Event) -> bool {
        if self.sender.send(event).await.is_err() {
            return false;
        }
        (self.wakeup)();
        true
    }

    fn blocking_send(&self, event: Event, cancel: &Cancellation) -> bool {
        if cancel.is_cancelled() || self.sender.blocking_send(event).is_err() {
            return false;
        }
        (self.wakeup)();
        !cancel.is_cancelled()
    }
}

struct Request {
    navigation: NavigationId,
    cancel: Cancellation,
    tasks: Vec<AbortHandle>,
}

impl Request {
    fn cancel(&self) {
        self.cancel.cancel();
        for task in &self.tasks {
            task.abort();
        }
    }
}

pub struct Engine {
    filesystem: Arc<dyn FileSystem>,
    outbox: Outbox,
    desktop: Option<Arc<dyn DesktopServices>>,
    watch: Option<Arc<dyn WatchService>>,
    settings: Option<Arc<dyn SettingsService>>,
}

#[derive(Default)]
struct SnapshotState {
    committed: Option<CommittedSnapshot>,
    visible: Option<CommittedSnapshot>,
    pending: Option<NavigationId>,
}

#[derive(Clone)]
struct CommittedSnapshot {
    navigation: NavigationId,
    path: PathBuf,
    entries: Arc<[Entry]>,
    sizes: SharedSizes,
}

type Snapshots = Arc<Mutex<[SnapshotState; 2]>>;
type SharedSizes = Arc<Mutex<HashMap<PathBuf, FolderSize>>>;

fn listing_failed(snapshots: &Snapshots, pane: PaneId, navigation: NavigationId) {
    let mut snapshots = snapshots.lock().unwrap_or_else(|error| error.into_inner());
    if snapshots[pane.index()].pending == Some(navigation) {
        snapshots[pane.index()].pending = None;
    }
}

struct SizeRequest {
    scan: FolderSizeScanId,
    navigation: NavigationId,
    path: PathBuf,
    sizes: SharedSizes,
    cancel: Cancellation,
    task: AbortHandle,
}

impl SizeRequest {
    fn cancel(self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

impl Engine {
    pub fn new(filesystem: Arc<dyn FileSystem>, events: mpsc::Sender<Event>) -> Self {
        Self::with_wakeup(filesystem, events, Arc::new(|| {}))
    }

    /// The wakeup only schedules UI work; it must not touch UI objects directly.
    pub fn with_wakeup(
        filesystem: Arc<dyn FileSystem>,
        events: mpsc::Sender<Event>,
        wakeup: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            filesystem,
            desktop: None,
            watch: None,
            settings: None,
            outbox: Outbox {
                sender: events,
                wakeup,
            },
        }
    }

    /// Optional native services stay independent of the filesystem provider.
    pub fn with_desktop(mut self, desktop: Arc<dyn DesktopServices>) -> Self {
        self.desktop = Some(desktop);
        self
    }

    /// Native change hints and polling run independently of navigation.
    pub fn with_watch(mut self, watch: Arc<dyn WatchService>) -> Self {
        self.watch = Some(watch);
        self
    }

    /// Configuration I/O is serialized independently of browsing and native dialogs.
    pub fn with_settings(mut self, settings: Arc<dyn SettingsService>) -> Self {
        self.settings = Some(settings);
        self
    }

    pub async fn run(self, mut commands: mpsc::Receiver<Command>) {
        // Reserve capacity for each pane even when the other pane accumulates
        // uncancellable OS reads on unavailable media.
        let listing_budgets = [Arc::new(Semaphore::new(2)), Arc::new(Semaphore::new(2))];
        let auxiliary_budget = Arc::new(Semaphore::new(2));
        let mut requests: [Option<Request>; 2] = [None, None];
        let mut tasks = JoinSet::new();
        let selection = selection::Controller::new(self.outbox.clone(), &mut tasks);
        let (properties_sender, mut properties_ready) =
            mpsc::channel::<selection::PreparedProperties>(2);
        let properties_budget = Arc::new(Semaphore::new(2));
        let fonts = fonts::Controller::new(self.settings.clone(), self.outbox.clone(), &mut tasks);
        let settings = self
            .settings
            .map(|service| preferences::Controller::new(service, self.outbox.clone(), &mut tasks));
        let watcher = self.watch.map(|native| {
            watch::Controller::new(
                self.filesystem.clone(),
                Some(native),
                self.outbox.clone(),
                &mut tasks,
            )
        });
        let desktop = desktop::Desktop::new(self.desktop, self.outbox.clone(), &mut tasks);
        let snapshots: Snapshots = Arc::new(Mutex::new(Default::default()));
        let search = search::Controller::new(snapshots.clone(), self.outbox.clone(), &mut tasks);
        let size_budget = Arc::new(Semaphore::new(2));
        let size_io_budget = Arc::new(folder_sizes::IoBudget::new());
        let mut size_requests: [Option<SizeRequest>; 2] = [None, None];
        let volumes = volumes::Controller::new(
            self.filesystem.clone(),
            self.outbox.clone(),
            desktop.volume_icon_sender(),
            desktop.context_sender(),
            &mut tasks,
        );

        loop {
            tokio::select! {
                Some(prepared) = properties_ready.recv() => {
                    desktop.properties(self.filesystem.clone(), prepared.request, prepared.paths, &mut tasks);
                }
                command = commands.recv() => {
                    let resort = matches!(&command, Some(Command::Resort { .. }));
                    let desired_source = match &command {
                        Some(Command::Resort { source, .. }) => source.clone(),
                        _ => None,
                    };
                    let refresh = matches!(&command, Some(Command::Refresh { .. }));
                    let continued_scan = match &command {
                        Some(Command::Refresh { scan, .. }) => *scan,
                        _ => None,
                    };
                    match command {
                        Some(Command::Search(request)) => search.query(request),
                        Some(Command::CancelSearch { pane, revision }) => search.cancel(pane, revision),
                        None | Some(Command::Shutdown) => break,
                        Some(Command::Selection(request)) => selection.submit(request),
                        Some(Command::ListDesktopContext) => desktop.context(),
                        Some(Command::ListFontFamilies) => {
                            fonts.list();
                        }
                        Some(Command::LoadPreferences) => {
                            if let Some(settings) = &settings { settings.load(); }
                            else {
                                let outbox = self.outbox.clone();
                                tasks.spawn(async move { outbox.send(Event::PreferencesLoaded(Ok(Default::default()))).await; });
                            }
                        }
                        Some(Command::SavePreferences { revision, preferences }) => {
                            if let Some(settings) = &settings { settings.save(revision, preferences); }
                            else {
                                let outbox = self.outbox.clone();
                                tasks.spawn(async move { outbox.send(Event::PreferencesSaved { revision, result: Err(fm_core::FsError::from_io("save preferences", std::path::Path::new(""), std::io::Error::from(std::io::ErrorKind::Unsupported))) }).await; });
                            }
                        }
                        Some(Command::PaneCommitted(visible)) => {
                            let watch_target = visible.clone();
                            let mut snapshots = snapshots.lock().unwrap_or_else(|error| error.into_inner());
                            let state = &mut snapshots[visible.pane.index()];
                            if state.visible.as_ref().is_some_and(|old| old.navigation > visible.navigation) { continue; }
                            let same_snapshot = |snapshot: &&CommittedSnapshot| snapshot.navigation == visible.navigation && snapshot.path == visible.path && Arc::ptr_eq(&snapshot.entries, &visible.entries);
                            let existing = state.committed.as_ref().filter(same_snapshot).or_else(|| state.visible.as_ref().filter(same_snapshot));
                            state.visible = Some(existing.cloned().unwrap_or_else(|| CommittedSnapshot { navigation: visible.navigation, path: visible.path, entries: visible.entries, sizes: Arc::new(Mutex::new(HashMap::new())) }));
                            drop(snapshots);
                            if let Some(watcher) = &watcher { watcher.set_visible(watch_target); }
                        }
                        Some(Command::LaunchTerminal { request, terminal }) => desktop.launch(request, terminal, &mut tasks),
                        Some(Command::ShowProperties { request, paths }) => desktop.properties(self.filesystem.clone(), request, paths, &mut tasks),
                        Some(Command::ShowSelectionProperties { request, selection, view, fallback }) => {
                            selection::prepare_properties(selection::PropertiesInput { request, selection, view, fallback },
                                properties_budget.clone(), self.outbox.clone(), properties_sender.clone(), &mut tasks);
                        }
                        Some(Command::OpenEntry { request, pane, navigation, path }) => desktop.open_entry(request, pane, navigation, path, &mut tasks),
                        Some(Command::ComputeFolderSizes { pane, navigation, scan }) => {
                            // Retrying an accepted command must not falsely finish
                            // the still-running scan with the same identity.
                            if size_requests[pane.index()].as_ref().is_some_and(|request| request.navigation == navigation && request.scan == scan) { continue; }
                            let entries = {
                                let snapshots = snapshots.lock().unwrap_or_else(|error| error.into_inner());
                                let state = &snapshots[pane.index()];
                                state.visible.as_ref().filter(|snapshot| snapshot.navigation == navigation)
                                    .or_else(|| state.committed.as_ref().filter(|snapshot| snapshot.navigation == navigation))
                                    .filter(|_| state.pending.is_none()).cloned()
                            };
                            if entries.is_none() || size_requests[pane.index()].as_ref().is_some_and(|request| request.navigation == navigation && scan <= request.scan) {
                                let outbox = self.outbox.clone();
                                tasks.spawn(async move { outbox.send(Event::FolderSizesFinished { pane, navigation, scan, cancelled: true }).await; });
                                continue;
                            }
                            if let Some(old) = size_requests[pane.index()].take() { old.cancel(); }
                            let entries = entries.map(|selected| scan_cache::prepare(selected, &snapshots, pane));
                            let cancel = Cancellation::default();
                            let worker_cancel = cancel.clone();
                            let filesystem = self.filesystem.clone();
                            let outbox = self.outbox.clone();
                            let budget = size_budget.clone();
                            let io_budget = size_io_budget.clone();
                            let scan_path = entries.as_ref().map(|snapshot| snapshot.path.clone());
                            let scan_sizes = entries.as_ref().map(|snapshot| snapshot.sizes.clone());
                            let task = tasks.spawn(async move {
                                let Some(snapshot) = entries else {
                                    outbox.send(Event::FolderSizesFinished { pane, navigation, scan, cancelled: true }).await;
                                    return;
                                };
                                let Ok(permit) = budget.acquire_owned().await else { return; };
                                if worker_cancel.is_cancelled() { return; }
                                let final_cancel = worker_cancel.clone();
                                let final_outbox = outbox.clone();
                                let result = tokio::task::spawn_blocking(move || {
                                    let _permit = permit;
                                    let paths: HashMap<_, _> = snapshot.entries.iter()
                                        .filter(|entry| entry.kind == EntryKind::Directory)
                                        .map(|entry| (entry.id, entry.path.clone())).collect();
                                    folder_sizes::compute_batched(filesystem.as_ref(), &snapshot.entries, &worker_cancel, &mut |entry, size| {
                                        if worker_cancel.is_cancelled() { return false; }
                                        let Some(path) = paths.get(&entry) else { return false; };
                                        snapshot.sizes.lock().unwrap_or_else(|error| error.into_inner()).insert(path.clone(), size);
                                        outbox.blocking_send(Event::FolderSizeUpdated { pane, navigation, scan, entry, size }, &worker_cancel)
                                    }, &io_budget);
                                }).await;
                                if !final_cancel.is_cancelled() {
                                    final_outbox.send(Event::FolderSizesFinished { pane, navigation, scan, cancelled: result.is_err() }).await;
                                }
                            });
                            if let (Some(path), Some(sizes)) = (scan_path, scan_sizes) {
                                size_requests[pane.index()] = Some(SizeRequest { scan, navigation, path, sizes, cancel, task });
                            }
                        }
                        Some(Command::CancelFolderSizes { pane, scan }) => {
                            if size_requests[pane.index()].as_ref().is_some_and(|request| request.scan == scan)
                                && let Some(old) = size_requests[pane.index()].take() {
                                let navigation = old.navigation;
                                old.cancel();
                                let outbox = self.outbox.clone();
                                tasks.spawn(async move { outbox.send(Event::FolderSizesFinished { pane, navigation, scan, cancelled: true }).await; });
                            }
                        }
                        Some(Command::ListVolumes) => {
                            volumes.refresh();
                        }
                        Some(Command::Navigate { pane, navigation, path, sort })
                        | Some(Command::Refresh { pane, navigation, path, sort, .. })
                        | Some(Command::Resort { pane, navigation, path, sort, .. }) => {
                            if requests[pane.index()].as_ref().is_some_and(|old| navigation <= old.navigation) { continue; }
                            search.invalidate(pane);
                            let keep_scan = size_requests[pane.index()].as_ref().is_some_and(|request|
                                request.path == path && (resort || (refresh && continued_scan == Some(request.scan))));
                            let continued_sizes = keep_scan.then(|| size_requests[pane.index()].as_ref().expect("validated scan").sizes.clone());
                            if !keep_scan && let Some(old) = size_requests[pane.index()].take() { old.cancel(); }
                            if let Some(old) = requests[pane.index()].take() { old.cancel(); }
                            let source = {
                                let mut snapshots = snapshots.lock().unwrap_or_else(|error| error.into_inner());
                                let state = &mut snapshots[pane.index()];
                                let source = if resort {
                                    desired_source.filter(|desired| desired.pane == pane && desired.path == path && desired.navigation < navigation)
                                        .map(|desired| {
                                            let exact = |snapshot: &&CommittedSnapshot|
                                                snapshot.navigation == desired.navigation && snapshot.path == desired.path
                                                    && Arc::ptr_eq(&snapshot.entries, &desired.entries);
                                            state.visible.as_ref().filter(exact).or_else(|| state.committed.as_ref().filter(exact))
                                                .cloned().unwrap_or_else(|| CommittedSnapshot {
                                                    navigation: desired.navigation, path: desired.path, entries: desired.entries,
                                                    sizes: continued_sizes.clone().unwrap_or_else(|| Arc::new(Mutex::new(HashMap::new()))),
                                                })
                                        })
                                } else { None };
                                state.pending = Some(navigation);
                                source
                            };
                            let cancel = Cancellation::default();
                            let filesystem = self.filesystem.clone();
                            let outbox = self.outbox.clone();
                            let budget = listing_budgets[pane.index()].clone();
                            let request_cancel = cancel.clone();
                            let listing_path = path.clone();
                            let icons = desktop.icon_sender();
                            let listing_snapshots = snapshots.clone();
                            let listing = tasks.spawn(async move {
                                if !outbox.send(Event::ListingStarted { pane, navigation, path: listing_path.clone() }).await { return; }
                                let Ok(permit) = budget.acquire_owned().await else { return; };
                                if request_cancel.is_cancelled() { return; }
                                let error_outbox = outbox.clone();
                                let error_cancel = request_cancel.clone();
                                let error_path = listing_path.clone();
                                let error_snapshots = listing_snapshots.clone();
                                let result = tokio::task::spawn_blocking(move || {
                                    // The blocking worker owns its permit until it really finishes,
                                    // including when its async parent has been superseded.
                                    let _permit = permit;
                                    list_directory(ListingTask { filesystem, outbox, pane, navigation, path: listing_path, sort, cancel: request_cancel, icons, snapshots: listing_snapshots, source, continued_sizes });
                                }).await;
                                if let Err(error) = result
                                    && !error_cancel.is_cancelled() {
                                    let failure = fm_core::FsError::from_io("listing worker", &error_path, std::io::Error::other(error.to_string()));
                                    listing_failed(&error_snapshots, pane, navigation);
                                    error_outbox.send(Event::ListingFailed { pane, navigation, error: failure }).await;
                                }
                            });
                            let filesystem = self.filesystem.clone();
                            let outbox = self.outbox.clone();
                            let budget = auxiliary_budget.clone();
                            let space_cancel = cancel.clone();
                            let space = tasks.spawn(async move {
                                let Ok(permit) = budget.acquire_owned().await else { return; };
                                if space_cancel.is_cancelled() { return; }
                                let result = tokio::task::spawn_blocking(move || {
                                    let _permit = permit;
                                    filesystem.space_info(&path)
                                }).await;
                                if !space_cancel.is_cancelled()
                                    && let Ok(Ok(space)) = result {
                                    outbox.send(Event::SpaceUpdated { pane, navigation, space }).await;
                                }
                            });
                            requests[pane.index()] = Some(Request { navigation, cancel, tasks: vec![listing, space] });
                        }
                    }
                }
                _ = tasks.join_next(), if !tasks.is_empty() => {}
            }
        }

        for request in requests.into_iter().flatten() {
            request.cancel();
        }
        for request in size_requests.into_iter().flatten() {
            request.cancel();
        }
        if let Some(watcher) = watcher {
            watcher.shutdown();
        }
        drop(selection);
        drop(search);
        fonts.shutdown();
        if let Some(settings) = settings {
            settings.shutdown();
            let _ =
                tokio::time::timeout(std::time::Duration::from_secs(2), settings.finished()).await;
        }
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
}

struct ListingTask {
    filesystem: Arc<dyn FileSystem>,
    outbox: Outbox,
    pane: PaneId,
    navigation: NavigationId,
    path: PathBuf,
    sort: SortSpec,
    cancel: Cancellation,
    icons: mpsc::Sender<IconKey>,
    snapshots: Snapshots,
    source: Option<CommittedSnapshot>,
    continued_sizes: Option<SharedSizes>,
}

fn list_directory(task: ListingTask) {
    let ListingTask {
        filesystem,
        outbox,
        pane,
        navigation,
        path,
        sort,
        cancel,
        icons,
        snapshots,
        source,
        continued_sizes,
    } = task;
    if let Some(source) = source {
        let entries = source.entries.to_vec();
        publish_listing(
            &outbox,
            &snapshots,
            pane,
            sort,
            &cancel,
            CommittedSnapshot {
                navigation,
                path,
                entries: Arc::from(entries),
                sizes: source.sizes,
            },
        );
        return;
    }
    let mut icon_types = std::collections::HashSet::new();
    let mut entries = Vec::new();
    let mut batch = Vec::with_capacity(BATCH_SIZE);
    let result = filesystem.read_directory(&path, &cancel, &mut |entry| {
        if cancel.is_cancelled() {
            return false;
        }
        entries.push(entry.clone());
        if icon_types.len() < 128 {
            let key = IconKey::for_entry(&entry);
            if icon_types.insert(key.clone()) {
                let _ = icons.try_send(key);
            }
        }
        batch.push(entry);
        if batch.len() == BATCH_SIZE {
            return outbox.blocking_send(
                Event::EntriesBatch {
                    pane,
                    navigation,
                    entries: std::mem::replace(&mut batch, Vec::with_capacity(BATCH_SIZE)),
                },
                &cancel,
            );
        }
        true
    });
    if cancel.is_cancelled() {
        return;
    }
    if let Err(error) = result {
        listing_failed(&snapshots, pane, navigation);
        outbox.blocking_send(
            Event::ListingFailed {
                pane,
                navigation,
                error,
            },
            &cancel,
        );
        return;
    }
    if !batch.is_empty()
        && !outbox.blocking_send(
            Event::EntriesBatch {
                pane,
                navigation,
                entries: batch,
            },
            &cancel,
        )
    {
        return;
    }

    let mut metadata = Vec::with_capacity(BATCH_SIZE);
    for entry in &mut entries {
        if cancel.is_cancelled() {
            return;
        }
        // A file can disappear or become unreadable while the directory is open.
        // Its name remains useful; unknown metadata is not presented as zero.
        if let Ok(value) = filesystem.entry_metadata(&entry.path) {
            entry.metadata = value.clone();
            metadata.push((entry.id, value));
        }
        if metadata.len() == BATCH_SIZE
            && !outbox.blocking_send(
                Event::MetadataBatch {
                    pane,
                    navigation,
                    entries: std::mem::replace(&mut metadata, Vec::with_capacity(BATCH_SIZE)),
                },
                &cancel,
            )
        {
            return;
        }
    }
    if !metadata.is_empty()
        && !outbox.blocking_send(
            Event::MetadataBatch {
                pane,
                navigation,
                entries: metadata,
            },
            &cancel,
        )
    {
        return;
    }
    if cancel.is_cancelled() {
        return;
    }
    publish_listing(
        &outbox,
        &snapshots,
        pane,
        sort,
        &cancel,
        CommittedSnapshot {
            navigation,
            path,
            entries: Arc::from(entries),
            sizes: continued_sizes.unwrap_or_else(|| Arc::new(Mutex::new(HashMap::new()))),
        },
    );
}

fn publish_listing(
    outbox: &Outbox,
    snapshots: &Snapshots,
    pane: PaneId,
    sort: SortSpec,
    cancel: &Cancellation,
    mut snapshot: CommittedSnapshot,
) {
    if cancel.is_cancelled() {
        return;
    }
    {
        let sizes = snapshot
            .sizes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for entry in Arc::make_mut(&mut snapshot.entries) {
            if cancel.is_cancelled() {
                return;
            }
            if entry.kind == EntryKind::Directory {
                // Native directory-entry lengths are not recursive totals.
                entry.metadata.size = sizes
                    .get(&entry.path)
                    .filter(|size| size.finished && size.errors == 0)
                    .map(|size| size.bytes);
            }
        }
    }
    sort_entries(Arc::make_mut(&mut snapshot.entries), sort);
    let index = Arc::new(fm_core::DirectoryIndex::from_entries(&snapshot.entries));
    let navigation = snapshot.navigation;
    let entries = snapshot.entries.clone();
    {
        let mut cached = snapshots.lock().unwrap_or_else(|error| error.into_inner());
        if cancel.is_cancelled() {
            return;
        }
        cached[pane.index()].committed = Some(snapshot);
        cached[pane.index()].pending = None;
    }
    outbox.blocking_send(
        Event::ListingFinished {
            pane,
            navigation,
            entries,
            index,
        },
        cancel,
    );
}
