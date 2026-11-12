//! Production tree lifecycle. Two bounded model lanes own all sessions on workers.
//!
//! Each pane retains eight commands, one completion, one coalesced root notification
//! and one navigation reset. Shared views keep the existing 10,000-row/16 MiB tree
//! limits; enable requests exceeding that row bound never enter the queue. Only one
//! provider branch per pane can run, using the pane's persistent/common I/O budget.
//! A closed lane drains its payloads on its model worker. A blocked provider keeps
//! its resources and permit until it returns; neither navigation nor shutdown
//! invents a replacement permit or waits on the UI thread.
use crate::{Outbox, Snapshots, providers::NativeProvider, tree_actor::WorkerBudget};
use midnight_core::{
    Cancellation, FileSystem, LocaleService, NavigationId, PaneId, SortSpec, TreeCommand,
    TreeDiagnostic, TreeEvent, TreeProjection, TreeRequestId, TreeRowId,
};
use std::sync::{Arc, Weak};
use tokio::{sync::Semaphore, task::JoinHandle};

mod mailbox;
mod worker;
use mailbox::{Generation, Mailbox};

pub(crate) const CAPTURE_CAPACITY: usize = 4;

pub(crate) struct Services {
    pub(crate) rename: crate::tree_rename::Retirement,
    pub(crate) filesystem: Arc<dyn FileSystem>,
    pub(crate) locale: Option<Arc<dyn LocaleService>>,
    pub(crate) reports: crate::ReportArena,
}

pub(crate) struct Controller {
    lanes: [Arc<Mailbox>; 2],
    workers: [Option<JoinHandle<()>>; 2],
    watch: Option<crate::watch::Handle>,
}

/// Completion notifications carry no provider resources and coalesce per pane.
#[derive(Clone)]
pub(crate) struct RootReady(Weak<Mailbox>);
impl RootReady {
    pub(crate) fn notify(&self, navigation: NavigationId) {
        if let Some(lane) = self.0.upgrade() {
            lane.ready(navigation);
        }
    }
}

impl Controller {
    pub(crate) fn new(
        services: Services,
        listings: [Arc<Semaphore>; 2],
        mutation_gate: Arc<Semaphore>,
        snapshots: Snapshots,
        outbox: Outbox,
        watch: Option<crate::watch::Handle>,
    ) -> Self {
        let Services {
            filesystem,
            locale,
            reports,
            rename,
        } = services;
        let lanes = std::array::from_fn(|_| Arc::new(Mailbox::default()));
        {
            let mut states = snapshots.lock().unwrap_or_else(|e| e.into_inner());
            for pane in 0..2 {
                states[pane].tree_notify = Some(RootReady(Arc::downgrade(&lanes[pane])));
            }
        }
        let copy_capacity = Arc::new(Semaphore::new(CAPTURE_CAPACITY));
        let workers = std::array::from_fn(|pane| {
            let mut native = NativeProvider::new(filesystem.clone());
            if let Some(locale) = &locale {
                native = native.with_locale(locale.clone());
            }
            let mut worker = worker::Worker::new(
                lanes[pane].clone(),
                snapshots.clone(),
                worker::Services {
                    native,
                    locale: locale.clone(),
                    reports: reports.clone(),
                    rename: rename.clone(),
                },
                WorkerBudget::new(listings[pane].clone()).with_mutation_gate(mutation_gate.clone()),
                outbox.clone(),
                copy_capacity.clone(),
            );
            worker.watch = watch.clone();
            Some(tokio::task::spawn_blocking(move || worker.run()))
        });
        Self {
            lanes,
            workers,
            watch,
        }
    }

    pub(crate) async fn submit(&self, mut command: TreeCommand) -> Option<TreeEvent> {
        let pane = command_pane(&command);
        if let TreeCommand::Enable { root, selected, .. } = &command {
            let oversized_selection = matches!(selected,
                midnight_core::TreeRootSelection::Archive(ids) if ids.capacity() > TreeProjection::MAX_ROWS * 2);
            if oversized_selection
                || crate::tree_actor::preflight_root(root, &Cancellation::default()).is_err()
            {
                return reject_off_thread(command, "Tree root exceeds its resource budget.", false)
                    .await;
            }
        }
        if let TreeCommand::Reproject(request) = &command {
            if let Err(message) = request
                .options
                .filter
                .validate_bounds()
                .and_then(|()| request.options.search.validate_bounds())
            {
                return reject_off_thread(command, message, false).await;
            }
            if request.options.filter.expression.capacity() > midnight_core::MAX_FILTER_BYTES
                || request.options.search.query.capacity()
                    > midnight_core::TreeSearchOptions::MAX_QUERY_BYTES
            {
                // Valid short search/filter text can arrive with an oversized backing
                // allocation. Normalize and release it on this one awaited worker,
                // before the command can occupy a bounded mailbox slot.
                let stamp = request.view.stamp;
                let request_id = request.request;
                command = match tokio::task::spawn_blocking(move || {
                    if let TreeCommand::Reproject(request) = &mut command {
                        request.options.filter.expression =
                            request.options.filter.expression.as_str().to_owned();
                        request.options.search.query =
                            request.options.search.query.as_str().to_owned();
                    }
                    command
                })
                .await
                {
                    Ok(command) => command,
                    Err(_) => {
                        return Some(TreeEvent::ReprojectFailed {
                            stamp,
                            request: request_id,
                            retryable: false,
                            error: TreeDiagnostic::new("Tree options preparation failed."),
                        });
                    }
                };
            }
        }
        match self.lanes[pane.index()].push(command) {
            Ok(()) => None,
            Err(mailbox::PushError::Superseded(command)) => {
                // Releasing the last old projection can be expensive. Keep it
                // off dispatch and await disposal so retired views cannot pile up.
                let _ = tokio::task::spawn_blocking(move || drop(command)).await;
                None
            }
            Err(mailbox::PushError::Rejected(command)) => {
                reject_off_thread(
                    command,
                    "Tree command queue is full or its view/generation changed.",
                    true,
                )
                .await
            }
        }
    }

    pub(crate) fn invalidate(&self, pane: PaneId, navigation: NavigationId, sort: SortSpec) {
        if let Some(watch) = &self.watch {
            watch.clear_scope(pane, navigation);
        }
        self.lanes[pane.index()].reset(Generation {
            navigation,
            sort,
            cancel: Cancellation::default(),
        });
    }

    pub(crate) async fn shutdown(mut self) {
        if let Some(watch) = &self.watch {
            watch.shutdown();
        }
        for lane in &self.lanes {
            lane.close();
        }
        for worker in &mut self.workers {
            if let Some(worker) = worker.take() {
                // Idle workers terminate immediately. Uncancellable provider calls
                // keep bounded worker ownership until their actual return.
                let _ = tokio::time::timeout(std::time::Duration::from_millis(200), worker).await;
            }
        }
    }
}
async fn reject_off_thread(
    command: TreeCommand,
    message: &'static str,
    retryable: bool,
) -> Option<TreeEvent> {
    // Engine dispatch awaits this one worker, so rejected large payload cleanup
    // has one bounded owner and cannot accumulate unbounded detached tasks.
    tokio::task::spawn_blocking(move || rejection(command, message, retryable))
        .await
        .ok()
}
impl Drop for Controller {
    fn drop(&mut self) {
        if let Some(watch) = &self.watch {
            watch.shutdown();
        }
        for lane in &self.lanes {
            lane.close();
        }
    }
}

/// Commit and viewport are observations, not user actions. A newer publication
/// makes an older observation obsolete; never retry it or turn it into an error.
/// All explicit actions and foreign/current/future views retain strict checks.
fn superseded_observation(command: &TreeCommand, current: midnight_core::TreeViewStamp) -> bool {
    let view = match command {
        TreeCommand::Commit(view) | TreeCommand::Viewport { view, .. } => view,
        _ => return false,
    };
    let stamp = view.stamp;
    stamp.pane == current.pane
        && stamp.navigation == current.navigation
        && stamp.session == current.session
        && stamp.revision.0 < current.revision.0
}

fn command_pane(command: &TreeCommand) -> PaneId {
    match command {
        TreeCommand::Sizes(command) => size_command_stamp(command).pane,
        TreeCommand::Enable { root, .. } => root.pane,
        TreeCommand::Commit(view)
        | TreeCommand::CaptureDropDirectory { view, .. }
        | TreeCommand::Branch { view, .. }
        | TreeCommand::Viewport { view, .. }
        | TreeCommand::Navigate { view, .. } => view.stamp.pane,
        TreeCommand::Transfer(request) => request.view.stamp.pane,
        TreeCommand::Delete(request) => request.view.stamp.pane,
        TreeCommand::Clipboard(request) => request.view.stamp.pane,
        TreeCommand::Edit(request) => request.view.stamp.pane,
        TreeCommand::Metadata(request) => request.view.stamp.pane,
        TreeCommand::Rename(request) => request.view.stamp.pane,
        TreeCommand::Checksums(request) => request.view.stamp.pane,
        TreeCommand::UserCommand(request) => request.view.stamp.pane,
        TreeCommand::Select(request) => request.view.stamp.pane,
        TreeCommand::Reproject(request) => request.view.stamp.pane,
        TreeCommand::RefreshLoaded(request) => request.view.stamp.pane,
    }
}
fn command_navigation(command: &TreeCommand) -> NavigationId {
    match command {
        TreeCommand::Sizes(command) => size_command_stamp(command).navigation,
        TreeCommand::Enable { root, .. } => root.navigation,
        TreeCommand::Commit(view)
        | TreeCommand::CaptureDropDirectory { view, .. }
        | TreeCommand::Branch { view, .. }
        | TreeCommand::Viewport { view, .. }
        | TreeCommand::Navigate { view, .. } => view.stamp.navigation,
        TreeCommand::Transfer(request) => request.view.stamp.navigation,
        TreeCommand::Delete(request) => request.view.stamp.navigation,
        TreeCommand::Clipboard(request) => request.view.stamp.navigation,
        TreeCommand::Edit(request) => request.view.stamp.navigation,
        TreeCommand::Metadata(request) => request.view.stamp.navigation,
        TreeCommand::Rename(request) => request.view.stamp.navigation,
        TreeCommand::Checksums(request) => request.view.stamp.navigation,
        TreeCommand::UserCommand(request) => request.view.stamp.navigation,
        TreeCommand::Select(request) => request.view.stamp.navigation,
        TreeCommand::Reproject(request) => request.view.stamp.navigation,
        TreeCommand::RefreshLoaded(request) => request.view.stamp.navigation,
    }
}
fn rejection(command: TreeCommand, message: &str, retryable: bool) -> TreeEvent {
    diagnostic_rejection(command, TreeDiagnostic::new(message), retryable)
}
fn diagnostic_rejection(command: TreeCommand, error: TreeDiagnostic, retryable: bool) -> TreeEvent {
    match command {
        TreeCommand::CaptureDropDirectory {
            id, view, request, ..
        } => TreeEvent::DropDirectoryCaptured {
            stamp: view.stamp,
            request,
            id,
            result: Err(error),
        },
        TreeCommand::RefreshLoaded(request) => TreeEvent::RefreshRejected {
            stamp: request.view.stamp,
            request: request.request,
            error,
            retryable,
        },
        TreeCommand::Sizes(command) => TreeEvent::Sizes(midnight_core::TreeSizeEvent::Rejected {
            stamp: size_command_stamp(&command),
            request: match command {
                midnight_core::TreeSizeCommand::Compute(request) => request.request,
                midnight_core::TreeSizeCommand::Cancel { request, .. } => request,
                midnight_core::TreeSizeCommand::Acknowledge { .. } => TreeRequestId(0),
            },
            error,
        }),
        TreeCommand::Enable { root, .. } => TreeEvent::Unavailable { root, error },
        TreeCommand::Reproject(request) => TreeEvent::ReprojectFailed {
            stamp: request.view.stamp,
            request: request.request,
            retryable,
            error,
        },
        TreeCommand::Delete(request) => TreeEvent::DeleteCaptured {
            stamp: request.view.stamp,
            request: request.request,
            result: Err(error),
        },
        TreeCommand::Transfer(request) => TreeEvent::Transfer {
            stamp: request.view.stamp,
            request: request.request,
            result: Err(error),
        },
        TreeCommand::Metadata(request) => TreeEvent::MetadataCaptured {
            stamp: request.view.stamp,
            request: request.request,
            retryable,
            result: Err(error),
        },
        TreeCommand::Rename(request) => TreeEvent::RenameCaptured {
            stamp: request.view.stamp,
            request: request.request,
            result: Err(error),
        },
        TreeCommand::Edit(request) => TreeEvent::EditorCaptured {
            stamp: request.view.stamp,
            request: request.request,
            result: Err(error),
        },
        TreeCommand::UserCommand(request) => TreeEvent::UserCommandCaptured {
            stamp: request.view.stamp,
            request: request.request,
            retryable,
            result: Err(error),
        },
        TreeCommand::Checksums(request) => TreeEvent::ChecksumsCaptured {
            stamp: request.view.stamp,
            request: request.request,
            result: Err(error),
        },
        TreeCommand::Clipboard(request) => TreeEvent::Clipboard {
            stamp: request.view.stamp,
            request: request.request,
            result: Err(error),
        },
        TreeCommand::Select(request) => TreeEvent::SelectionFailed {
            stamp: request.view.stamp,
            revision: request.revision,
            error,
        },
        TreeCommand::Branch {
            view, request, row, ..
        }
        | TreeCommand::Navigate {
            view, request, row, ..
        } => TreeEvent::Rejected {
            stamp: view.stamp,
            request,
            row,
            error,
        },
        TreeCommand::Commit(view) | TreeCommand::Viewport { view, .. } => TreeEvent::Rejected {
            stamp: view.stamp,
            request: TreeRequestId(0),
            row: TreeRowId(0),
            error,
        },
    }
}

fn size_command_stamp(command: &midnight_core::TreeSizeCommand) -> midnight_core::TreeViewStamp {
    match command {
        midnight_core::TreeSizeCommand::Compute(request) => request.view.stamp,
        midnight_core::TreeSizeCommand::Acknowledge { stamp, .. }
        | midnight_core::TreeSizeCommand::Cancel { stamp, .. } => *stamp,
    }
}

#[cfg(test)]
mod lifetime_tests;
