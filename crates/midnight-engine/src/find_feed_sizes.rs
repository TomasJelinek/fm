//! At most one active and one coalesced folder-size request per Find-fed pane.
use crate::{Outbox, ReportArena, folder_sizes, task_lifetime::TaskLifetime};
use midnight_core::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::{
    sync::Semaphore,
    task::{AbortHandle, JoinSet},
};

struct Done(Arc<AtomicBool>);
impl Drop for Done {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

struct Active {
    cancel: Cancellation,
    done: Arc<AtomicBool>,
    task: AbortHandle,
}
impl Drop for Active {
    fn drop(&mut self) {
        if !self.done.load(Ordering::Acquire) {
            self.cancel.cancel();
            self.task.abort();
        }
    }
}

#[derive(Default)]
struct Slot {
    active: Option<Active>,
    pending: Option<Arc<FindFeedSizeRequest>>,
}

pub(crate) struct Controller {
    filesystem: Arc<dyn FileSystem>,
    outbox: Outbox,
    budget: Arc<Semaphore>,
    delivery_cancel: Cancellation,
    io_budget: Arc<folder_sizes::IoBudget>,
    scope: TaskLifetime,
    reports: ReportArena,
    slots: [Slot; 2],
}

impl Controller {
    pub(crate) fn new(
        filesystem: Arc<dyn FileSystem>,
        outbox: Outbox,
        budget: Arc<Semaphore>,
        delivery_cancel: Cancellation,
        io_budget: Arc<folder_sizes::IoBudget>,
        scope: TaskLifetime,
        reports: ReportArena,
    ) -> Self {
        Self {
            filesystem,
            outbox,
            budget,
            delivery_cancel,
            io_budget,
            scope,
            reports,
            slots: Default::default(),
        }
    }

    pub(crate) fn compute(&mut self, request: Arc<FindFeedSizeRequest>, tasks: &mut JoinSet<()>) {
        let slot = &mut self.slots[request.pane.index()];
        if let Some(old) = slot.pending.replace(request) {
            old.cancel.cancel();
        }
        if let Some(active) = &slot.active {
            // Its wrapper must await the blocking scan: JoinSet completion wakes
            // the actor to admit the replacement only after that worker leaves.
            active.cancel.cancel();
        }
        self.tick(tasks);
    }

    pub(crate) fn tick(&mut self, tasks: &mut JoinSet<()>) {
        for pane in [PaneId::Left, PaneId::Right] {
            let slot = &mut self.slots[pane.index()];
            if slot
                .active
                .as_ref()
                .is_some_and(|active| active.done.load(Ordering::Acquire))
            {
                slot.active = None;
            }
            if slot.active.is_some() {
                continue;
            }
            let Some(request) = slot.pending.take() else {
                continue;
            };
            if request.cancel.is_cancelled() {
                continue;
            }
            let done = Arc::new(AtomicBool::new(false));
            let done_guard = Done(done.clone());
            let cancel = request.cancel.clone();
            let filesystem = self.filesystem.clone();
            let outbox = self.outbox.clone();
            let budget = self.budget.clone();
            let delivery_cancel = self.delivery_cancel.clone();
            let io_budget = self.io_budget.clone();
            let reports = self.reports.clone();
            let scope = self.scope.clone();
            let final_outbox = outbox.clone();
            let identity = (request.pane, request.id, request.rows_serial);
            let task = tasks.spawn(self.scope.future(async move {
                let Ok(permit) = budget.acquire_owned().await else {
                    return;
                };
                let result = tokio::task::spawn_blocking(scope.blocking(move || {
                    let _done = done_guard;
                    let _permit = permit;
                    scan(
                        &request,
                        filesystem.as_ref(),
                        &io_budget,
                        &reports,
                        &mut |event| {
                            outbox.blocking_send(
                                Event::FindFeedSize(Box::new(event)),
                                &delivery_cancel,
                            )
                        },
                    );
                }))
                .await;
                if result.is_err() {
                    final_outbox
                        .send(Event::FindFeedSize(Box::new(FindFeedSizeEvent {
                            pane: identity.0,
                            id: identity.1,
                            rows_serial: identity.2,
                            update: FindFeedSizeUpdate::Finished {
                                cancelled: true,
                                error: Some(FsError::from_io(
                                    "measure Find folders",
                                    std::path::Path::new(""),
                                    std::io::Error::other(
                                        "The folder size worker stopped before it finished.",
                                    ),
                                )),
                            },
                        })))
                        .await;
                }
            }));
            slot.active = Some(Active { cancel, done, task });
        }
    }

    pub(crate) fn cancel_all(&mut self) {
        for slot in &mut self.slots {
            if let Some(request) = slot.pending.take() {
                request.cancel.cancel();
            }
            // Retain an active worker's slot until its blocking scan really leaves.
            if let Some(active) = &slot.active {
                active.cancel.cancel();
                active.task.abort();
            }
        }
    }

    pub(crate) fn busy(&self) -> bool {
        self.slots.iter().any(|slot| {
            slot.pending.is_some()
                || slot
                    .active
                    .as_ref()
                    .is_some_and(|active| !active.done.load(Ordering::Acquire))
        })
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

fn scan(
    request: &FindFeedSizeRequest,
    filesystem: &dyn FileSystem,
    budget: &Arc<folder_sizes::IoBudget>,
    reports: &ReportArena,
    emit: &mut dyn FnMut(FindFeedSizeEvent) -> bool,
) {
    let event = |update| FindFeedSizeEvent {
        pane: request.pane,
        id: request.id,
        rows_serial: request.rows_serial,
        update,
    };
    if request.cancel.is_cancelled() {
        emit(event(FindFeedSizeUpdate::Finished {
            cancelled: true,
            error: None,
        }));
        return;
    }
    let roots = match request.roots() {
        Ok(roots) => roots,
        Err(message) => {
            emit(event(FindFeedSizeUpdate::Finished {
                cancelled: false,
                error: Some(FsError::from_io(
                    "measure Find folders",
                    std::path::Path::new(""),
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, message),
                )),
            }));
            return;
        }
    };
    let mut delivered = true;
    folder_sizes::compute_batched(
        filesystem,
        &roots,
        &request.cancel,
        &mut |entry, size| {
            let sent = emit(event(FindFeedSizeUpdate::Updated { entry, size }));
            delivered &= sent;
            sent && !request.cancel.is_cancelled()
        },
        budget,
        reports,
    );
    emit(event(FindFeedSizeUpdate::Finished {
        cancelled: request.cancel.is_cancelled() || !delivered,
        error: None,
    }));
}

#[cfg(test)]
#[path = "find_feed_sizes_tests.rs"]
mod tests;
