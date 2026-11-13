#![forbid(unsafe_code)]

mod archive_selection;
mod archive_sizes;
mod archives;
pub use archives::ArchiveCapture;
mod captured_native;
mod checksum;
mod checksum_actor;
mod clipboard;
mod clipboard_text;
mod comparison;
mod copy_fence;
mod copy_operations;
mod desktop;
mod diagnostic_text;
mod drag_drop;
mod elevation;
mod engine_commands;
mod engine_configuration;
mod engine_lifetime;
pub use engine_configuration::{ServiceAlreadyConfigured, ServiceSlot};
mod engine_run;
mod report_arena;
pub use report_arena::ReportArena;
mod retirement;
mod task_lifetime;
pub use engine_lifetime::EngineLifetime;
pub use engine_run::EngineRun;
#[cfg(test)]
mod engine_lifecycle_tests;
mod entry_sizes;
mod file_list_actor;
mod file_list_output;
mod file_list_records;
mod file_list_worker;
mod file_targets;
mod filename_colors;
mod filters;
mod find;
mod find_sort;
mod find_worker;
mod folder_presence;
mod folder_size_totals;
mod folder_sizes;
mod fonts;
#[cfg(all(test, windows))]
#[path = "../tests/support/checksum_verify_links_windows.rs"]
mod native_test_links;
#[cfg(all(test, target_os = "linux"))]
#[path = "../tests/support/checksum_verify_links_linux.rs"]
mod native_test_links;
mod navigation_fallback;
#[cfg(test)]
mod navigation_fallback_tests;
#[cfg(all(test, unix))]
mod navigation_fallback_unix_tests;
#[cfg(all(test, windows))]
mod navigation_fallback_windows_tests;
mod navigation_ordering;
mod network_neighborhood;
mod operation_throughput;
mod operations;
#[cfg(test)]
mod outbox_tests;
mod path_picker;
mod preferences;
pub mod providers;
pub mod reclaim;
mod rectangle_summary;
mod scan_cache;
mod search;
mod selection;
mod selection_sizes;
mod shell_menu;
mod size_warning_requests;
mod size_warnings;
pub mod snapshot;
mod streaming_selection;
#[cfg(test)]
mod task_service_tests;
mod tree_actor;
mod tree_clipboard;
mod tree_controller;
mod tree_copy;
mod tree_delete;
mod tree_diagnostics;
mod tree_editor;
mod tree_metadata;
mod tree_projection;
mod tree_rename;
mod tree_roots;
mod tree_selection;
mod tree_sizes;
mod user_command_prepare;
mod user_command_runtime;
mod user_commands;
mod viewer_plugins;
mod volume_information;
mod volumes;
mod watch;
#[path = "copy_retirement.rs"]
mod worker_retirement;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use midnight_core::{
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

    fn blocking_send(&self, mut event: Event, cancel: &Cancellation) -> bool {
        loop {
            if cancel.is_cancelled() {
                return false;
            }
            match self.sender.try_send(event) {
                Ok(()) => {
                    (self.wakeup)();
                    return !cancel.is_cancelled();
                }
                Err(mpsc::error::TrySendError::Closed(_)) => return false,
                Err(mpsc::error::TrySendError::Full(unsent)) => event = unsent,
            }
            // A full UI queue must not strand a cancelled native worker or its permit.
            std::thread::park_timeout(std::time::Duration::from_millis(10));
        }
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

impl Drop for Request {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub struct Engine {
    services: retirement::WorkerOwned<EngineServices>,
    run: retirement::RetirementLease,
    commands: retirement::RetirementLease,
    resources: engine_lifetime::Resources,
}

#[derive(Clone)]
struct EngineServices {
    elevation: Option<Arc<dyn midnight_core::ElevatedDirectoryService>>,
    archive_capture: Option<ArchiveCapture>,
    navigation_fallback: Option<Arc<dyn midnight_core::NavigationFallbackService>>,
    user_commands: Option<Arc<dyn midnight_core::UserCommandService>>,
    find: Option<Arc<dyn midnight_core::FindService>>,
    locale: Option<Arc<dyn midnight_core::LocaleService>>,
    network: Option<Arc<dyn midnight_core::NetworkService>>,
    mutations: Option<Arc<dyn midnight_core::MutationService>>,
    filesystem: Arc<dyn FileSystem>,
    outbox: Outbox,
    desktop: Option<Arc<dyn DesktopServices>>,
    watch: Option<Arc<dyn WatchService>>,
    settings: Option<Arc<dyn SettingsService>>,
    editor: Option<Arc<dyn midnight_core::EditorService>>,
    clipboard: Option<Arc<dyn midnight_core::ClipboardService>>,
    shell_menu: Option<Arc<dyn midnight_core::ShellMenuService>>,
    path_picker: Option<Arc<dyn midnight_core::PathPickerService>>,
    transfers: Option<Arc<dyn midnight_core::TransferService>>,
    drag_drop: Option<Arc<dyn midnight_core::FileDragDropService>>,
}

#[derive(Default)]
struct SnapshotState {
    tree_notify: Option<tree_controller::RootReady>,
    network: Option<(NavigationId, Arc<[Entry]>)>,
    network_navigation: Option<NavigationId>,
    tree_root: Option<Arc<tree_roots::ReadyRoot>>,
    archive: Option<(NavigationId, Arc<[Entry]>)>,
    archive_navigation: Option<NavigationId>,
    archive_sort_sizes: Option<archive_sizes::published::Published>,
    committed: Option<CommittedSnapshot>,
    visible: Option<CommittedSnapshot>,
    pending: Option<NavigationId>,
}

impl SnapshotState {
    fn has_native_view(&self) -> bool {
        self.archive.is_none()
            && self.archive_navigation.is_none()
            && self.network.is_none()
            && self.network_navigation.is_none()
    }
}

#[derive(Clone)]
struct CommittedSnapshot {
    navigation: NavigationId,
    path: PathBuf,
    entries: Arc<[Entry]>,
    sizes: SharedSizes,
}

type Snapshots = Arc<retirement::WorkerOwned<Mutex<[SnapshotState; 2]>>>;

#[cfg(test)]
fn test_snapshots(states: [SnapshotState; 2]) -> Snapshots {
    let domain = retirement::RetirementDomain::try_start(1, 0).unwrap();
    Arc::new(domain.try_reserve().unwrap().own(Mutex::new(states)))
}
type SharedSizes = Arc<Mutex<HashMap<PathBuf, FolderSize>>>;

fn listing_failed(snapshots: &Snapshots, pane: PaneId, navigation: NavigationId) {
    let mut snapshots = snapshots.lock().unwrap_or_else(|error| error.into_inner());
    if snapshots[pane.index()].pending == Some(navigation) {
        snapshots[pane.index()].pending = None;
        if let Some(notify) = &snapshots[pane.index()].tree_notify {
            notify.notify(navigation);
        }
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

impl Drop for SizeRequest {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

impl Engine {
    /// Start and reserve runtime-independent cleanup before constructing providers.
    ///
    /// Returns an I/O error if the cleanup worker cannot start. No provider state
    /// has been accepted, and no Tokio runtime is required.
    pub fn lifetime() -> std::io::Result<EngineLifetime> {
        EngineLifetime::new()
    }

    /// Attach services to an already reserved lifetime; ownership is now protected
    /// even if the engine is discarded before `run` is called.
    pub fn new(
        lifetime: EngineLifetime,
        filesystem: Arc<dyn FileSystem>,
        events: mpsc::Sender<Event>,
    ) -> Self {
        Self::with_wakeup(lifetime, filesystem, events, Arc::new(|| {}))
    }

    /// The wakeup only schedules UI work; it must not touch UI objects directly.
    pub fn with_wakeup(
        lifetime: EngineLifetime,
        filesystem: Arc<dyn FileSystem>,
        events: mpsc::Sender<Event>,
        wakeup: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let services = EngineServices {
            elevation: None,
            archive_capture: None,
            navigation_fallback: None,
            user_commands: None,
            find: None,
            locale: None,
            network: None,
            mutations: None,
            filesystem,
            desktop: None,
            watch: None,
            settings: None,
            editor: None,
            clipboard: None,
            shell_menu: None,
            path_picker: None,
            transfers: None,
            drag_drop: None,
            outbox: Outbox {
                sender: events,
                wakeup,
            },
        };
        Self {
            services: lifetime.services.own(services),
            run: lifetime.run,
            commands: lifetime.commands,
            resources: lifetime.resources,
        }
    }

    /// Take command ownership synchronously, including before the first future poll.
    pub fn run(self, commands: mpsc::Receiver<Command>) -> EngineRun {
        let active = (*self.services).clone();
        let scope = task_lifetime::TaskLifetime::new(Arc::new(self.services));
        let commands = self.commands.own(engine_commands::CommandInbox(commands));
        EngineRun::new(
            self.run,
            active.run_inner(commands, self.resources, scope.clone()),
            scope,
        )
    }
}

impl EngineServices {
    async fn run_inner(
        self,
        mut commands: retirement::WorkerOwned<engine_commands::CommandInbox>,
        resources: engine_lifetime::Resources,
        scope: task_lifetime::TaskLifetime,
    ) {
        // Reserve capacity for each pane even when the other pane accumulates
        // uncancellable OS reads on unavailable media.
        let listing_budgets = [Arc::new(Semaphore::new(2)), Arc::new(Semaphore::new(2))];
        let auxiliary_budget = Arc::new(Semaphore::new(2));
        let network_budget = Arc::new(Semaphore::new(1));
        let mut locale_requested = false;
        let mut requests: [Option<Request>; 2] = [None, None];
        let denied_navigations = elevation::DeniedNavigations::default();
        let mut navigation_high_water: [Option<NavigationId>; 2] = [None, None];
        let mut tasks = JoinSet::new();
        let mut find = find::Controller::new(
            scope.clone(),
            resources.find,
            self.find,
            self.locale.clone(),
            self.outbox.clone(),
            &mut tasks,
        );
        let selection = selection::Controller::new(self.outbox.clone(), &mut tasks);
        let (properties_sender, mut properties_ready) =
            mpsc::channel::<selection::PreparedProperties>(2);
        let properties_budget = Arc::new(Semaphore::new(2));
        let archive_selection_budget = Arc::new(Semaphore::new(2));
        let fonts = fonts::Controller::new(
            scope.clone(),
            self.settings.clone(),
            self.outbox.clone(),
            &mut tasks,
        );
        let viewer_plugins = viewer_plugins::Controller::new(self.outbox.clone(), &mut tasks);
        let settings = self.settings.map(|service| {
            preferences::Controller::new(scope.clone(), service, self.outbox.clone(), &mut tasks)
        });
        let watcher = self.watch.map(|native| {
            watch::Controller::new(
                resources.watch,
                scope.clone(),
                self.filesystem.clone(),
                Some(native),
                self.outbox.clone(),
                &mut tasks,
            )
        });
        let desktop = desktop::Desktop::new(
            scope.clone(),
            resources.desktop,
            desktop::Processes {
                terminals: resources.user_command_processes,
                openers: resources.desktop_openers,
            },
            desktop::Services {
                desktop: self.desktop,
                locale: self.locale.clone(),
                capture: self.archive_capture.clone(),
            },
            self.outbox.clone(),
            &mut tasks,
        );
        let mut clipboard = clipboard::Controller::new(
            resources.clipboard,
            scope.clone(),
            self.clipboard,
            self.outbox.clone(),
        );
        let mut size_warning_requests = size_warning_requests::Controller::new(self.outbox.clone());
        let shell_menu = shell_menu::Controller::new(
            scope.clone(),
            resources.shell_menu,
            self.shell_menu,
            self.outbox.clone(),
        );
        let mut path_picker =
            path_picker::Controller::new(scope.clone(), self.path_picker, self.outbox.clone());
        let snapshots: Snapshots =
            Arc::new(resources.snapshots.own(Mutex::new(Default::default())));
        let mutation_gate = Arc::new(Semaphore::new(1));
        let trees = tree_controller::Controller::new(
            tree_controller::Services {
                filesystem: self.filesystem.clone(),
                locale: self.locale.clone(),
                reports: resources.reports.clone(),
                rename: resources.tree_rename,
            },
            listing_budgets.clone(),
            mutation_gate.clone(),
            snapshots.clone(),
            self.outbox.clone(),
            watcher.as_ref().map(watch::Controller::handle),
        );
        let folder_presence = folder_presence::Controller::new(
            scope.clone(),
            resources.reports.clone(),
            self.filesystem.clone(),
            snapshots.clone(),
            listing_budgets.clone(),
            self.outbox.clone(),
            &mut tasks,
        );
        let mut neighborhood = network_neighborhood::Controller::new(
            resources.network,
            scope.clone(),
            self.locale.clone(),
            self.outbox.clone(),
            snapshots.clone(),
            &mut tasks,
        );
        let mut archives = archives::Controller::new(
            resources.archives,
            scope.clone(),
            self.archive_capture,
            self.locale.clone(),
            self.outbox.clone(),
            snapshots.clone(),
            &mut tasks,
        );
        let comparison =
            comparison::Controller::new(snapshots.clone(), self.outbox.clone(), &mut tasks);
        let filters = filters::Controller::new(self.outbox.clone(), &mut tasks);
        let filename_colors = filename_colors::Controller::new(self.outbox.clone(), &mut tasks);
        let mut file_lists = file_list_actor::Controller::new(
            resources.file_lists,
            scope.clone(),
            self.transfers.clone(),
            mutation_gate.clone(),
            self.outbox.clone(),
            snapshots.clone(),
        );
        let user_commands = user_commands::Controller::new(
            resources.user_commands,
            self.user_commands,
            self.transfers.clone(),
            desktop.command_runtime(),
            self.outbox.clone(),
        );
        let checksums = checksum_actor::Controller::new(
            resources.checksums,
            self.transfers.clone(),
            self.outbox.clone(),
            mutation_gate.clone(),
        );
        let copies = copy_operations::Controller::new(
            resources.copies,
            self.transfers.clone(),
            self.outbox.clone(),
            mutation_gate.clone(),
            Arc::new(Semaphore::new(1)),
            &mut tasks,
        );
        let operations = operations::Controller::new(
            resources.operations,
            operations::Services {
                mutation: self.mutations,
                locale: self.locale.clone(),
            },
            snapshots.clone(),
            self.outbox.clone(),
            mutation_gate,
            copies.admission().fence(),
            &mut tasks,
        );
        let mut drag_drop = drag_drop::Controller::new(
            resources.drag,
            self.drag_drop,
            self.transfers,
            snapshots.clone(),
            copies.admission(),
            self.outbox.clone(),
            &mut tasks,
        );
        let search = search::Controller::new(snapshots.clone(), self.outbox.clone(), &mut tasks);
        let rectangle_summaries =
            rectangle_summary::Controller::new(self.outbox.clone(), &mut tasks);
        let selected_sizes =
            selection_sizes::Controller::new(self.outbox.clone(), snapshots.clone(), &mut tasks);
        let archive_sizes = archive_sizes::Controller::new(
            resources.archive_sizes,
            resources.reports.clone(),
            snapshots.clone(),
            self.outbox.clone(),
        );
        let size_budget = Arc::new(Semaphore::new(2));
        let size_delivery_cancel = Cancellation::default();
        let size_io_budget = Arc::new(folder_sizes::IoBudget::new());
        let mut entry_sizes = entry_sizes::Controller::new(
            entry_sizes::Services {
                filesystem: self.filesystem.clone(),
                scope: scope.clone(),
                reports: resources.reports.clone(),
            },
            self.outbox.clone(),
            snapshots.clone(),
            size_budget.clone(),
            size_delivery_cancel.clone(),
            size_io_budget.clone(),
            selected_sizes.observer.clone(),
        );
        let mut size_requests: [Option<SizeRequest>; 2] = [None, None];
        let volumes = volumes::Controller::new(
            scope.clone(),
            self.filesystem.clone(),
            self.locale.clone(),
            self.outbox.clone(),
            desktop.volume_icon_sender(),
            desktop.context_sender(),
            &mut tasks,
        );
        let volume_information = volume_information::Controller::new(
            scope.clone(),
            self.filesystem.clone(),
            self.outbox.clone(),
            &mut tasks,
        );

        loop {
            // Drain completions before accepting another continuously-ready command.
            while tasks.try_join_next().is_some() {}
            tokio::select! {
                Some(prepared) = properties_ready.recv() => {
                    desktop.properties(self.filesystem.clone(), prepared.request, prepared.paths, &mut tasks).await;
                }
                command = commands.recv() => {
                    let mut elevated = None;
                    let offer_elevation = self.elevation.is_some() && matches!(&command, Some(Command::Navigate { .. }));
                    let command = match command {
                        Some(Command::RetryDirectoryWithElevation { pane, denied_navigation, navigation }) => {
                            let Some(service) = self.elevation.clone() else { continue; };
                            let Some((path, sort)) = denied_navigations.consume(pane, denied_navigation, navigation) else { continue; };
                            elevated = Some(service);
                            Some(Command::Navigate { pane, navigation, path, sort })
                        }
                        command => command,
                    };
                    let navigation_request = match &command {
                        Some(Command::Navigate { pane, navigation, .. }
                            | Command::Refresh { pane, navigation, .. }
                            | Command::Resort { pane, navigation, .. }
                            | Command::ArchiveNavigate { pane, navigation, .. }
                            | Command::NetworkNavigate { pane, navigation, .. }) => Some((*pane, *navigation)),
                        _ => None,
                    };
                    if let Some((pane, navigation)) = navigation_request {
                        if navigation_high_water[pane.index()].is_some_and(|old| navigation <= old) {
                            if let Some(Command::ArchiveNavigate { action, .. }) = command
                                && let Err(error) = archives.retire_action(action).await {
                                eprintln!("Stale archive input cleanup failed: {error}");
                            }
                            continue;
                        }
                        navigation_high_water[pane.index()] = Some(navigation);
                        denied_navigations.begin(pane, navigation);
                        folder_presence.cancel(pane);
                    }
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
                        Some(Command::RetryDirectoryWithElevation { .. }) => unreachable!("normalized above"),
                        Some(Command::RunFileList(request))=>{
                            let (id,generation)=(request.id,request.generation);
                            if let Err(error)=file_lists.start(*request,&mut tasks){
                                self.outbox.send(Event::FileList(Box::new(midnight_core::FileListEvent{id,generation,result:Err(error)}))).await;
                            }
                        }
                        Some(Command::ConfirmFileList{id,generation})=>{
                            if let Err(error)=file_lists.confirm(id,generation,&mut tasks){
                                self.outbox.send(Event::FileList(Box::new(midnight_core::FileListEvent{id,generation,result:Err(error)}))).await;
                            }
                        }
                        Some(Command::CancelFileList{id,generation})=>file_lists.cancel(id,generation),
                        Some(Command::CopyGeneratedText{request,content,cancel})=>clipboard.copy_generated(request,content,cancel,&mut tasks).await,
                        Some(Command::ReadDiagnosticText { request, content, cancel }) => size_warning_requests.preview(request, content, cancel, &mut tasks).await,
                        Some(Command::OpenSizeWarnings { request, report, cancel }) => size_warning_requests.open(request, report, cancel, &mut tasks).await,
                        Some(Command::PreviewUserCommand(request)) => {
                            let (request_id, command) = (request.id, request.command.clone());
                            if let Err(error) = user_commands.start(*request, true) {
                                self.outbox.send(Event::UserCommand(Box::new(midnight_core::UserCommandEvent { request: request_id, command, update: midnight_core::UserCommandUpdate::Rejected(error) }))).await;
                            }
                        }
                        Some(Command::ExecuteUserCommand(request)) => {
                            let (request_id, command) = (request.id, request.command.clone());
                            if let Err(error) = user_commands.start(*request, false) {
                                self.outbox.send(Event::UserCommand(Box::new(midnight_core::UserCommandEvent { request: request_id, command, update: midnight_core::UserCommandUpdate::Rejected(error) }))).await;
                            }
                        }
                        Some(Command::CancelUserCommand(id)) => user_commands.cancel(id),
                        Some(Command::StartChecksums(request)) => {
                            let (job, generation) = (request.job, request.generation);
                            if let Err(error) = checksums.start(*request) {
                                self.outbox.send(Event::ChecksumRejected(Box::new(midnight_core::ChecksumRejected { job, generation, error }))).await;
                            }
                        }
                        Some(Command::StartSelectedChecksums(request)) => {
                            let (job, generation) = (request.job, request.generation);
                            if let Err(error) = checksums.start_selected(*request) {
                                self.outbox.send(Event::ChecksumRejected(Box::new(midnight_core::ChecksumRejected { job, generation, error }))).await;
                            }
                        }
                        Some(Command::StartCapturedChecksums(request)) => {
                            let (job, generation) = (request.job, request.generation);
                            if let Err(error) = checksums.start_captured(*request).await {
                                self.outbox.send(Event::ChecksumRejected(Box::new(midnight_core::ChecksumRejected { job, generation, error }))).await;
                            }
                        }
                        Some(Command::CancelChecksums { job, generation }) => checksums.cancel(job, generation),
                        Some(Command::VerifyChecksumManifest(request)) => {
                            let (job, generation) = (request.job, request.generation);
                            if let Err(error) = checksums.verify(*request) {
                                self.outbox.send(Event::ChecksumRejected(Box::new(midnight_core::ChecksumRejected { job, generation, error }))).await;
                            }
                        }
                        Some(Command::ReleaseChecksums { job, generation }) => checksums.release(job, generation),
                        Some(Command::SaveChecksumManifest { save_id, job, generation, request }) => {
                            if let Err(error) = checksums.save(save_id, (job, generation), *request) {
                                self.outbox.send(Event::ChecksumManifest(Box::new(midnight_core::ChecksumManifestEvent {
                                    save_id, job, generation, update: midnight_core::ChecksumManifestUpdate::Rejected(error),
                                }))).await;
                            }
                        }
                        Some(Command::CancelChecksumManifest { save_id }) => checksums.cancel_save(save_id),
                        Some(Command::StartFind(request)) => {
                            let session = request.session;
                            let generation = request.generation;
                            if let Err(error) = find.start(*request) {
                                self.outbox.send(Event::FindRejected(Box::new(midnight_core::FindRejected { session, generation, error }))).await;
                            }
                        }
                        Some(Command::CaptureFindFile(request)) => {
                            if let Err(error) = find.capture(request) {
                                self.outbox.send(Event::FindFileCaptured(Box::new(midnight_core::FindFileCaptureResult { request, result: Err(error) }))).await;
                            }
                        }
                        Some(Command::ExecuteFindFile { request, source, action, cancel }) => desktop.execute_find_file(self.editor.clone(), request, source, action, cancel, &mut tasks).await,
                        Some(Command::SortFind(request)) => find.sort(request),
                        Some(Command::CancelFind { session, generation }) => find.cancel(session, generation),
                        Some(Command::CloseFind(session)) => find.close(session),
                        Some(Command::ValidateFindResult(request)) => {
                            if let Err(error) = find.validate(request) {
                                self.outbox.send(Event::FindResultValidated(Box::new(midnight_core::FindValidationResult { request, result: Err(error) }))).await;
                            }
                        }
                        Some(Command::ArchiveSelect { pane, navigation, revision, entries, view, selected, action }) => {
                            let permit = match archive_selection_budget.clone().try_acquire_owned() {
                                Ok(permit) => permit,
                                Err(_) => {
                                    self.outbox.send(Event::ArchiveSelection { pane, navigation, revision, view, result: Err(archive_selection::error("Selection queue is busy. Try again.")) }).await;
                                    continue;
                                }
                            };
                            let outbox = self.outbox.clone();
                            tasks.spawn(async move {
                                let _permit = permit;
                                let source = view.clone();
                                let result = tokio::task::spawn_blocking(move || archive_selection::apply(&entries, &source, &selected, action))
                                    .await.unwrap_or_else(|error| Err(archive_selection::error(error.to_string())));
                                outbox.send(Event::ArchiveSelection { pane, navigation, revision, view, result }).await;
                            });
                        }
                        Some(Command::NetworkNavigate { pane, navigation, action, sort }) => {
                            if requests[pane.index()].as_ref().is_some_and(|old| navigation <= old.navigation) { continue; }
                            trees.invalidate(pane, navigation, sort);
                            if let Some(old) = requests[pane.index()].take() { old.cancel(); }
                            if let Some(old) = size_requests[pane.index()].take() { old.cancel(); }
                            entry_sizes.cancel_all(pane);
                            search.invalidate(pane);
                            comparison.invalidate(pane);
                            archives.cancel(pane);
                            neighborhood.navigate(pane, navigation, action, sort, &mut tasks);
                        }
                        Some(Command::ArchiveNavigate { pane, navigation, action, sort }) => {
                            if requests[pane.index()].as_ref().is_some_and(|old| navigation <= old.navigation) {
                                if let Err(error) = archives.retire_action(action).await {
                                    eprintln!("Stale archive input cleanup failed: {error}");
                                }
                                continue;
                            }
                            neighborhood.cancel(pane);
                            trees.invalidate(pane, navigation, sort);
                            if let Some(old) = requests[pane.index()].take() { old.cancel(); }
                            if let Some(old) = size_requests[pane.index()].take() { old.cancel(); }
                            entry_sizes.cancel_all(pane);
                            search.invalidate(pane);
                            comparison.invalidate(pane);
                            archives.navigate(pane, navigation, action, sort, &mut tasks).await;
                        }
                        Some(Command::LoadLocale) if !locale_requested => {
                            locale_requested = true;
                            let service = self.locale.clone();
                            let outbox = self.outbox.clone();
                            let worker_scope = scope.clone();
                            tasks.spawn(scope.future(async move {
                                let result = tokio::task::spawn_blocking(worker_scope.blocking(move || {
                                    service.map_or_else(|| Ok(midnight_core::NumberLocale::default()), |service| service.number_locale())
                                })).await.unwrap_or_else(|error| Err(midnight_core::FsError::from_io(
                                    "load locale", std::path::Path::new("locale"), std::io::Error::other(error.to_string()))));
                                outbox.send(Event::LocaleLoaded(result)).await;
                            }));
                        }
                        Some(Command::LoadLocale) => {}
                        Some(Command::ManageNetworkConnections { disconnect }) => {
                            let service = self.network.clone();
                            let permit = network_budget.clone().try_acquire_owned();
                            match (service, permit) {
                                (Some(service), Ok(permit)) => {
                                    let outbox = self.outbox.clone();
                                    let worker_scope = scope.clone();
                                    let permit = Arc::new(permit);
                                    let worker_permit = permit.clone();
                                    tasks.spawn(scope.future(async move {
                                        let _permit = permit;
                                        let result = tokio::task::spawn_blocking(worker_scope.blocking(move || {
                                            let _permit = worker_permit;
                                            service.manage_connections(disconnect)
                                        }))
                                            .await.unwrap_or_else(|error| Err(midnight_core::FsError::from_io(
                                                "network dialog", std::path::Path::new("Network"), std::io::Error::other(error.to_string()))));
                                        // Retain the permit through delivery so a stopped UI cannot
                                        // accumulate successful-but-unreported native dialog tasks.
                                        outbox.send(Event::NetworkConnectionsFinished(result)).await;
                                    }));
                                }
                                _ => { self.outbox.send(Event::NetworkConnectionsFinished(Err(
                                    midnight_core::FsError::from_io("network dialog", std::path::Path::new("Network"),
                                        std::io::Error::new(std::io::ErrorKind::WouldBlock, "Network dialogs are unavailable or already open."))
                                ))).await; }
                            }
                        }
                        Some(Command::PrepareOperation(request)) => operations.prepare(request).await,
                        Some(Command::PrepareCapturedDelete(request)) => operations.prepare_captured_delete(request).await,
                        Some(Command::PrepareCapturedRename(request)) => operations.prepare_captured_rename(request).await,
                        Some(Command::PrepareCapturedMetadata(request)) => operations.prepare_captured_metadata(request).await,
                        Some(Command::ConfirmOperation { id, new_name }) => operations.confirm(id, new_name).await,
                        Some(Command::CancelOperation(id)) => operations.cancel(id),
                        Some(Command::PreviewMetadata { id, revision, patch }) => operations.preview_metadata(id, revision, patch).await,
                        Some(Command::ConfirmMetadata { id, revision, conflicts }) => operations.confirm_metadata(id, revision, conflicts).await,
                        Some(Command::ResolveMetadataConflict { id, token, choice }) => operations.resolve_metadata_conflict(id, token, choice).await,
                        Some(Command::DecideRename(decision)) => operations.decide_rename(decision).await,
                        Some(Command::CompareDirectories(request)) => comparison.compare(request),
                        Some(Command::CancelComparison(id)) => comparison.cancel(id),
                        Some(Command::ValidatePanelFilter(request)) => filters.validate(request),
                        Some(Command::ValidateFilenameColors(request)) => filename_colors.validate(request),
                        Some(Command::MatchFilenameColors(request)) => filename_colors.matches(request),
                        Some(Command::ClearFilenameColors { pane, id }) => filename_colors.clear(pane, id),
                        Some(Command::EnqueueCopy(request)) => copies.enqueue(request).await,
                        Some(Command::EnqueueSelectedCopy(request)) => copies.enqueue_selected(request).await,
                        Some(Command::EnqueueMove(request)) => copies.enqueue_move(request).await,
                        Some(Command::EnqueueSelectedMove(request)) => copies.enqueue_selected_move(request).await,
                        Some(Command::EnqueueClipboardMove(request)) => copies.enqueue_clipboard_move(request).await,
                        Some(Command::EnqueueTransferWithMetadata(request)) => copies.enqueue_metadata(*request).await,
                        Some(Command::EnqueueCopyWithOptions(request)) => copies.enqueue_options(*request).await,
                        Some(Command::EnqueueArchiveCopy(request)) => copies.enqueue_archive(request).await,
                        Some(Command::EnqueueCapturedTransfer(request)) => copies.enqueue_captured(request).await,
                        Some(Command::DecideCopy(decision)) => copies.decide(decision).await,
                        Some(Command::CancelCopy(id)) => copies.cancel(id),
                        Some(Command::SetCopyPaused { id, paused }) => copies.set_paused(id, paused).await,
                        Some(Command::SetOperationPaused { id, paused }) => operations.set_paused(id, paused).await,
                        Some(Command::AttachDragDrop { host, targets }) => drag_drop.attach(host, targets, &mut tasks).await,
                        Some(Command::DetachDragDrop { host_generation }) => drag_drop.detach(host_generation),
                        Some(Command::PrepareDrag { id, pane, navigation, files }) => drag_drop.prepare(id, pane, navigation, files).await,
                        Some(Command::PrepareCapturedDrag { id, pane, navigation, source }) => drag_drop.prepare_captured(id, pane, navigation, source).await,
                        Some(Command::StartDrag(id)) => drag_drop.start(id).await,
                        Some(Command::CancelDrag(id)) => drag_drop.cancel(id).await,
                        Some(Command::AcceptDrop { token, id, verification }) => drag_drop.accept(token, id, verification, &copies).await,
                        Some(Command::RejectDrop(token)) => drag_drop.reject(token),
                        Some(Command::InternalDrop(request)) => {
                            if let Some(command) = drag_drop.internal_request(*request).await
                                && let Some(event) = trees.submit(command).await {
                                self.outbox.send(Event::Tree(Box::new(event))).await;
                            }
                        },
                        Some(Command::CompleteInternalDrop { id, target }) => drag_drop.complete_internal(id, target).await,
                        Some(Command::PreviewInternalDrop(request)) => {
                            if let Some(command) = drag_drop.preview_internal(*request).await
                                && let Some(event) = trees.submit(command).await {
                                self.outbox.send(Event::Tree(Box::new(event))).await;
                            }
                        },
                        Some(Command::CompleteInternalDropPreview { id, revision, target }) => drag_drop.complete_preview(id, revision, target).await,
                        Some(Command::Search(request)) => search.query(request),
                        Some(Command::CancelSearch { pane, revision }) => search.cancel(pane, revision),
                        None | Some(Command::Shutdown) => break,
                        Some(Command::Tree(command)) => {
                            if let Some(event) = trees.submit(*command).await {
                                self.outbox.send(Event::Tree(Box::new(event))).await;
                            }
                        },
                        Some(Command::SummarizeRectangle(request)) => rectangle_summaries.submit(request),
                        Some(Command::ProbeFolderPresence(request)) => folder_presence.submit(request).await,
                        Some(Command::Selection(request)) => selection.submit(request),
                        Some(Command::ListDesktopContext) => desktop.context(),
                        Some(Command::OpenSystemLocation { request, location }) => desktop.open_system_location(request, location, &mut tasks).await,
                        Some(Command::SetCommandIconView { generation, targets }) => desktop.command_icons(generation, targets),
                        Some(Command::ListViewerPlugins) => viewer_plugins.refresh(),
                        Some(Command::ResolveDefaultEditor) => desktop.resolve_editor(self.editor.clone(), &mut tasks).await,
                        Some(Command::EditFile { request, path, editor }) => desktop.edit_file(self.editor.clone(), request, path, editor, &mut tasks).await,
                        Some(Command::EditCapturedFile { request, source, editor, cancel }) => desktop.edit_captured_file(self.editor.clone(), request, source, editor, cancel, &mut tasks).await,
                        Some(Command::ListFontFamilies) => {
                            fonts.list();
                        }
                        Some(Command::LoadPreferences) => {
                            if let Some(settings) = &settings { settings.load(); }
                            else {
                                self.outbox.send(Event::PreferencesLoaded(Ok(Default::default()))).await;
                            }
                        }
                        Some(Command::SavePreferences { revision, preferences }) => {
                            if let Some(settings) = &settings { settings.save(revision, *preferences); }
                            else {
                                self.outbox.send(Event::PreferencesSaved { revision, result: Err(midnight_core::FsError::from_io("save preferences", std::path::Path::new(""), std::io::Error::from(std::io::ErrorKind::Unsupported))) }).await;
                            }
                        }
                        Some(Command::PaneCommitted(visible)) => {
                            let watch_target = visible.clone();
                            let mut snapshots = snapshots.lock().unwrap_or_else(|error| error.into_inner());
                            let state = &mut snapshots[visible.pane.index()];
                            if !state.has_native_view() { continue; }
                            if state.visible.as_ref().is_some_and(|old| old.navigation > visible.navigation) { continue; }
                            let same_snapshot = |snapshot: &&CommittedSnapshot| snapshot.navigation == visible.navigation && snapshot.path == visible.path && Arc::ptr_eq(&snapshot.entries, &visible.entries);
                            let existing = state.committed.as_ref().filter(same_snapshot).or_else(|| state.visible.as_ref().filter(same_snapshot));
                            state.visible = Some(existing.cloned().unwrap_or_else(|| CommittedSnapshot { navigation: visible.navigation, path: visible.path, entries: visible.entries, sizes: Arc::new(Mutex::new(HashMap::new())) }));
                            drop(snapshots);
                            if let Some(watcher) = &watcher { watcher.set_visible(watch_target); }
                        }
                        Some(Command::LaunchTerminal { request, terminal }) => desktop.launch(request, terminal, &mut tasks).await,
                        Some(Command::CopyText { request, text }) => clipboard.copy(request, text, &mut tasks).await,
                        Some(Command::ReadPathClipboardText { request }) => clipboard.read_text(request, &mut tasks).await,
                        Some(Command::PickPath { request, input }) => path_picker.show(request, input, &mut tasks).await,
                        Some(Command::CancelPathPicker { request }) => path_picker.cancel(request),
                        Some(Command::CopyFiles { request, files }) => clipboard.copy_files(request, files, &mut tasks).await,
                        Some(Command::CopyCapturedFiles { request, source, cancel }) => clipboard.copy_captured(request, source, cancel, &mut tasks).await,
                        Some(Command::CutFiles { request, files }) => clipboard.cut_files(request, files, &mut tasks).await,
                        Some(Command::ReadFileClipboard { request, destination }) => clipboard.read_files(request, destination, &mut tasks).await,
                        Some(Command::ShowShellMenu { request, files, owner, position, modifiers, target }) => {
                            shell_menu.show(shell_menu::Input { request, files, owner, position, modifiers, target }, &mut tasks).await;
                        }
                        Some(Command::ShowCapturedShellMenu { request, source, cancel, owner, position, modifiers }) => {
                            shell_menu.show_captured(shell_menu::CapturedInput { request, source, cancel, owner, position, modifiers }, &mut tasks).await;
                        }
                        Some(Command::ShowCapturedProperties { request, source, cancel }) => {
                            desktop.properties_captured(self.filesystem.clone(), request, source, cancel, &mut tasks).await;
                        }
                        Some(Command::OpenCapturedEntry { request, pane, navigation, row, source, cancel }) => {
                            desktop.open_captured_entry(request, (pane, navigation), source, cancel, row, &mut tasks).await;
                        }
                        Some(Command::ReadCapturedFileClipboard { request, source, cancel }) => {
                            clipboard.read_captured_files(request, source, cancel, &mut tasks).await;
                        }
                        Some(Command::ShowProperties { request, paths }) => desktop.properties(self.filesystem.clone(), request, paths, &mut tasks).await,
                        Some(Command::ShowSelectionProperties { request, selection, view, fallback }) => {
                            selection::prepare_properties(selection::PropertiesInput { request, selection, view, fallback },
                                properties_budget.clone(), self.outbox.clone(), properties_sender.clone(), &mut tasks).await;
                        }
                        Some(Command::OpenEntry { request, pane, navigation, path }) => desktop.open_entry(request, pane, navigation, path, false, &mut tasks).await,
                        Some(Command::ResolveDirectoryLink { request, pane, navigation, path }) => desktop.open_entry(request, pane, navigation, path, true, &mut tasks).await,
                        Some(Command::ArchiveSizes(command)) => archive_sizes.submit(command).await,
                        Some(Command::ComputeFolderSizes { pane, navigation, scan, selection }) => {
                            // Retrying an accepted command must not falsely finish
                            // the still-running scan with the same identity.
                            if size_requests[pane.index()].as_ref().is_some_and(|request| request.navigation == navigation && request.scan == scan) { continue; }
                            let entries = {
                                let snapshots = snapshots.lock().unwrap_or_else(|error| error.into_inner());
                                let state = &snapshots[pane.index()];
                                state.visible.as_ref().filter(|snapshot| snapshot.navigation == navigation)
                                    .or_else(|| state.committed.as_ref().filter(|snapshot| snapshot.navigation == navigation))
                                    .filter(|_| state.pending.is_none() && state.has_native_view()).cloned()
                            };
                            if entries.is_none() || size_requests[pane.index()].as_ref().is_some_and(|request| request.navigation == navigation && scan <= request.scan) {
                                self.outbox.send(Event::FolderSizesFinished { pane, navigation, scan, cancelled: true }).await;
                                continue;
                            }
                            if let Some(old) = size_requests[pane.index()].take() { old.cancel(); }
                            entry_sizes.cancel_all(pane);
                            let entries = entries.map(|selected| {
                                let old_sizes = selected.sizes.clone();
                                let selected = scan_cache::prepare(selected, &snapshots, pane, selection.as_deref());
                                if let Some(event) = selected_sizes.observer.replaced(pane, &old_sizes, &selected.sizes, &selected.entries) {
                                    selected_sizes.observer.publish(event);
                                }
                                selected
                            });
                            let cancel = Cancellation::default();
                            let worker_cancel = cancel.clone();
                            let filesystem = self.filesystem.clone();
                            let outbox = self.outbox.clone();
                            let budget = size_budget.clone();
                            let delivery_cancel = size_delivery_cancel.clone();
                            let io_budget = size_io_budget.clone();
                            let reports = resources.reports.clone();
                            let size_observer = selected_sizes.observer.clone();
                            let scan_path = entries.as_ref().map(|snapshot| snapshot.path.clone());
                            let scan_sizes = entries.as_ref().map(|snapshot| snapshot.sizes.clone());
                            let worker_scope = scope.clone();
                            let task = tasks.spawn(scope.future(async move {
                                let Some(snapshot) = entries else {
                                    outbox.send(Event::FolderSizesFinished { pane, navigation, scan, cancelled: true }).await;
                                    return;
                                };
                                let Ok(permit) = budget.acquire_owned().await else { return; };
                                if worker_cancel.is_cancelled() { return; }
                                let permit = Arc::new(permit);
                                let worker_permit = permit.clone();
                                let _permit = permit;
                                let final_cancel = worker_cancel.clone();
                                let final_outbox = outbox.clone();
                                let result = tokio::task::spawn_blocking(worker_scope.blocking(move || {
                                    let _permit = worker_permit;
                                    let roots: std::borrow::Cow<'_, [Entry]> = match &selection {
                                        None => std::borrow::Cow::Borrowed(&snapshot.entries),
                                        Some(selection) => std::borrow::Cow::Owned(snapshot.entries.iter()
                                            .filter(|entry| matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink) && selection.contains_path(&entry.path))
                                            .cloned().collect()),
                                    };
                                    let paths: HashMap<_, _> = roots.iter()
                                        .filter(|entry| matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink))
                                        .map(|entry| (entry.id, entry.path.clone())).collect();
                                    folder_sizes::compute_batched(filesystem.as_ref(), &roots, &worker_cancel, &mut |entry, size| {
                                        let Some(path) = paths.get(&entry) else { return false; };
                                        snapshot.sizes.lock().unwrap_or_else(|error| error.into_inner()).insert(path.clone(), size.clone());
                                        if let Some(event) = size_observer.updated(pane, &snapshot.sizes, path) {
                                            size_observer.publish(event);
                                        }
                                        // Cancellation freezes and publishes the last partial report.
                                        // Engine shutdown still interrupts a saturated UI queue.
                                        outbox.blocking_send(Event::FolderSizeUpdated { pane, navigation, scan, entry, size }, &delivery_cancel)
                                    }, &io_budget, &reports);
                                })).await;
                                if !final_cancel.is_cancelled() {
                                    final_outbox.send(Event::FolderSizesFinished { pane, navigation, scan, cancelled: result.is_err() }).await;
                                }
                            }));
                            if let (Some(path), Some(sizes)) = (scan_path, scan_sizes) {
                                size_requests[pane.index()] = Some(SizeRequest { scan, navigation, path, sizes, cancel, task });
                            }
                        }
                        Some(Command::ComputeEntrySize { pane, navigation, scan, entry }) => {
                            entry_sizes.compute(pane, navigation, scan, entry, &mut tasks).await;
                        }
                        Some(Command::SummarizeSelectedFolders(request)) => selected_sizes.submit(request),
                        Some(Command::CancelFolderSizes { pane, scan }) => {
                            entry_sizes.cancel(pane, scan).await;
                            if size_requests[pane.index()].as_ref().is_some_and(|request| request.scan == scan)
                                && let Some(old) = size_requests[pane.index()].take() {
                                let navigation = old.navigation;
                                old.cancel();
                                self.outbox.send(Event::FolderSizesFinished { pane, navigation, scan, cancelled: true }).await;
                            }
                        }
                        Some(Command::QueryVolumeInformation(request)) => volume_information.query(request),
                        Some(Command::ListVolumes) => {
                            volumes.refresh();
                        }
                        Some(Command::Navigate { pane, navigation, path, sort })
                        | Some(Command::Refresh { pane, navigation, path, sort, .. })
                        | Some(Command::Resort { pane, navigation, path, sort, .. }) => {
                            if requests[pane.index()].as_ref().is_some_and(|old| navigation <= old.navigation) { continue; }
                            if snapshots.lock().unwrap_or_else(|e| e.into_inner())[pane.index()].archive_navigation.is_some_and(|old| navigation <= old) { continue; }
                            trees.invalidate(pane, navigation, sort);
                            archives.cancel(pane);
                            neighborhood.cancel(pane);
                            let continued_entry_sizes = (refresh || resort)
                                .then(|| entry_sizes.continued_sizes(pane, &path, continued_scan)).flatten();
                            entry_sizes.navigate(pane, &path, refresh || resort);
                            search.invalidate(pane);
                            comparison.invalidate(pane);
                            let keep_scan = size_requests[pane.index()].as_ref().is_some_and(|request|
                                request.path == path && (resort || (refresh && continued_scan == Some(request.scan))));
                            let continued_sizes = keep_scan.then(|| size_requests[pane.index()].as_ref().expect("validated scan").sizes.clone()).or(continued_entry_sizes);
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
                            let locale = self.locale.clone();
                            let request_cancel = cancel.clone();
                            let listing_path = path.clone();
                            let icons = desktop.icon_sender();
                            let listing_snapshots = snapshots.clone();
                            let selection_stream = selection.streaming.clone();
                            let navigation_fallback = self.navigation_fallback.clone();
                            let denied_navigations = denied_navigations.clone();
                            let worker_scope = scope.clone();
                            let listing = tasks.spawn(scope.future(async move {
                                if !outbox.send(Event::ListingStarted { pane, navigation, path: listing_path.clone() }).await { return; }
                                let Ok(permit) = budget.acquire_owned().await else { return; };
                                if request_cancel.is_cancelled() { return; }
                                let error_outbox = outbox.clone();
                                let error_cancel = request_cancel.clone();
                                let error_path = listing_path.clone();
                                let error_snapshots = listing_snapshots.clone();
                                let result = tokio::task::spawn_blocking(worker_scope.blocking(move || {
                                    // The blocking worker owns its permit until it really finishes,
                                    // including when its async parent has been superseded.
                                    let _permit = permit;
                                    list_directory(ListingTask { elevated, offer_elevation, denied_navigations, filesystem, locale, outbox, pane, navigation, path: listing_path, sort, cancel: request_cancel, icons, snapshots: listing_snapshots, source, continued_sizes, selection_stream, navigation_fallback });
                                })).await;
                                if let Err(error) = result
                                    && !error_cancel.is_cancelled() {
                                    let failure = midnight_core::FsError::from_io("listing worker", &error_path, std::io::Error::other(error.to_string()));
                                    listing_failed(&error_snapshots, pane, navigation);
                                    error_outbox.send(Event::ListingFailed { pane, navigation, error: failure }).await;
                                }
                            }));
                            let filesystem = self.filesystem.clone();
                            let outbox = self.outbox.clone();
                            let budget = auxiliary_budget.clone();
                            let space_cancel = cancel.clone();
                            let worker_scope = scope.clone();
                            let space = tasks.spawn(scope.future(async move {
                                let Ok(permit) = budget.acquire_owned().await else { return; };
                                if space_cancel.is_cancelled() { return; }
                                let permit = Arc::new(permit);
                                let worker_permit = permit.clone();
                                let _permit = permit;
                                let storage_outbox = outbox.clone();
                                let storage_cancel = space_cancel.clone();
                                let result = tokio::task::spawn_blocking(worker_scope.blocking(move || {
                                    let _permit = worker_permit;
                                    let kind = filesystem.storage_kind(&path).unwrap_or_default();
                                    storage_outbox.blocking_send(Event::StorageKindUpdated { pane, navigation, kind }, &storage_cancel);
                                    filesystem.space_info(&path)
                                })).await;
                                if !space_cancel.is_cancelled()
                                    && let Ok(Ok(space)) = result {
                                    outbox.send(Event::SpaceUpdated { pane, navigation, space }).await;
                                }
                            }));
                            requests[pane.index()] = Some(Request { navigation, cancel, tasks: vec![listing, space] });
                        }
                    }
                }
                _ = tasks.join_next(), if !tasks.is_empty() => {}
            }
        }

        commands.close();
        drag_drop.shutdown();
        drop(folder_presence);
        user_commands.stop();
        find.shutdown();
        operations.shutdown();
        copies.shutdown();
        drop(neighborhood);
        drop(archives);
        trees.shutdown().await;
        // Queued commands may retain provider captures. Final release belongs to a
        // reserved cleanup worker even when Shutdown precedes them in the queue.
        retire_commands(commands).await;
        archive_sizes.shutdown().await;
        size_delivery_cancel.cancel();
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
        drop(selected_sizes);
        entry_sizes.shutdown();
        clipboard.shutdown();
        size_warning_requests.shutdown();
        file_lists.shutdown();
        shell_menu.shutdown();
        path_picker.shutdown();
        drop(search);
        fonts.shutdown();
        drop(volume_information);
        if let Some(settings) = settings {
            settings.shutdown();
            let _ =
                tokio::time::timeout(std::time::Duration::from_secs(2), settings.finished()).await;
        }
        checksums.shutdown().await;
        user_commands.shutdown().await;
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        tree_roots::shutdown(snapshots).await;
    }
}

async fn retire_commands(commands: retirement::WorkerOwned<engine_commands::CommandInbox>) {
    let _ = commands.retire().await;
}

struct ListingTask {
    elevated: Option<Arc<dyn midnight_core::ElevatedDirectoryService>>,
    offer_elevation: bool,
    denied_navigations: elevation::DeniedNavigations,
    navigation_fallback: Option<Arc<dyn midnight_core::NavigationFallbackService>>,
    selection_stream: streaming_selection::StreamingSelections,
    filesystem: Arc<dyn FileSystem>,
    locale: Option<Arc<dyn midnight_core::LocaleService>>,
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
        elevated,
        offer_elevation,
        denied_navigations,
        navigation_fallback,
        selection_stream,
        filesystem,
        locale,
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
            locale.as_deref(),
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
    let mut emit = |mut entry: Entry| {
        if cancel.is_cancelled() {
            return false;
        }
        localize_metadata(&mut entry.metadata, locale.as_deref());
        entries.push(entry.clone());
        if icon_types.len() < 128 {
            let key = IconKey::for_entry(&entry);
            if icon_types.insert(key.clone()) {
                let _ = icons.try_send(key);
            }
        }
        batch.push(entry);
        if batch.len() == BATCH_SIZE {
            selection_stream.append(pane, navigation, &path, &batch);
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
    };
    let result = if let Some(service) = &elevated {
        service.read_directory(&path, &cancel, &mut emit)
    } else {
        filesystem.read_directory(&path, &cancel, &mut emit)
    };
    if cancel.is_cancelled() {
        return;
    }
    if let Err(error) = result {
        listing_failed(&snapshots, pane, navigation);
        if offer_elevation
            && error.kind == std::io::ErrorKind::PermissionDenied
            && path.is_absolute()
            && denied_navigations.record(pane, navigation, &path, sort)
        {
            if outbox.blocking_send(
                Event::ListingFailed {
                    pane,
                    navigation,
                    error,
                },
                &cancel,
            ) {
                outbox.blocking_send(
                    Event::DirectoryElevationAvailable {
                        pane,
                        navigation,
                        path,
                    },
                    &cancel,
                );
            }
            return;
        }
        let event = match navigation_fallback::next_path(
            &path,
            &error,
            if elevated.is_some() {
                None
            } else {
                navigation_fallback.as_deref()
            },
            &cancel,
        ) {
            Ok(Some(path)) => Event::ListingFallback {
                pane,
                navigation,
                path,
                error,
            },
            Ok(None) => Event::ListingFailed {
                pane,
                navigation,
                error,
            },
            Err(error) => Event::ListingFailed {
                pane,
                navigation,
                error,
            },
        };
        outbox.blocking_send(event, &cancel);
        return;
    }
    selection_stream.append(pane, navigation, &path, &batch);
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
        if elevated.is_some() || filesystem.listing_metadata_complete(entry) {
            continue;
        }
        // A file can disappear or become unreadable while the directory is open.
        // Its name remains useful; unknown metadata is not presented as zero.
        if let Ok(mut value) = filesystem.entry_metadata(&entry.path) {
            localize_metadata(&mut value, locale.as_deref());
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
        locale.as_deref(),
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

fn localize_metadata(
    metadata: &mut midnight_core::EntryMetadata,
    locale: Option<&dyn midnight_core::LocaleService>,
) {
    if metadata.localized_modified.is_none()
        && let (Some(locale), Some(modified)) = (locale, metadata.modified)
    {
        metadata.localized_modified = locale.format_datetime(modified);
    }
}

fn publish_listing(
    outbox: &Outbox,
    snapshots: &Snapshots,
    locale: Option<&dyn midnight_core::LocaleService>,
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
            if matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink) {
                // Native directory-entry lengths are not recursive totals.
                entry.metadata.size = sizes
                    .get(&entry.path)
                    .filter(|size| size.finished && size.errors == 0)
                    .map(|size| size.bytes);
            }
        }
    }
    if let Some(locale) = locale
        && let Err(error) =
            midnight_core::prepare_sort_keys(Arc::make_mut(&mut snapshot.entries), locale, cancel)
    {
        if !cancel.is_cancelled() {
            listing_failed(snapshots, pane, snapshot.navigation);
            outbox.blocking_send(
                Event::ListingFailed {
                    pane,
                    navigation: snapshot.navigation,
                    error,
                },
                cancel,
            );
        }
        return;
    }
    sort_entries(Arc::make_mut(&mut snapshot.entries), sort);
    if cancel.is_cancelled() {
        return;
    }
    let index = Arc::new(midnight_core::DirectoryIndex::from_entries(
        &snapshot.entries,
    ));
    let navigation = snapshot.navigation;
    let entries = snapshot.entries.clone();
    let retired_root = {
        let mut cached = snapshots.lock().unwrap_or_else(|error| error.into_inner());
        if cancel.is_cancelled() {
            return;
        }
        let retired = cached[pane.index()].remember_native_tree_root(
            midnight_core::TreeRootSnapshot {
                pane,
                navigation,
                entries: entries.clone(),
                index: index.clone(),
            },
            snapshot.path.clone(),
        );
        cached[pane.index()].archive = None;
        cached[pane.index()].network_navigation = None;
        cached[pane.index()].network = None;
        cached[pane.index()].committed = Some(snapshot);
        cached[pane.index()].pending = None;
        retired
    };
    drop(retired_root);
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

#[cfg(test)]
mod native_output_test_fixture;
