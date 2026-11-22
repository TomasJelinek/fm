//! Bounded optional desktop work, independent of navigation and folder scans.

use crate::{
    Outbox,
    retirement::RetirementWaiter,
    task_lifetime::TaskLifetime,
    user_command_runtime::{ProcessTicket, ProcessTracker},
};
use midnight_core::*;
use std::{
    collections::{HashSet, VecDeque},
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch},
    task::JoinSet,
};

#[path = "desktop_captured.rs"]
mod captured;

#[path = "desktop_command_icons.rs"]
mod command_icons;
#[path = "desktop_editor.rs"]
mod editor;
#[path = "desktop_file_icons.rs"]
mod file_icons;
#[path = "desktop_find.rs"]
mod find;
#[path = "desktop_link_targets.rs"]
mod link_targets;
#[path = "desktop_properties.rs"]
mod properties;

const ICON_CAPACITY: usize = DESKTOP_ICON_CAPACITY;
/// Concurrent Properties folder-contents counts.
const MAX_CONTENT_COUNTS: usize = 2;
const MAX_TERMINALS: usize = crate::user_command_runtime::MAX_PROCESSES;
const MAX_OPEN_HELPERS: usize = crate::user_command_runtime::MAX_OPEN_PROCESSES;
pub(crate) const MAX_OPEN_LAUNCHES: usize = 4;
pub(crate) const EDITOR_RETIREMENT_CAPACITY: usize = MAX_OPEN_LAUNCHES + 2;
pub(crate) struct Retirement {
    pub(crate) intake: RetirementWaiter,
    pub(crate) external: crate::desktop_external_compare::ExternalComparisonFrontDoor,
}
pub(crate) struct Processes {
    pub(crate) terminals: ProcessTracker,
    pub(crate) openers: ProcessTracker,
}

pub(crate) struct Services {
    pub(crate) desktop: Option<Arc<dyn DesktopServices>>,
    pub(crate) locale: Option<Arc<dyn LocaleService>>,
    pub(crate) capture: Option<crate::archives::ArchiveCapture>,
}

pub(crate) struct Desktop {
    locale: Option<Arc<dyn LocaleService>>,
    services: Option<Arc<dyn DesktopServices>>,
    scope: TaskLifetime,
    editor_intake: tokio::sync::Mutex<RetirementWaiter>,
    processes: Processes,
    icons: mpsc::Sender<IconKey>,
    command_icons: watch::Sender<command_icons::View>,
    file_icons: watch::Sender<file_icons::View>,
    icon_edges: watch::Sender<IconEdges>,
    volume_icons: watch::Sender<Vec<IconKey>>,
    children: mpsc::Sender<TrackedChild>,
    property_budget: Arc<Semaphore>,
    /// Folder-contents counts of Properties summaries; a replaced summary's count
    /// is cancelled, so a later one waits only briefly.
    contents_budget: Arc<Semaphore>,
    launch_budget: Arc<Semaphore>,
    context_requests: watch::Sender<u64>,
    link_targets: watch::Sender<Option<link_targets::Lookup>>,
    terminal_slots: Arc<Semaphore>,
    open_budget: Arc<Semaphore>,
    open_slots: Arc<Semaphore>,
    outbox: Outbox,
}

pub(crate) struct TrackedChild {
    pub(crate) child: Box<dyn TerminalProcess>,
    pub(crate) _slot: Arc<OwnedSemaphorePermit>,
    pub(crate) completion: Option<(DesktopRequestId, OpenCompletionTarget)>,
}

pub(crate) enum OpenCompletionTarget {
    File(PathBuf),
    System(SystemLocation),
    /// A followed web (`mail` false) or mail link.
    Url {
        mail: bool,
    },
}

struct OpenAdmission {
    permit: Arc<OwnedSemaphorePermit>,
    slot: Arc<OwnedSemaphorePermit>,
    ticket: ProcessTicket,
}

impl Desktop {
    pub(crate) fn compare_external(
        &self,
        service: Option<Arc<dyn UserCommandService>>,
        request: crate::desktop_external_compare::ArmedExternalComparison,
        tasks: &mut JoinSet<()>,
    ) {
        crate::desktop_external_compare::launch(
            crate::desktop_external_compare::LaunchContext {
                scope: self.scope.clone(),
                processes: self.terminal_slots.clone(),
                tracker: self.processes.terminals.clone(),
                children: self.children.clone(),
            },
            service,
            request,
            tasks,
        );
    }

    pub(crate) fn new(
        scope: TaskLifetime,
        retirement: Retirement,
        processes: Processes,
        services: Services,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let Services {
            desktop: services,
            locale,
            capture,
        } = services;
        let (icons, receiver) = mpsc::channel(64);
        let (command_icons, command_view) = watch::channel(command_icons::View::default());
        let (file_icons, file_view) = watch::channel(file_icons::View::default());
        let (icon_edges, drawn_edges) = watch::channel(IconEdges::default());
        tasks.spawn(scope.future(command_icons::worker(
            scope.clone(),
            services.clone(),
            capture,
            outbox.clone(),
            command_icons::Inputs {
                icons: receiver,
                commands: command_view,
                files: file_view,
                edges: drawn_edges,
            },
        )));
        let (volume_icons, roots) = watch::channel(Vec::new());
        tasks.spawn(forward_icons(roots, icons.clone()));
        let (location_icons, locations) = watch::channel(Vec::new());
        tasks.spawn(forward_icons(locations, icons.clone()));
        let (context_requests, context_receiver) = watch::channel(0);
        tasks.spawn(scope.future(context_worker(
            scope.clone(),
            services.clone(),
            locale.clone(),
            outbox.clone(),
            location_icons,
            context_receiver,
        )));
        let (link_targets, link_lookups) = watch::channel(None);
        if let Some(desktop) = services.clone() {
            tasks.spawn(scope.future(link_targets::run(
                scope.clone(),
                desktop,
                outbox.clone(),
                link_lookups,
            )));
        }
        let (children, child_receiver) = mpsc::channel(MAX_TERMINALS);
        tasks.spawn(reap_children(child_receiver, outbox.clone()));
        let result = Self {
            locale,
            services,
            scope,
            editor_intake: tokio::sync::Mutex::new(retirement.intake),
            processes,
            icons,
            command_icons,
            file_icons,
            icon_edges,
            volume_icons,
            children,
            property_budget: Arc::new(Semaphore::new(1)),
            contents_budget: Arc::new(Semaphore::new(MAX_CONTENT_COUNTS)),
            launch_budget: Arc::new(Semaphore::new(1)),
            context_requests,
            link_targets,
            terminal_slots: Arc::new(Semaphore::new(MAX_TERMINALS)),
            open_budget: retirement.external.open_budget(),
            open_slots: Arc::new(Semaphore::new(MAX_OPEN_HELPERS)),
            outbox,
        };
        for key in [
            IconKey::file(),
            IconKey::folder(),
            IconKey::folder_link(),
            IconKey::drive(),
            IconKey {
                kind: IconKind::Link,
                ..IconKey::file()
            },
        ]
        .into_iter()
        .chain(ActionIcon::ALL.into_iter().map(IconKey::action))
        {
            let _ = result.icons.try_send(key);
        }
        result
    }

    pub(crate) fn read_directory_link_target(
        &self,
        request: DesktopRequestId,
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
        snapshots: &crate::Snapshots,
    ) {
        if self.services.is_none() || !link_targets::current(snapshots, pane, navigation, &path) {
            return;
        }
        self.link_targets.send_replace(Some(link_targets::Lookup {
            request,
            pane,
            navigation,
            path,
            snapshots: snapshots.clone(),
        }));
    }

    pub(crate) fn command_icons(&self, generation: u64, targets: Vec<CommandIconTarget>) {
        if targets.capacity() > MAX_USER_COMMANDS || !command_icons::valid_targets(&targets) {
            return;
        }
        let mut targets = Some(targets);
        self.command_icons.send_if_modified(|view| {
            if generation <= view.generation {
                return false;
            }
            view.cancel.cancel();
            *view = command_icons::View {
                generation,
                targets: targets.take().unwrap_or_default(),
                cancel: Cancellation::default(),
            };
            true
        });
    }

    /// Replaces the visible tiles whose files carry their own artwork. The previous
    /// view's load in progress is cancelled; its tiles that are still visible and
    /// still lack artwork are part of the new view.
    pub(crate) fn file_icons(&self, generation: u64, targets: Vec<FileIconTarget>) {
        if targets.capacity() > MAX_FILE_ICON_TARGETS || !file_icons::valid_targets(&targets) {
            return;
        }
        let mut targets = Some(targets);
        self.file_icons.send_if_modified(|view| {
            if generation <= view.generation {
                return false;
            }
            view.cancel.cancel();
            *view = file_icons::View {
                generation,
                targets: targets.take().unwrap_or_default(),
                cancel: Cancellation::default(),
            };
            true
        });
    }

    /// Icons delivered so far are loaded again at edges they were not drawn at before.
    pub(crate) fn icon_edges(&self, edges: IconEdges) {
        if edges.is_valid() {
            self.icon_edges.send_if_modified(|current| {
                let changed = *current != edges;
                *current = edges;
                changed
            });
        }
    }

    pub(crate) fn icon_sender(&self) -> mpsc::Sender<IconKey> {
        self.icons.clone()
    }

    pub(crate) fn command_runtime(&self) -> crate::user_command_runtime::Runtime {
        crate::user_command_runtime::Runtime::new(
            self.launch_budget.clone(),
            self.terminal_slots.clone(),
            self.children.clone(),
            self.processes.terminals.clone(),
        )
    }

    pub(crate) fn volume_icon_sender(&self) -> watch::Sender<Vec<IconKey>> {
        self.volume_icons.clone()
    }

    pub(crate) fn context_sender(&self) -> watch::Sender<u64> {
        self.context_requests.clone()
    }

    pub(crate) fn context(&self) {
        self.context_requests
            .send_modify(|version| *version = version.wrapping_add(1));
    }

    pub(crate) async fn launch(
        &self,
        request: DesktopRequestId,
        terminal: TerminalRequest,
        tasks: &mut JoinSet<()>,
    ) {
        let Some(services) = self.services.clone() else {
            self.fail(request, "Terminal integration is unavailable.")
                .await;
            return;
        };
        // Exactly empty opens an interactive shell. Whitespace is still an invalid
        // submitted command, and neither launch path accepts an embedded NUL.
        if (!terminal.command.is_empty() && terminal.command.trim().is_empty())
            || terminal.command.contains('\0')
        {
            self.fail(
                request,
                "Terminal commands cannot contain NUL or consist only of whitespace.",
            )
            .await;
            return;
        }
        let Ok(permit) = self.launch_budget.clone().try_acquire_owned() else {
            self.fail(request, "A terminal is already being started.")
                .await;
            return;
        };
        let Ok(slot) = self.terminal_slots.clone().try_acquire_owned() else {
            self.fail(request, "Eight terminal processes are already active.")
                .await;
            return;
        };
        let Ok(ticket) = self.processes.terminals.try_reserve() else {
            self.fail(request, "Terminal process cleanup is busy. Try again.")
                .await;
            return;
        };
        let slot = Arc::new(slot);
        let permit = Arc::new(permit);
        let worker_permit = permit.clone();
        let outbox = self.outbox.clone();
        let children = self.children.clone();
        let scope = self.scope.clone();
        let process_scope = Arc::new(scope.clone());
        tasks.spawn(self.scope.future(async move {
            // Capacity covers native work and its terminal delivery, including panic.
            let _permit = permit;
            let path = terminal.working_directory.clone();
            let history = CommandHistoryEntry {
                command: terminal.command.clone(),
                shell: terminal.shell,
            };
            let result = tokio::task::spawn_blocking(scope.blocking(move || {
                let _permit = worker_permit;
                services
                    .launch_terminal(&terminal)
                    .map(|child| ticket.own(child, slot, process_scope))
            }))
            .await;
            let result = match result {
                Ok(Ok(child)) => {
                    let process_id = child.id();
                    // The bounded reaper retains the slot until the process exits.
                    let _ = children
                        .send(TrackedChild {
                            _slot: child.quota(),
                            child: Box::new(child),
                            completion: None,
                        })
                        .await;
                    Ok(DesktopActionResult::TerminalStarted {
                        process_id,
                        history,
                    })
                }
                Ok(Err(error)) => Err(error),
                Err(error) => Err(worker_error("launch terminal", &path, error)),
            };
            outbox
                .send(Event::DesktopActionFinished { request, result })
                .await;
        }));
    }

    pub(crate) async fn open_entry(
        &self,
        request: DesktopRequestId,
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
        directory_only: bool,
        tasks: &mut JoinSet<()>,
    ) {
        let Some(services) = self.services.clone() else {
            self.fail(request, "Desktop file opening is unavailable.")
                .await;
            return;
        };
        let Some(OpenAdmission {
            permit,
            slot,
            ticket,
        }) = self.reserve_open(request).await
        else {
            return;
        };
        let worker_permit = permit.clone();
        let outbox = self.outbox.clone();
        let children = self.children.clone();
        let scope = self.scope.clone();
        let process_scope = Arc::new(scope.clone());
        let tracking_slot = slot.clone();
        tasks.spawn(self.scope.future(async move {
            // Capacity covers native work and its terminal delivery, including panic.
            let _permit = permit;
            let target = path.clone();
            let result = tokio::task::spawn_blocking(scope.blocking(move || {
                let _permit = worker_permit;
                let result = if directory_only {
                    services
                        .directory_destination(&target)
                        .map(|path| OpenEntryResult::DirectoryTarget { path })
                } else {
                    services.open_entry(&target)
                };
                result.map(|result| match result {
                    OpenEntryResult::File {
                        process: Some(child),
                    } => OpenEntryResult::File {
                        process: Some(Box::new(ticket.own(child, slot, process_scope))),
                    },
                    other => other,
                })
            }))
            .await;
            let result = match result {
                Ok(result) => result.map(|result| (path, result)),
                Err(error) => Err(worker_error("open entry", &path, error)),
            };
            OpenCompletion {
                request,
                pane,
                navigation,
                children,
                outbox,
                slot: tracking_slot,
            }
            .finish(result)
            .await;
        }));
    }

    pub(crate) async fn open_system_location(
        &self,
        request: DesktopRequestId,
        location: SystemLocation,
        tasks: &mut JoinSet<()>,
    ) {
        let dispatch = move |services: &dyn DesktopServices| {
            if !services.supports_system_location(location) {
                return Err(unsupported_system_location());
            }
            services.open_system_location(location)
        };
        let target = OpenCompletionTarget::System(location);
        self.open_typed(
            request,
            target,
            unsupported_system_location,
            dispatch,
            tasks,
        )
        .await;
    }

    /// Hand a followed web or mail link to the system's default handler.
    pub(crate) async fn open_url(
        &self,
        request: DesktopRequestId,
        url: ExternalUrl,
        tasks: &mut JoinSet<()>,
    ) {
        let target = OpenCompletionTarget::Url {
            mail: url.is_mail(),
        };
        let dispatch = move |services: &dyn DesktopServices| services.open_url(&url);
        self.open_typed(request, target, unsupported_url, dispatch, tasks)
            .await;
    }

    /// Explicit typed activation without a filesystem path, such as a system location or
    /// a link. `dispatch` runs on a blocking backend worker within the shared native open
    /// budget and helper quota; a helper it returns is owned and reaped before `target`'s
    /// success is published. Without desktop services the answer is `unavailable()`.
    async fn open_typed(
        &self,
        request: DesktopRequestId,
        target: OpenCompletionTarget,
        unavailable: fn() -> FsError,
        dispatch: impl FnOnce(&dyn DesktopServices) -> Result<Option<Box<dyn TerminalProcess>>, FsError>
        + Send
        + 'static,
        tasks: &mut JoinSet<()>,
    ) {
        let Some(services) = self.services.clone() else {
            self.outbox
                .send(Event::DesktopActionFinished {
                    request,
                    result: Err(unavailable()),
                })
                .await;
            return;
        };
        let Some(OpenAdmission {
            permit,
            slot,
            ticket,
        }) = self.reserve_open(request).await
        else {
            return;
        };
        let worker_permit = permit.clone();
        let tracking_slot = slot.clone();
        let scope = self.scope.clone();
        let process_scope = Arc::new(scope.clone());
        let children = self.children.clone();
        let outbox = self.outbox.clone();
        tasks.spawn(self.scope.future(async move {
            let _permit = permit;
            let result = tokio::task::spawn_blocking(scope.blocking(move || {
                let _permit = worker_permit;
                // The ticket and helper quota exist before dispatch. A returned
                // child becomes owned before it can cross cancellation boundaries.
                dispatch(services.as_ref()).map(|child| {
                    child.map(|child| -> Box<dyn TerminalProcess> {
                        Box::new(ticket.own(child, slot, process_scope))
                    })
                })
            }))
            .await;
            let result = match result {
                Ok(Ok(Some(child))) => {
                    let _ = children
                        .send(TrackedChild {
                            child,
                            _slot: tracking_slot,
                            completion: Some((request, target)),
                        })
                        .await;
                    return;
                }
                Ok(Ok(None)) => Ok(target.success()),
                Ok(Err(error)) => Err(error),
                Err(error) => Err(target.error(false, io::Error::other(error.to_string()))),
            };
            outbox
                .send(Event::DesktopActionFinished { request, result })
                .await;
        }));
    }

    async fn reserve_open(&self, request: DesktopRequestId) -> Option<OpenAdmission> {
        let Ok(permit) = self.open_budget.clone().try_acquire_owned() else {
            self.fail(request, "File opening is busy. Try again.").await;
            return None;
        };
        let Ok(slot) = self.open_slots.clone().try_acquire_owned() else {
            self.fail(request, "Too many desktop file-opening helpers are active.")
                .await;
            return None;
        };
        let Ok(ticket) = self.processes.openers.try_reserve() else {
            self.fail(request, "Desktop opener cleanup is busy. Try again.")
                .await;
            return None;
        };
        Some(OpenAdmission {
            permit: Arc::new(permit),
            slot: Arc::new(slot),
            ticket,
        })
    }

    pub(crate) async fn properties(
        &self,
        filesystem: Arc<dyn FileSystem>,
        request: DesktopRequestId,
        paths: Vec<PathBuf>,
        cancel: Cancellation,
        tasks: &mut JoinSet<()>,
    ) {
        if paths.is_empty() {
            return;
        }
        let Ok(permit) = self.property_budget.clone().try_acquire_owned() else {
            self.fail(request, "A properties request is already open.")
                .await;
            return;
        };
        let services = self.services.clone();
        let locale = self.locale.clone();
        let permit = Arc::new(permit);
        let worker_permit = permit.clone();
        let outbox = self.outbox.clone();
        let scope = self.scope.clone();
        let counting = self.contents_budget.clone();
        let worker_cancel = cancel.clone();
        tasks.spawn(self.scope.future(async move {
            // Capacity covers native work and its terminal delivery, including panic.
            let permit = permit;
            let result = tokio::task::spawn_blocking(scope.blocking(move || {
                let _permit = worker_permit;
                let shown = show_properties(
                    filesystem.as_ref(),
                    services.as_deref(),
                    locale.as_deref(),
                    &paths,
                    &worker_cancel,
                );
                // The provider outlives this worker only for a folder counted next.
                shown.map(|(result, folder)| (result, folder.map(|folder| (folder, filesystem))))
            }))
            .await;
            let (result, folder) = match result {
                Ok(Ok((result, folder))) => (Ok(result), folder),
                Ok(Err(error)) => (Err(error), None),
                Err(error) => (Err(worker_error("properties", Path::new(""), error)), None),
            };
            outbox
                .send(Event::DesktopActionFinished { request, result })
                .await;
            // Another Properties request may open while this folder is counted.
            drop(permit);
            if let Some((folder, filesystem)) = folder {
                count_contents(scope, counting, filesystem, folder, request, cancel, outbox).await;
            }
        }));
    }

    pub(crate) async fn resolve_editor(
        &self,
        services: Option<Arc<dyn EditorService>>,
        tasks: &mut JoinSet<()>,
    ) {
        let outbox = self.outbox.clone();
        let Ok(permit) = self.launch_budget.clone().try_acquire_owned() else {
            outbox
                .send(Event::EditorDefaultResolved(Err(editor_error(
                    "Editor discovery is busy. Reopen Options to retry.",
                ))))
                .await;
            return;
        };
        let Some(services) = services else {
            outbox
                .send(Event::EditorDefaultResolved(Err(editor_error(
                    "External editing is unavailable.",
                ))))
                .await;
            return;
        };
        let scope = self.scope.clone();
        let permit = Arc::new(permit);
        let worker_permit = permit.clone();
        tasks.spawn(self.scope.future(async move {
            let _permit = permit;
            let result = tokio::task::spawn_blocking(scope.blocking(move || {
                let _permit = worker_permit;
                services.default_editor()
            }))
            .await;
            let result = match result {
                Ok(result) => result,
                Err(cause) => Err(worker_error("find editor", Path::new(""), cause)),
            };
            outbox.send(Event::EditorDefaultResolved(result)).await;
        }));
    }

    async fn editor_failure(&self, request: DesktopRequestId, message: &str) {
        self.outbox
            .send(Event::DesktopActionFinished {
                request,
                result: Err(editor_error(message)),
            })
            .await;
    }

    async fn fail(&self, request: DesktopRequestId, message: &'static str) {
        self.outbox
            .send(Event::DesktopActionFinished {
                request,
                result: Err(FsError::from_io(
                    "desktop action",
                    Path::new(""),
                    io::Error::other(message),
                )),
            })
            .await;
    }
}

#[cfg(test)]
impl Desktop {
    fn testing(
        services: Option<Arc<dyn DesktopServices>>,
        locale: Option<Arc<dyn LocaleService>>,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let domain =
            crate::retirement::RetirementDomain::try_start(EDITOR_RETIREMENT_CAPACITY, 1).unwrap();
        Self::new(
            TaskLifetime::testing((services.clone(), locale.clone())),
            Retirement {
                intake: domain.new_waiter().unwrap(),
                external: crate::desktop_external_compare::ExternalComparisonFrontDoor::new(
                    domain.clone(),
                ),
            },
            Processes {
                terminals: ProcessTracker::try_start().unwrap(),
                openers: ProcessTracker::try_start_openers().unwrap(),
            },
            Services {
                desktop: services,
                locale,
                capture: None,
            },
            outbox,
            tasks,
        )
    }
}

fn editor_error(message: &str) -> FsError {
    FsError::from_io(
        "external editor",
        Path::new(""),
        io::Error::other(message.to_owned()),
    )
}

fn unsupported_system_location() -> FsError {
    FsError::from_io(
        "open system location",
        Path::new(""),
        io::Error::new(
            io::ErrorKind::Unsupported,
            "This desktop system location is unavailable.",
        ),
    )
}

fn unsupported_url() -> FsError {
    FsError::from_io(
        "open link",
        Path::new(""),
        io::Error::new(
            io::ErrorKind::Unsupported,
            "Opening web and mail links is unavailable.",
        ),
    )
}

impl OpenCompletionTarget {
    fn success(self) -> DesktopActionResult {
        match self {
            Self::File(_) => DesktopActionResult::FileOpened,
            Self::System(location) => DesktopActionResult::SystemLocationOpened { location },
            Self::Url { mail } => DesktopActionResult::UrlOpened { mail },
        }
    }
    fn error(&self, waiting: bool, error: io::Error) -> FsError {
        let (operation, path) = match self {
            Self::File(path) => (
                if waiting {
                    "wait for desktop opener"
                } else {
                    "open entry"
                },
                path.as_path(),
            ),
            Self::System(_) => (
                if waiting {
                    "wait for system location opener"
                } else {
                    "open system location"
                },
                Path::new(""),
            ),
            Self::Url { .. } => (
                if waiting {
                    "wait for link opener"
                } else {
                    "open link"
                },
                Path::new(""),
            ),
        };
        FsError::from_io(operation, path, error)
    }
}

struct OpenCompletion {
    request: DesktopRequestId,
    pane: PaneId,
    navigation: NavigationId,
    children: mpsc::Sender<TrackedChild>,
    outbox: Outbox,
    slot: Arc<OwnedSemaphorePermit>,
}
impl OpenCompletion {
    async fn finish(self, result: Result<(PathBuf, OpenEntryResult), FsError>) {
        let result = match result {
            Ok((path, OpenEntryResult::Directory)) => Ok(DesktopActionResult::OpenDirectory {
                pane: self.pane,
                navigation: self.navigation,
                path,
            }),
            Ok((path, OpenEntryResult::DirectoryTarget { path: target })) => {
                Ok(DesktopActionResult::OpenDirectoryTarget {
                    pane: self.pane,
                    navigation: self.navigation,
                    path,
                    target,
                })
            }
            Ok((_, OpenEntryResult::File { process: None })) => Ok(DesktopActionResult::FileOpened),
            Ok((
                path,
                OpenEntryResult::File {
                    process: Some(child),
                },
            )) => {
                let _ = self
                    .children
                    .send(TrackedChild {
                        child,
                        _slot: self.slot,
                        completion: Some((self.request, OpenCompletionTarget::File(path))),
                    })
                    .await;
                return;
            }
            Err(error) => Err(error),
        };
        self.outbox
            .send(Event::DesktopActionFinished {
                request: self.request,
                result,
            })
            .await;
    }
}

/// A native Properties dialog, or the metadata summary and, for a single folder,
/// the summary to complete once its contents are counted.
fn show_properties(
    filesystem: &dyn FileSystem,
    services: Option<&dyn DesktopServices>,
    locale: Option<&dyn LocaleService>,
    paths: &[PathBuf],
    cancel: &Cancellation,
) -> Result<(DesktopActionResult, Option<properties::Single>), FsError> {
    crate::captured_native::check_cancel(cancel)?;
    if let Some(services) = services
        && services.show_properties(paths)?
    {
        return Ok((DesktopActionResult::PropertiesOpened, None));
    }
    crate::captured_native::check_cancel(cancel)?;
    let numbers = locale.map_or_else(
        || Ok(NumberLocale::default()),
        |service| service.number_locale(),
    )?;
    let time = |value: std::time::SystemTime| {
        locale
            .and_then(|service| service.format_datetime(value))
            .map_or_else(
                || properties::utc_text(value),
                |text| format!("{} {}", text.date, text.time),
            )
    };
    // Details are an addition: a provider that cannot read them keeps the summary.
    let details = |path: &Path| {
        services
            .and_then(|services| services.property_details(path).ok().flatten())
            .map(|details| properties::Details::format(details, &time))
    };
    let summary = properties::summary(
        paths,
        &numbers,
        cancel,
        &|path| filesystem.entry_metadata(path),
        &details,
    )?;
    Ok((summary.result, summary.folder))
}

/// Count a single folder's contents for its Properties summary, publishing the
/// totals so far and then the final text. The request's cancellation (its summary
/// closed or replaced) stops the count without publishing.
async fn count_contents(
    scope: TaskLifetime,
    budget: Arc<Semaphore>,
    filesystem: Arc<dyn FileSystem>,
    folder: properties::Single,
    request: DesktopRequestId,
    cancel: Cancellation,
    outbox: Outbox,
) {
    let Ok(permit) = budget.acquire_owned().await else {
        return;
    };
    if cancel.is_cancelled() {
        return;
    }
    let _ = tokio::task::spawn_blocking(scope.blocking(move || {
        let _permit = permit;
        let publish = |body: String| {
            outbox.blocking_send(
                Event::PropertiesUpdated {
                    request,
                    title: folder.title.clone(),
                    body,
                },
                &cancel,
            )
        };
        // A closed outbox (the engine stopping) ends the count like a cancellation.
        let counted =
            properties::count_contents(filesystem.as_ref(), &folder.path, &cancel, &mut |so_far| {
                publish(folder.body(Some(properties::Contents::Calculating(Some(so_far)))))
            });
        if cancel.is_cancelled() {
            return;
        }
        publish(match &counted {
            Ok(totals) => folder.body(Some(properties::Contents::Counted(totals))),
            Err(error) => folder.body(Some(properties::Contents::Failed(&error.message))),
        });
    }))
    .await;
}

/// A watch slot coalesces refreshes without overlapping uninterruptible host calls.
async fn context_worker(
    scope: TaskLifetime,
    services: Option<Arc<dyn DesktopServices>>,
    locale: Option<Arc<dyn LocaleService>>,
    outbox: Outbox,
    icons: watch::Sender<Vec<IconKey>>,
    mut requests: watch::Receiver<u64>,
) {
    let Some(services) = services else {
        return;
    };
    while requests.changed().await.is_ok() {
        let version = *requests.borrow_and_update();
        let worker = services.clone();
        let locale = locale.clone();
        let result = tokio::task::spawn_blocking(scope.blocking(move || {
            let locations =
                crate::navigation_ordering::locations(worker.locations()?, locale.as_deref())?;
            let system_locations = crate::navigation_ordering::system_locations(
                &locations,
                worker.supports_system_location(SystemLocation::RecycleBin),
                locale.as_deref(),
            )?;
            Ok((worker.shell_flavor(), locations, system_locations))
        }))
        .await;
        let (event, keys) = match result {
            Ok(Ok((shell, locations, system_locations))) => {
                // Keep the finite semantic aliases ahead of large mount inventories.
                let keys = locations
                    .iter()
                    .filter(|location| location.kind != LocationKind::Volume)
                    .chain(
                        locations
                            .iter()
                            .filter(|location| location.kind == LocationKind::Volume),
                    )
                    .take(ICON_CAPACITY)
                    .map(IconKey::for_location)
                    .collect();
                (
                    Event::DesktopContext {
                        shell,
                        locations,
                        // Network is a virtual section boundary after the drives,
                        // before every local special folder. Its label never
                        // takes part in native location sorting.
                        network_position: 0,
                        system_locations,
                    },
                    keys,
                )
            }
            Ok(Err(error)) => (Event::DesktopContextFailed(error), Vec::new()),
            Err(error) => (
                Event::DesktopContextFailed(worker_error(
                    "desktop locations",
                    Path::new(""),
                    error,
                )),
                Vec::new(),
            ),
        };
        // Recheck after backpressure. No obsolete result, success or failure,
        // may replace the typed metadata requested for the latest inventory.
        let Ok(permit) = outbox.sender.reserve().await else {
            return;
        };
        {
            let latest = requests.borrow();
            if version != *latest {
                continue;
            }
            permit.send(event);
            icons.send_replace(keys);
        }
        (outbox.wakeup)();
    }
}

#[cfg(test)]
async fn icon_worker(
    scope: TaskLifetime,
    services: Option<Arc<dyn DesktopServices>>,
    outbox: Outbox,
    receiver: mpsc::Receiver<IconKey>,
) {
    let (sender, commands) = watch::channel(command_icons::View::default());
    drop(sender);
    let (sender, files) = watch::channel(file_icons::View::default());
    drop(sender);
    let (sender, edges) = watch::channel(IconEdges::default());
    drop(sender);
    let inputs = command_icons::Inputs {
        icons: receiver,
        commands,
        files,
        edges,
    };
    command_icons::worker(scope, services, None, outbox, inputs).await;
}

fn fallback_icon(kind: IconKind) -> Option<IconPixels> {
    if matches!(kind, IconKind::Link | IconKind::FolderLink) {
        return Some(link_fallback(kind == IconKind::FolderLink));
    }
    // Small backend-generated RGBA assets also work without a font icon glyph.
    let color = match kind {
        IconKind::Folder => [232, 182, 62, 255],
        IconKind::Drive => [126, 144, 163, 255],
        IconKind::Link | IconKind::FolderLink => unreachable!("link icons use a badge"),
        IconKind::File => [181, 200, 220, 255],
        IconKind::Action(_) | IconKind::Location(_) => return None,
    };
    let mut rgba = vec![0; 24 * 24 * 4];
    for y in 4..21 {
        for x in 3..21 {
            let i = (y * 24 + x) * 4;
            rgba[i..i + 4].copy_from_slice(&color);
        }
    }
    Some(IconPixels {
        width: 24,
        height: 24,
        rgba: rgba.into(),
        generic: kind == IconKind::File,
    })
}

fn link_fallback(folder: bool) -> IconPixels {
    const EDGE: usize = 24;
    let mut rgba = vec![0; EDGE * EDGE * 4];
    let mut paint = |left: usize, top: usize, right: usize, bottom: usize, color: [u8; 4]| {
        for y in top..bottom {
            for x in left..right {
                let offset = (y * EDGE + x) * 4;
                rgba[offset..offset + 4].copy_from_slice(&color);
            }
        }
    };
    if folder {
        paint(3, 4, 12, 8, [197, 140, 36, 255]);
        paint(2, 7, 23, 21, [197, 140, 36, 255]);
        paint(3, 8, 22, 20, [244, 193, 72, 255]);
    } else {
        paint(6, 2, 21, 22, [91, 112, 137, 255]);
        paint(7, 3, 20, 21, [224, 234, 245, 255]);
        paint(16, 3, 20, 7, [165, 188, 214, 255]);
    }
    // A bordered arrow badge remains recognizable without host art or icon fonts.
    paint(0, 13, 13, 24, [43, 73, 107, 255]);
    paint(1, 14, 12, 23, [250, 252, 255, 255]);
    paint(5, 15, 10, 17, [26, 93, 174, 255]);
    paint(8, 15, 10, 20, [26, 93, 174, 255]);
    paint(6, 17, 8, 19, [26, 93, 174, 255]);
    paint(4, 19, 6, 21, [26, 93, 174, 255]);
    paint(2, 20, 4, 22, [26, 93, 174, 255]);
    IconPixels {
        width: EDGE as u32,
        height: EDGE as u32,
        rgba: rgba.into(),
        generic: false,
    }
}

/// Coalesce discovery while native lookup is busy, without delaying volume metadata.
async fn forward_icons(mut roots: watch::Receiver<Vec<IconKey>>, icons: mpsc::Sender<IconKey>) {
    let mut pending = VecDeque::new();
    loop {
        tokio::select! {
            biased;
            changed = roots.changed() => {
                if changed.is_err() { return; }
                // Producer and consumer both bound this snapshot independently.
                pending = roots.borrow_and_update().iter().take(ICON_CAPACITY).cloned().collect();
            }
            permit = icons.reserve(), if !pending.is_empty() => {
                let Ok(permit) = permit else { return; };
                if let Some(key) = pending.pop_front() { permit.send(key); }
            }
            _ = icons.closed() => return,
        }
    }
}

async fn reap_children(mut receiver: mpsc::Receiver<TrackedChild>, outbox: Outbox) {
    let mut children = Vec::<TrackedChild>::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            child = receiver.recv() => match child {
                Some(child) => {
                    children.push(child);
                    // Openers commonly hand off and exit at once: observe a
                    // new helper promptly instead of up to a full period later.
                    tick.reset_immediately();
                }
                None => return,
            },
            _ = tick.tick(), if !children.is_empty() => {
                let result = tokio::task::spawn_blocking(move || poll_children(children)).await;
                let Ok((remaining, completed)) = result else { return; };
                children = remaining;
                for event in completed {
                    if !outbox.send(event).await { return; }
                }
            }
        }
    }
}

fn poll_children(mut children: Vec<TrackedChild>) -> (Vec<TrackedChild>, Vec<Event>) {
    let mut completed = Vec::new();
    children.retain_mut(|child| match child.child.try_wait() {
        Ok(Some(status)) => {
            if let Some((request, target)) = child.completion.take() {
                let result = if status.success() {
                    Ok(target.success())
                } else {
                    Err(target.error(
                        false,
                        io::Error::other(format!("Desktop opener exited with {status}")),
                    ))
                };
                completed.push(Event::DesktopActionFinished { request, result });
            }
            false
        }
        Ok(None) => true,
        Err(error) => {
            if let Some((request, target)) = child.completion.take() {
                completed.push(Event::DesktopActionFinished {
                    request,
                    result: Err(target.error(true, error)),
                });
            }
            true
        }
    });
    (children, completed)
}

fn worker_error(operation: &'static str, path: &Path, error: tokio::task::JoinError) -> FsError {
    FsError::from_io(operation, path, io::Error::other(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::{Future, poll_fn};
    use std::sync::{
        Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::Notify;

    #[test]
    fn link_fallbacks_have_a_recognizable_badge_and_distinct_file_or_folder_bases() {
        let folder = fallback_icon(IconKind::Folder).unwrap();
        let folder_link = fallback_icon(IconKind::FolderLink).unwrap();
        let link = fallback_icon(IconKind::Link).unwrap();
        assert_ne!(
            folder_link.rgba, folder.rgba,
            "a junction must not look like an ordinary folder"
        );
        assert_ne!(
            folder_link.rgba, link.rgba,
            "folder/file link bases must differ"
        );
        for icon in [folder_link, link] {
            let colors: HashSet<_> = icon.rgba.as_chunks::<4>().0.iter().copied().collect();
            assert!(
                colors.len() >= 4,
                "link fallback needs a visible base, border and arrow"
            );
            assert!(colors.contains(&[0, 0, 0, 0]));
        }
    }

    #[derive(Default)]
    struct Icons {
        keys: Mutex<Vec<IconKey>>,
        roots: Mutex<Vec<Volume>>,
        gate: Option<Arc<IconGate>>,
        context_gate: Option<Arc<IconGate>>,
        context_calls: AtomicUsize,
        metadata_calls: Option<Arc<AtomicUsize>>,
        _disposed: Option<NotifyOnDrop>,
    }

    struct NotifyOnDrop(Arc<Notify>);
    impl Drop for NotifyOnDrop {
        fn drop(&mut self) {
            self.0.notify_one();
        }
    }

    #[derive(Default)]
    struct IconGate {
        entered: Notify,
        released: Mutex<bool>,
        wake: Condvar,
    }

    impl IconGate {
        fn release(&self) {
            *self.released.lock().unwrap() = true;
            self.wake.notify_all();
        }
    }

    struct ReleaseIcon(Arc<IconGate>);

    impl Drop for ReleaseIcon {
        fn drop(&mut self) {
            self.0.release();
        }
    }

    impl IconService for Icons {
        fn load_icon(&self, key: &IconKey) -> Result<Option<IconPixels>, FsError> {
            self.keys.lock().unwrap().push(key.clone());
            if key.kind == IconKind::Action(ActionIcon::Options)
                && let Some(gate) = &self.gate
            {
                gate.entered.notify_one();
                let mut released = gate.released.lock().unwrap();
                while !*released {
                    released = gate.wake.wait(released).unwrap();
                }
            }
            Ok(match key.kind {
                IconKind::Action(ActionIcon::Options) => {
                    Some(IconPixels::new(1, 1, vec![12, 34, 56, 128]).unwrap())
                }
                IconKind::Action(ActionIcon::Help) => {
                    Some(IconPixels::new(1, 1, vec![0; 4]).unwrap())
                }
                IconKind::FolderLink => Some(IconPixels::new(1, 1, vec![0; 4]).unwrap()),
                IconKind::Action(ActionIcon::View) => Some(IconPixels {
                    width: 257,
                    height: 1,
                    rgba: vec![255; 1028].into(),
                    generic: false,
                }),
                _ => None,
            })
        }
    }
    impl TerminalService for Icons {
        fn shell_flavor(&self) -> ShellFlavor {
            ShellFlavor::Posix
        }
        fn launch_terminal(
            &self,
            _: &TerminalRequest,
        ) -> Result<Box<dyn TerminalProcess>, FsError> {
            unreachable!("icon lookup must not launch a terminal")
        }
    }
    impl PropertyService for Icons {
        fn show_properties(&self, _: &[PathBuf]) -> Result<bool, FsError> {
            unreachable!("icon lookup must not open properties")
        }
    }
    impl LocationService for Icons {
        fn locations(&self) -> Result<Vec<NavigationLocation>, FsError> {
            let locations = self
                .roots
                .lock()
                .unwrap()
                .iter()
                .map(|volume| NavigationLocation {
                    id: volume.id.clone(),
                    label: volume.label.clone(),
                    path: volume.root.clone(),
                    kind: LocationKind::Volume,
                    // Deliberately unrelated to path syntax: this is native metadata.
                    drive_letter: Some('Z'),
                })
                .collect();
            let call = self.context_calls.fetch_add(1, Ordering::SeqCst);
            if call == 0
                && let Some(gate) = &self.context_gate
            {
                gate.entered.notify_one();
                let mut released = gate.released.lock().unwrap();
                while !*released {
                    released = gate.wake.wait(released).unwrap();
                }
            }
            Ok(locations)
        }
    }
    impl OpenService for Icons {
        fn open_entry(&self, _: &Path) -> Result<OpenEntryResult, FsError> {
            unreachable!("icon lookup must not open files")
        }
    }

    impl DirectoryService for Icons {
        fn read_directory(
            &self,
            _: &Path,
            _: &Cancellation,
            _: &mut dyn FnMut(Entry) -> bool,
        ) -> Result<(), FsError> {
            Ok(())
        }
        fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
            let calls = self
                .metadata_calls
                .as_ref()
                .expect("only Properties reads metadata");
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(EntryMetadata {
                size: Some(17),
                ..EntryMetadata::default()
            })
        }
    }
    impl VolumeService for Icons {
        fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
            Ok(self.roots.lock().unwrap().clone())
        }
    }
    impl StorageService for Icons {
        fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
            Ok(SpaceInfo {
                available: 4,
                total: 8,
            })
        }
    }

    #[tokio::test]
    async fn unsupported_desktop_requests_do_not_allocate_a_task_per_rejection() {
        let (sender, events) = mpsc::channel(1);
        sender.try_send(Event::VolumesUpdated(Vec::new())).unwrap();
        let mut tasks = JoinSet::new();
        let desktop = Desktop::testing(
            None,
            None,
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        );
        let actors = tasks.len();
        // Poll each rejected dispatch once, then cancel its blocked delivery.
        // No scheduler yield or completion reaping can hide a retained response task.
        for id in 1..=2048 {
            let mut dispatch = std::pin::pin!(desktop.open_entry(
                DesktopRequestId(id),
                PaneId::Left,
                NavigationId(1),
                "captured/file".into(),
                false,
                &mut tasks,
            ));
            poll_fn(|context| {
                assert!(dispatch.as_mut().poll(context).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
        }
        let pending = tasks.len() - actors;
        drop(events);
        drop(desktop);
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        assert_eq!(
            pending, 0,
            "Unsupported commands retained {pending} extra tasks"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_single_folder_summary_is_completed_with_its_counted_contents() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("photos");
        std::fs::create_dir_all(folder.join("inner")).unwrap();
        std::fs::write(folder.join("a.jpg"), vec![1; 1000]).unwrap();
        std::fs::write(folder.join("inner/b.jpg"), vec![2; 24]).unwrap();
        let platform = Arc::new(midnight_platform::LocalFileSystem::new(None));
        let (sender, mut events) = mpsc::channel(8);
        let mut tasks = JoinSet::new();
        let desktop = Desktop::testing(
            Some(platform.clone() as Arc<dyn DesktopServices>),
            None,
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        );
        async fn received(events: &mut mpsc::Receiver<Event>) -> Event {
            loop {
                let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
                    .await
                    .expect("a Properties event")
                    .expect("an open outbox");
                if matches!(
                    event,
                    Event::DesktopActionFinished { .. } | Event::PropertiesUpdated { .. }
                ) {
                    return event;
                }
            }
        }
        let cancel = Cancellation::default();
        desktop
            .properties(
                platform.clone(),
                DesktopRequestId(9),
                vec![folder.clone()],
                cancel.clone(),
                &mut tasks,
            )
            .await;
        let Event::DesktopActionFinished {
            request: DesktopRequestId(9),
            result: Ok(DesktopActionResult::PropertiesInfo { body, .. }),
        } = received(&mut events).await
        else {
            panic!("expected the metadata summary first");
        };
        assert!(body.contains("Owner: "), "{body}");
        assert!(body.contains("Contents: Calculating…"), "{body}");
        let body = loop {
            match received(&mut events).await {
                Event::PropertiesUpdated { request, body, .. }
                    if request == DesktopRequestId(9) && !body.contains("Calculating") =>
                {
                    break body;
                }
                Event::PropertiesUpdated { .. } => {}
                other => panic!("unexpected event {other:?}"),
            }
        };
        assert!(body.contains("Contents: 2 files, 1 folder, "), "{body}");
        assert!(
            body.contains(" 1024 bytes") || body.contains(" 1,024 bytes"),
            "{body}"
        );
        assert!(!cancel.is_cancelled());
        drop(desktop);
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }

    #[tokio::test]
    async fn completed_properties_keep_capacity_until_their_terminal_event_is_delivered() {
        let (sender, events) = mpsc::channel(1);
        sender.try_send(Event::VolumesUpdated(Vec::new())).unwrap();
        let mut tasks = JoinSet::new();
        let desktop = Desktop::testing(
            None,
            None,
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        );
        let calls = Arc::new(AtomicUsize::new(0));
        for id in 1..=2 {
            let disposed = Arc::new(Notify::new());
            let filesystem = Arc::new(Icons {
                metadata_calls: Some(calls.clone()),
                _disposed: Some(NotifyOnDrop(disposed.clone())),
                ..Icons::default()
            });
            {
                let mut dispatch = std::pin::pin!(desktop.properties(
                    filesystem,
                    DesktopRequestId(id),
                    vec!["captured/file".into()],
                    Cancellation::default(),
                    &mut tasks,
                ));
                // Accepted work starts on this poll; a rejection waits for output.
                // Cancel only that waiting delivery before observing provider disposal.
                let _ = poll_fn(|context| std::task::Poll::Ready(dispatch.as_mut().poll(context)))
                    .await;
            }
            // The sole filesystem Arc is dropped after the native closure finishes,
            // or after a rejected dispatch is dropped. No timer guesses completion.
            tokio::time::timeout(Duration::from_secs(3), disposed.notified())
                .await
                .unwrap();
        }
        let actual = calls.load(Ordering::SeqCst);
        drop(events);
        drop(desktop);
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        assert_eq!(
            actual, 1,
            "A completed but undelivered Properties request released capacity"
        );
    }

    #[tokio::test]
    async fn unsupported_actions_and_locations_keep_negative_cached_semantic_fallbacks() {
        let services = Arc::new(Icons::default());
        let (sender, mut events) = mpsc::channel(16);
        let outbox = Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        };
        let (icons, receiver) = mpsc::channel(16);
        let worker = tokio::spawn(icon_worker(
            TaskLifetime::testing(services.clone()),
            Some(services.clone()),
            outbox,
            receiver,
        ));
        let missing = IconKey::action(ActionIcon::Back);
        let native = IconKey::action(ActionIcon::Options);
        let home = IconKey {
            kind: IconKind::Location(LocationKind::Home),
            ..IconKey::folder()
        };
        for key in [
            missing.clone(),
            native.clone(),
            missing.clone(),
            IconKey::action(ActionIcon::Help),
            IconKey::action(ActionIcon::View),
            home.clone(),
            home.clone(),
            IconKey {
                kind: IconKind::Location(LocationKind::Downloads),
                ..IconKey::folder()
            },
            IconKey::folder(),
        ] {
            icons.send(key).await.unwrap();
        }
        drop(icons);
        worker.await.unwrap();
        let mut emitted = HashSet::new();
        while let Some(Event::IconLoaded { key, pixels }) = events.recv().await {
            if key == native {
                assert_eq!(&*pixels.rgba, [12, 34, 56, 128]);
            }
            assert_ne!(
                key, missing,
                "generic bitmap replaced the semantic Back fallback"
            );
            emitted.insert(key);
        }
        assert_eq!(emitted, HashSet::from([native, IconKey::folder()]));
        for negative in [missing, home] {
            assert_eq!(
                services
                    .keys
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|key| **key == negative)
                    .count(),
                1
            );
        }
    }

    #[tokio::test]
    async fn transparent_host_link_art_uses_and_caches_the_recognizable_fallback() {
        let services = Arc::new(Icons::default());
        let (sender, mut events) = mpsc::channel(2);
        let outbox = Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        };
        let (icons, receiver) = mpsc::channel(2);
        let worker = tokio::spawn(icon_worker(
            TaskLifetime::testing(services.clone()),
            Some(services.clone()),
            outbox,
            receiver,
        ));
        icons.send(IconKey::folder_link()).await.unwrap();
        icons.send(IconKey::folder_link()).await.unwrap();
        drop(icons);
        worker.await.unwrap();
        match events.recv().await.unwrap() {
            Event::IconLoaded { key, pixels } => {
                assert_eq!(key, IconKey::folder_link());
                assert_eq!(pixels.rgba, link_fallback(true).rgba);
            }
            event => panic!("unexpected icon event: {event:?}"),
        }
        assert!(events.recv().await.is_none());
        assert_eq!(*services.keys.lock().unwrap(), [IconKey::folder_link()]);
    }

    #[tokio::test]
    async fn hotplug_discovery_does_not_wait_for_icons_and_reused_ids_keep_raw_root_keys() {
        let gate = Arc::new(IconGate::default());
        let _release = ReleaseIcon(gate.clone());
        let services = Arc::new(Icons {
            gate: Some(gate.clone()),
            ..Icons::default()
        });

        let (sender, mut events) = mpsc::channel(64);
        let mut engine = crate::Engine::new(
            crate::Engine::lifetime().expect("engine cleanup worker"),
            services.clone(),
            sender,
        );
        let (commands, receiver) = engine.command_channel(8).unwrap();
        engine
            .configure_desktop()
            .unwrap()
            .install(services.clone());
        let task = tokio::spawn(engine.run(receiver));
        tokio::time::timeout(Duration::from_secs(4), gate.entered.notified())
            .await
            .unwrap();
        let first = PathBuf::from("/native/文档-old");
        let second = PathBuf::from("/native/👩🏽‍💻-new");
        for root in [&first, &second] {
            *services.roots.lock().unwrap() = vec![Volume {
                id: "reused-id".into(),
                label: "same label".into(),
                root: root.clone(),
            }];
            commands.send(Command::ListVolumes).await.unwrap();
            loop {
                let event = tokio::time::timeout(Duration::from_secs(4), events.recv())
                    .await
                    .unwrap()
                    .unwrap();
                if let Event::VolumesUpdated(volumes) = event {
                    assert_eq!(volumes.len(), 1);
                    assert_eq!(&volumes[0].root, root);
                    break;
                }
            }
        }
        gate.release();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(4), events.recv())
                .await
                .unwrap()
                .unwrap();
            if let Event::IconLoaded { key, .. } = event
                && key == IconKey::for_drive(&second)
            {
                break;
            }
        }
        {
            let keys = services.keys.lock().unwrap();
            assert!(keys.contains(&IconKey::for_drive(&second)));
        }
        assert_ne!(IconKey::for_drive(&first), IconKey::for_drive(&second));
        commands.send(Command::Shutdown).await.unwrap();
        drop(events);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn latest_volume_roots_replace_pending_icons_under_channel_backpressure() {
        let (roots, receiver) = watch::channel(Vec::new());
        let (icons, mut received) = mpsc::channel(1);
        let sentinel = IconKey::folder();
        icons.send(sentinel.clone()).await.unwrap();
        let forwarder = forward_icons(receiver, icons);
        tokio::pin!(forwarder);
        let latest = IconKey::for_drive(Path::new("/native/新しい-volume"));
        for key in [
            IconKey::for_drive(Path::new("/native/removed")),
            latest.clone(),
        ] {
            roots.send_replace(vec![key]);
            poll_fn(|context| {
                assert!(forwarder.as_mut().poll(context).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
        }
        assert_eq!(received.recv().await, Some(sentinel));
        let forwarded = tokio::select! {
            biased;
            _ = &mut forwarder => panic!("forwarder stopped while receivers were open"),
            key = received.recv() => key,
        };
        assert_eq!(forwarded, Some(latest));
        assert!(received.try_recv().is_err());
        received.close();
        tokio::time::timeout(Duration::from_secs(1), forwarder)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn hotplug_during_native_context_lookup_coalesces_to_current_typed_metadata() {
        let gate = Arc::new(IconGate::default());
        let _release = ReleaseIcon(gate.clone());
        let old = PathBuf::from("/native/old");
        let current = PathBuf::from("/native/新しい-drive");
        let volume = |root: PathBuf| Volume {
            id: "same-id".into(),
            label: "native name".into(),
            root,
        };
        let services = Arc::new(Icons {
            roots: Mutex::new(vec![volume(old)]),
            context_gate: Some(gate.clone()),
            ..Icons::default()
        });

        let (sender, mut events) = mpsc::channel(32);
        let mut engine = crate::Engine::new(
            crate::Engine::lifetime().expect("engine cleanup worker"),
            services.clone(),
            sender,
        );
        let (commands, receiver) = engine.command_channel(8).unwrap();
        engine
            .configure_desktop()
            .unwrap()
            .install(services.clone());
        let task = tokio::spawn(engine.run(receiver));
        commands.send(Command::ListDesktopContext).await.unwrap();
        tokio::time::timeout(Duration::from_secs(4), gate.entered.notified())
            .await
            .unwrap();
        *services.roots.lock().unwrap() = vec![volume(current.clone())];
        commands.send(Command::ListVolumes).await.unwrap();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(4), events.recv())
                .await
                .unwrap()
                .unwrap();
            if let Event::VolumesUpdated(volumes) = event {
                assert_eq!(volumes[0].root, current);
                break;
            }
        }
        assert_eq!(services.context_calls.load(Ordering::SeqCst), 1);
        gate.release();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(4), events.recv())
                .await
                .unwrap()
                .unwrap();
            if let Event::DesktopContext { locations, .. } = event {
                assert_eq!(locations.len(), 1);
                assert_eq!(locations[0].path, current, "stale context was published");
                assert_eq!(locations[0].drive_letter, Some('Z'));
                break;
            }
        }
        assert_eq!(services.context_calls.load(Ordering::SeqCst), 2);
        commands.send(Command::Shutdown).await.unwrap();
        drop(events);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn icon_cache_deduplicates_and_keeps_loading_new_types_beyond_256() {
        let (sender, mut events) = mpsc::channel(512);
        let outbox = Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        };
        let (icons, receiver) = mpsc::channel(512);
        let worker = tokio::spawn(icon_worker(
            TaskLifetime::testing(()),
            None,
            outbox,
            receiver,
        ));
        for i in 0..300 {
            let key = IconKey {
                extension: Some(format!("type-{i}").into()),
                ..IconKey::file()
            };
            icons.send(key.clone()).await.unwrap();
            icons.send(key).await.unwrap();
        }
        drop(icons);
        worker.await.unwrap();
        let mut keys = HashSet::new();
        while let Some(Event::IconLoaded { key, pixels }) = events.recv().await {
            // Immediate repeats are deduplicated; the bounded cache forgets old
            // types instead of refusing every type after the 256th.
            assert!(keys.insert(key));
            assert_eq!(
                pixels.rgba.len(),
                (pixels.width * pixels.height * 4) as usize
            );
        }
        assert_eq!(keys.len(), 300);
    }
}

#[cfg(test)]
#[path = "desktop_lifetime_tests.rs"]
mod lifetime_tests;

#[cfg(test)]
#[path = "desktop_external_compare_ledger_tests.rs"]
mod external_compare_ledger_tests;

impl Drop for Desktop {
    fn drop(&mut self) {
        self.command_icons.borrow().cancel.cancel();
    }
}
