//! Backend-owned confirmation and FIFO mutation jobs, independent of pane navigation.
use crate::copy_fence::{CopyFence, Predecessors};
use crate::{Outbox, Snapshots};
use midnight_core::*;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinSet,
};

#[path = "operation_metadata.rs"]
mod metadata;
#[path = "operation_rename.rs"]
mod rename;
#[path = "operation_worker.rs"]
mod worker;
type Status = Arc<Mutex<OperationProgress>>;
type Registry = Arc<Mutex<HashMap<OperationId, Registered>>>;
struct Registered {
    cancel: Cancellation,
    status: Status,
    confirmation: Option<Arc<Predecessors>>,
}
struct ConfirmationDelivery {
    registry: Registry,
    id: OperationId,
    queued: bool,
}
impl Drop for ConfirmationDelivery {
    fn drop(&mut self) {
        if !self.queued {
            release_confirmation(&self.registry, self.id);
        }
    }
}
type Running = (
    Status,
    Result<OperationProgress, tokio::task::JoinError>,
    Option<Arc<MetadataResult>>,
);
const MAX_JOBS: usize = 32;

enum Message {
    Prepare(PrepareOperation, Cancellation, Status),
    Confirm(OperationId, Option<String>, Arc<Predecessors>),
    PreviewMetadata(OperationId, u64, MetadataPatch),
    ConfirmMetadata(OperationId, u64, MetadataConflictPolicy, Arc<Predecessors>),
    ResolveMetadataConflict(OperationId, u64, MetadataConflictPolicy),
    DecideRename(CopyDecision),
    Wake,
}

pub(crate) struct Controller {
    sender: mpsc::Sender<Message>,
    registry: Registry,
    highest: AtomicU64,
    cancel: Cancellation,
    snapshots: Snapshots,
    outbox: Outbox,
    copies: CopyFence,
}

impl Controller {
    pub(crate) fn new(
        service: Option<Arc<dyn MutationService>>,
        locale: Option<Arc<dyn LocaleService>>,
        snapshots: Snapshots,
        outbox: Outbox,
        mutation_gate: Arc<Semaphore>,
        copies: CopyFence,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(64);
        let registry = Registry::default();
        let cancel = Cancellation::default();
        tasks.spawn(run(
            receiver,
            service,
            locale,
            registry.clone(),
            cancel.clone(),
            outbox.clone(),
            mutation_gate,
        ));
        Self {
            sender,
            registry,
            highest: AtomicU64::new(0),
            cancel,
            snapshots,
            outbox,
            copies,
        }
    }

    pub(crate) async fn prepare(&self, request: PrepareOperation) {
        // A process-wide high-water mark makes replay idempotent without an unbounded ID set.
        if request.id.0 <= self.highest.fetch_max(request.id.0, Ordering::Relaxed) {
            return;
        }
        let mut progress = worker::initial(&request);
        let valid = {
            let snapshots = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
            let state = &snapshots[request.source.pane.index()];
            state.has_native_view()
                && state.pending.is_none()
                && state.visible.as_ref().is_some_and(|visible| {
                    visible.navigation == request.source.navigation
                        && visible.path == request.source.path
                        && Arc::ptr_eq(&visible.entries, &request.source.entries)
                })
        };
        let rejection = if !valid {
            Some("The directory changed. Try the operation again.")
        } else if self.cancel.is_cancelled() {
            Some("The application is closing.")
        } else if self
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
            >= MAX_JOBS
        {
            Some("The operation queue is full (32 jobs).")
        } else {
            None
        };
        if let Some(message) = rejection {
            progress.state = OperationState::Failed;
            progress.message = message.into();
            publish(&self.outbox, &progress).await;
            return;
        }
        let cancel = if matches!(request.kind, MutationKind::Delete(_)) {
            self.cancel.pausable_child()
        } else {
            self.cancel.child()
        };
        let status = Arc::new(Mutex::new(progress.clone()));
        self.registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                request.id,
                Registered {
                    cancel: cancel.clone(),
                    status: status.clone(),
                    confirmation: None,
                },
            );
        publish(&self.outbox, &progress).await;
        if self
            .sender
            .try_send(Message::Prepare(request, cancel, status))
            .is_err()
        {
            self.registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&progress.id);
            progress.state = OperationState::Failed;
            progress.message = "The operation queue is unavailable.".into();
            publish(&self.outbox, &progress).await;
        }
    }

    pub(crate) async fn confirm(&self, id: OperationId, name: Option<String>) {
        if let Some(predecessors) = self.capture_confirmation(id) {
            self.send_confirmation(id, Message::Confirm(id, name, predecessors))
                .await;
        }
    }

    async fn send_confirmation(&self, id: OperationId, message: Message) {
        // A cancelled send must allow retry without retaining a phantom confirmation.
        let mut delivery = ConfirmationDelivery {
            registry: self.registry.clone(),
            id,
            queued: false,
        };
        delivery.queued = self.sender.send(message).await.is_ok();
    }

    fn capture_confirmation(&self, id: OperationId) -> Option<Arc<Predecessors>> {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let job = registry.get_mut(&id)?;
        let status = job.status.lock().unwrap_or_else(|e| e.into_inner());
        if job.confirmation.is_some() || status.state != OperationState::AwaitingConfirmation {
            return None;
        }
        let predecessors = Arc::new(
            if matches!(
                status.kind,
                MutationKind::CreateDirectory | MutationKind::CreateFile
            ) {
                Predecessors::default()
            } else {
                self.copies.snapshot()
            },
        );
        job.confirmation = Some(predecessors.clone());
        Some(predecessors)
    }
    pub(crate) async fn preview_metadata(
        &self,
        id: OperationId,
        revision: u64,
        patch: MetadataPatch,
    ) {
        let _ = self
            .sender
            .send(Message::PreviewMetadata(id, revision, patch))
            .await;
    }
    pub(crate) async fn confirm_metadata(
        &self,
        id: OperationId,
        revision: u64,
        conflicts: MetadataConflictPolicy,
    ) {
        if let Some(predecessors) = self.capture_confirmation(id) {
            self.send_confirmation(
                id,
                Message::ConfirmMetadata(id, revision, conflicts, predecessors),
            )
            .await;
        }
    }
    pub(crate) async fn resolve_metadata_conflict(
        &self,
        id: OperationId,
        token: u64,
        choice: MetadataConflictPolicy,
    ) {
        let _ = self
            .sender
            .send(Message::ResolveMetadataConflict(id, token, choice))
            .await;
    }
    pub(crate) async fn decide_rename(&self, decision: CopyDecision) {
        let _ = self.sender.send(Message::DecideRename(decision)).await;
    }
    pub(crate) async fn set_paused(&self, id: OperationId, paused: bool) {
        let progress = {
            let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            let Some(job) = registry.get(&id) else { return };
            let mut progress = job.status.lock().unwrap_or_else(|e| e.into_inner());
            if !matches!(progress.kind, MutationKind::Delete(_))
                || progress.state.is_terminal()
                || job.cancel.is_cancelled()
            {
                return;
            }
            job.cancel.set_pause_requested(paused);
            progress.pause_requested = job.cancel.pause_requested();
            progress.paused = job.cancel.is_paused();
            progress.clone()
        };
        publish(&self.outbox, &progress).await;
    }
    pub(crate) fn cancel(&self, id: OperationId) {
        if let Some(job) = self
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
        {
            job.cancel.cancel();
        }
        let _ = self.sender.try_send(Message::Wake);
    }
    pub(crate) fn shutdown(&self) {
        self.cancel.cancel();
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn publish(outbox: &Outbox, progress: &OperationProgress) {
    outbox
        .send(Event::Operation(OperationEvent::Progress(Arc::new(
            progress.clone(),
        ))))
        .await;
}
fn release_confirmation(registry: &Registry, id: OperationId) {
    if let Some(job) = registry
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(&id)
    {
        job.confirmation = None;
    }
}
async fn finish(registry: &Registry, outbox: &Outbox, progress: OperationProgress) {
    registry
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&progress.id);
    publish(outbox, &progress).await;
}

async fn run(
    mut receiver: mpsc::Receiver<Message>,
    service: Option<Arc<dyn MutationService>>,
    locale: Option<Arc<dyn LocaleService>>,
    registry: Registry,
    cancel: Cancellation,
    outbox: Outbox,
    mutation_gate: Arc<Semaphore>,
) {
    let budget = Arc::new(Semaphore::new(2));
    let metadata_budget = Arc::new(Semaphore::new(MAX_METADATA_TARGETS));
    let mut preparing = JoinSet::new();
    let mut running = JoinSet::<Running>::new();
    let mut disposing = JoinSet::new();
    let mut prepared = HashMap::<OperationId, worker::Job>::new();
    let mut drafts = HashMap::<OperationId, (u64, MetadataPatch)>::new();
    let mut conflict_states = HashMap::<OperationId, Arc<metadata::ConflictState>>::new();
    let mut queued = VecDeque::<worker::Job>::new();
    let mut timer = tokio::time::interval(Duration::from_millis(50));
    let mut rename_states: HashMap<OperationId, Arc<rename::ConflictState>> = HashMap::new();
    let mut accepting = true;
    loop {
        tokio::select! {
            message = receiver.recv(), if accepting => match message {
                Some(Message::Prepare(request, job_cancel, status)) => {
                    let budget = budget.clone();
                    let service = service.clone();
                    let locale = locale.clone();
                    let metadata_budget = metadata_budget.clone();
                    preparing.spawn(async move {
                        let original = status.clone();
                        let permit = budget.acquire_owned().await.expect("operation semaphore stays open");
                        let result = tokio::task::spawn_blocking(move || {
                            let _permit = permit;
                            worker::prepare(request, service, job_cancel, status, metadata_budget).map(|mut job| {
                                if let Some(edit) = &mut job.metadata { edit.localize(locale.as_deref()); }
                                job
                            })
                        }).await;
                        (original, result)
                    });
                }
                Some(Message::Confirm(id, name, predecessors)) => {
                    if let Some(mut job) = prepared.remove(&id) {
                        if let Err(error) = job.confirm(name) {
                            disposing.spawn(job.failure(error));
                        } else {
                            if !matches!(job.progress().kind, MutationKind::CreateDirectory | MutationKind::CreateFile) {
                                job.predecessors = predecessors;
                            }
                            job.set_state(OperationState::Queued);
                            publish(&outbox, &job.progress()).await;
                            queued.push_back(job);
                        }
                    } else { release_confirmation(&registry, id); }
                }
                Some(Message::PreviewMetadata(id, revision, patch)) => {
                    let awaiting = registry.lock().unwrap_or_else(|e| e.into_inner()).get(&id)
                        .is_some_and(|job| {
                            let progress = job.status.lock().unwrap_or_else(|e| e.into_inner());
                            progress.kind == MutationKind::EditMetadata && matches!(progress.state, OperationState::Preparing | OperationState::AwaitingConfirmation)
                        });
                    if awaiting && revision > 0 && drafts.get(&id).is_none_or(|(latest, _)| revision > *latest) {
                        if let Some(job) = prepared.remove(&id) {
                            if job.metadata.as_ref().is_some_and(|edit| revision > edit.preview.revision) {
                                spawn_preview(&mut preparing, job, revision, patch, budget.clone(), locale.clone());
                            } else { prepared.insert(id, job); }
                        } else { drafts.insert(id, (revision, patch)); }
                    }
                }
                Some(Message::ConfirmMetadata(id, revision, conflicts, predecessors)) => {
                    let valid = !drafts.contains_key(&id) && prepared.get(&id).is_some_and(|job|
                        job.metadata.as_ref().is_some_and(|edit| edit.preview.revision == revision && !edit.preview.patch.is_empty()
                            && edit.preview.unsupported == 0));
                    if valid && let Some(mut job) = prepared.remove(&id) {
                        if conflicts == MetadataConflictPolicy::Cancel && job.metadata.as_ref().is_some_and(|edit| edit.preview.conflicts > 0) {
                            disposing.spawn(job.cancelled());
                            continue;
                        }
                        job.metadata.as_mut().expect("validated metadata job").conflicts = conflicts;
                        job.predecessors = predecessors;
                        job.set_state(OperationState::Queued);
                        publish(&outbox, &job.progress()).await;
                        queued.push_back(job);
                    } else { release_confirmation(&registry, id); }
                }
                Some(Message::ResolveMetadataConflict(id, token, choice)) => {
                    if let Some(state) = conflict_states.get(&id) { state.resolve(token, choice); }
                }
                Some(Message::DecideRename(decision)) => {
                    if let Some(state) = rename_states.get(&decision.id) { state.resolve(decision); }
                }
                Some(Message::Wake) => {}
                None => { accepting = false; cancel.cancel(); }
            },
            Some(result) = preparing.join_next(), if !preparing.is_empty() => {
                let Ok((status, result)) = result else { continue; };
                match result {
                    Ok(Ok(job)) if !job.cancel.is_cancelled() => {
                        let id = job.progress().id;
                        if let Some((revision, patch)) = drafts.remove(&id)
                            && job.metadata.as_ref().is_some_and(|edit| revision > edit.preview.revision) {
                            spawn_preview(&mut preparing, job, revision, patch, budget.clone(), locale.clone());
                            continue;
                        }
                        job.set_state(OperationState::AwaitingConfirmation);
                        publish(&outbox, &job.progress()).await;
                        outbox.send(Event::Operation(OperationEvent::Preview(Arc::new(job.preview())))).await;
                        if let Some(edit) = &job.metadata {
                            conflict_states.insert(id, edit.conflict_state.clone());
                            outbox.send(Event::Operation(OperationEvent::MetadataPreview(Arc::new(edit.preview.clone())))).await;
                        }
                        if job.progress().kind == MutationKind::Rename {
                            rename_states.insert(id, job.rename_conflicts.clone());
                        }
                        prepared.insert(job.progress().id, job);
                    }
                    Ok(Ok(job)) => { disposing.spawn(job.cancelled()); }
                    Ok(Err(error)) => {
                        let progress = worker::failed_status(&status, error);
                        finish(&registry, &outbox, progress).await;
                    }
                    Err(error) => {
                        let progress = worker::panicked(&status, error.to_string());
                        finish(&registry, &outbox, progress).await;
                    }
                }
            },
            Some(result) = running.join_next(), if !running.is_empty() => {
                if let Ok((status, result, metadata_result)) = result {
                    let progress = result.unwrap_or_else(|error| worker::panicked(&status, error.to_string()));
                    if let Some(result) = metadata_result {
                        outbox.send(Event::Operation(OperationEvent::MetadataResult(result))).await;
                    }
                    finish(&registry, &outbox, progress).await;
                }
            },
            Some(result) = disposing.join_next(), if !disposing.is_empty() => {
                if let Ok(progress) = result {
                    finish(&registry, &outbox, progress).await;
                }
            },
            _ = timer.tick() => {}
        }
        let cancelled: Vec<_> = prepared
            .iter()
            .filter(|(_, job)| job.cancel.is_cancelled())
            .map(|(id, _)| *id)
            .collect();
        for id in cancelled {
            drafts.remove(&id);
            if let Some(job) = prepared.remove(&id) {
                disposing.spawn(job.cancelled());
            }
        }
        let mut remaining = VecDeque::new();
        while let Some(job) = queued.pop_front() {
            if job.cancel.is_cancelled() {
                disposing.spawn(job.cancelled());
            } else {
                remaining.push_back(job);
            }
        }
        queued = remaining;
        let updates: Vec<_> = registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter_map(|job| {
                let mut progress = job.status.lock().unwrap_or_else(|e| e.into_inner());
                let requested = !progress.state.is_terminal() && job.cancel.pause_requested();
                let paused = requested && job.cancel.is_paused();
                let changed = (progress.pause_requested, progress.paused) != (requested, paused);
                progress.pause_requested = requested;
                progress.paused = paused;
                if job.cancel.is_cancelled() && progress.state == OperationState::Running {
                    progress.state = OperationState::CancelRequested;
                    crate::operation_throughput::stop(&mut progress.throughput);
                    Some(progress.clone())
                } else if changed {
                    Some(progress.clone())
                } else {
                    None
                }
            })
            .collect();
        for progress in updates {
            publish(&outbox, &progress).await;
        }
        // Failed/panicked preparation and cancelled in-flight previews also retire drafts.
        // Keep this cleanup independent of which terminal path removed the registry entry.
        drafts.retain(|id, _| {
            registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(id)
        });
        conflict_states.retain(|id, _| {
            registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(id)
        });
        rename_states.retain(|id, _| {
            registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(id)
        });
        if running.is_empty()
            && let Some(job) = queued.pop_front()
        {
            let status = job.status.clone();
            let output = outbox.clone();
            let gate = mutation_gate.clone();
            running.spawn(async move {
                if let Err(error) = job
                    .predecessors
                    .wait(&job.cancel, &job.progress().source_directory)
                    .await
                {
                    return (status, Ok(job.failure(error).await), None);
                }
                let acquire = gate.acquire_owned();
                tokio::pin!(acquire);
                let permit = loop {
                    tokio::select! {
                        permit = &mut acquire => break permit.expect("mutation gate stays open"),
                        _ = tokio::time::sleep(Duration::from_millis(25)) => {
                            if job.cancel.is_cancelled() {
                                return (status, Ok(job.cancelled().await), None);
                            }
                        }
                    }
                };
                job.set_state(OperationState::Running);
                publish(&output, &job.progress()).await;
                let result_slot = job.metadata.as_ref().map(|edit| edit.result.clone());
                let mut throughput = crate::operation_throughput::Tracker::default();
                throughput.deletion(&mut status.lock().unwrap_or_else(|e| e.into_inner()));
                let worker_output = output.clone();
                let worker_cancel = job.cancel.clone();
                let pending = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    job.execute(worker_output)
                });
                tokio::pin!(pending);
                let mut timer = tokio::time::interval(crate::operation_throughput::CADENCE);
                timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                let result = loop {
                    tokio::select! {
                        result = &mut pending => {
                            let mut progress = result.unwrap_or_else(|error| worker::panicked(&status, error.to_string()));
                            progress.pause_requested = false;
                            progress.paused = false;
                            throughput.deletion(&mut progress);
                            break Ok(progress);
                        },
                        _ = timer.tick() => {
                            let progress = {
                                let mut progress = status.lock().unwrap_or_else(|e| e.into_inner());
                                let previous = progress.throughput.clone();
                                let requested = worker_cancel.pause_requested();
                                let paused = requested && worker_cancel.is_paused();
                                let changed = (progress.pause_requested, progress.paused) != (requested, paused);
                                progress.pause_requested = requested;
                                progress.paused = paused;
                                throughput.deletion(&mut progress);
                                (changed || !Arc::ptr_eq(&previous, &progress.throughput)).then(|| progress.clone())
                            };
                            if let Some(progress) = progress
                                && output.sender.try_send(Event::Operation(OperationEvent::Progress(Arc::new(progress)))).is_ok()
                            {
                                (output.wakeup)();
                            }
                        },
                    }
                };
                let metadata_result = result_slot
                    .and_then(|slot| slot.lock().unwrap_or_else(|e| e.into_inner()).take());
                (status, result, metadata_result)
            });
        }
        if cancel.is_cancelled()
            && preparing.is_empty()
            && running.is_empty()
            && disposing.is_empty()
            && registry
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty()
        {
            break;
        }
    }
}

type Preparation = (
    Status,
    Result<Result<worker::Job, FsError>, tokio::task::JoinError>,
);

fn spawn_preview(
    tasks: &mut JoinSet<Preparation>,
    mut job: worker::Job,
    revision: u64,
    patch: MetadataPatch,
    budget: Arc<Semaphore>,
    locale: Option<Arc<dyn LocaleService>>,
) {
    job.set_state(OperationState::Preparing);
    tasks.spawn(async move {
        let status = job.status.clone();
        let permit = budget
            .acquire_owned()
            .await
            .expect("operation semaphore stays open");
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if let Some(edit) = &mut job.metadata {
                edit.update(revision, patch, &job.targets, &job.cancel);
                edit.localize(locale.as_deref());
            }
            Ok(job)
        })
        .await;
        (status, result)
    });
}

#[cfg(test)]
#[path = "operation_metadata_tests.rs"]
mod metadata_tests;
#[cfg(test)]
#[path = "operation_tests.rs"]
mod tests;
