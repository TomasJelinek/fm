//! Backend-owned confirmation and FIFO mutation jobs, independent of pane navigation.
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

#[path = "operation_worker.rs"]
mod worker;
type Status = Arc<Mutex<OperationProgress>>;
type Registry = Arc<Mutex<HashMap<OperationId, (Cancellation, Status)>>>;
const MAX_JOBS: usize = 32;

enum Message {
    Prepare(PrepareOperation, Cancellation, Status),
    Confirm(OperationId, Option<String>),
    Wake,
}

pub(crate) struct Controller {
    sender: mpsc::Sender<Message>,
    registry: Registry,
    highest: AtomicU64,
    cancel: Cancellation,
    snapshots: Snapshots,
    outbox: Outbox,
}

impl Controller {
    pub(crate) fn new(
        service: Option<Arc<dyn MutationService>>,
        snapshots: Snapshots,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(64);
        let registry = Registry::default();
        let cancel = Cancellation::default();
        tasks.spawn(run(
            receiver,
            service,
            registry.clone(),
            cancel.clone(),
            outbox.clone(),
        ));
        Self {
            sender,
            registry,
            highest: AtomicU64::new(0),
            cancel,
            snapshots,
            outbox,
        }
    }

    pub(crate) async fn prepare(&self, request: PrepareOperation) {
        // A process-wide high-water mark makes replay idempotent without an unbounded ID set.
        if request.id.0 <= self.highest.fetch_max(request.id.0, Ordering::Relaxed) {
            return;
        }
        let mut progress = worker::initial(&request);
        let valid = self.snapshots.lock().unwrap_or_else(|e| e.into_inner())
            [request.source.pane.index()]
        .visible
        .as_ref()
        .is_some_and(|visible| {
            visible.navigation == request.source.navigation
                && visible.path == request.source.path
                && Arc::ptr_eq(&visible.entries, &request.source.entries)
        });
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
        let cancel = self.cancel.child();
        let status = Arc::new(Mutex::new(progress.clone()));
        self.registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(request.id, (cancel.clone(), status.clone()));
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
        let _ = self.sender.send(Message::Confirm(id, name)).await;
    }
    pub(crate) fn cancel(&self, id: OperationId) {
        if let Some((cancel, _)) = self
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
        {
            cancel.cancel();
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
    registry: Registry,
    cancel: Cancellation,
    outbox: Outbox,
) {
    let budget = Arc::new(Semaphore::new(2));
    let mut preparing = JoinSet::new();
    let mut running = JoinSet::<(Status, Result<OperationProgress, tokio::task::JoinError>)>::new();
    let mut disposing = JoinSet::new();
    let mut prepared = HashMap::<OperationId, worker::Job>::new();
    let mut queued = VecDeque::<worker::Job>::new();
    let mut timer = tokio::time::interval(Duration::from_millis(50));
    let mut accepting = true;
    loop {
        tokio::select! {
            message = receiver.recv(), if accepting => match message {
                Some(Message::Prepare(request, job_cancel, status)) => {
                    let budget = budget.clone();
                    let service = service.clone();
                    preparing.spawn(async move {
                        let original = status.clone();
                        let permit = budget.acquire_owned().await.expect("operation semaphore stays open");
                        let result = tokio::task::spawn_blocking(move || {
                            let _permit = permit;
                            worker::prepare(request, service, job_cancel, status)
                        }).await;
                        (original, result)
                    });
                }
                Some(Message::Confirm(id, name)) => {
                    if let Some(mut job) = prepared.remove(&id) {
                        if let Err(error) = job.confirm(name) {
                            disposing.spawn(job.failure(error));
                        } else {
                            job.set_state(OperationState::Queued);
                            publish(&outbox, &job.progress()).await;
                            queued.push_back(job);
                        }
                    }
                }
                Some(Message::Wake) => {}
                None => { accepting = false; cancel.cancel(); }
            },
            Some(result) = preparing.join_next(), if !preparing.is_empty() => {
                let Ok((status, result)) = result else { continue; };
                match result {
                    Ok(Ok(job)) if !job.cancel.is_cancelled() => {
                        job.set_state(OperationState::AwaitingConfirmation);
                        publish(&outbox, &job.progress()).await;
                        outbox.send(Event::Operation(OperationEvent::Preview(Arc::new(job.preview())))).await;
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
                if let Ok((status, result)) = result {
                    let progress = result.unwrap_or_else(|error| worker::panicked(&status, error.to_string()));
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
            .filter_map(|(cancel, status)| {
                let mut progress = status.lock().unwrap_or_else(|e| e.into_inner());
                if cancel.is_cancelled() && progress.state == OperationState::Running {
                    progress.state = OperationState::CancelRequested;
                    Some(progress.clone())
                } else {
                    None
                }
            })
            .collect();
        for progress in updates {
            publish(&outbox, &progress).await;
        }
        if running.is_empty()
            && let Some(job) = queued.pop_front()
        {
            let status = job.status.clone();
            job.set_state(OperationState::Running);
            publish(&outbox, &job.progress()).await;
            let output = outbox.clone();
            running.spawn(async move {
                let result = tokio::task::spawn_blocking(move || job.execute(output)).await;
                (status, result)
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

#[cfg(test)]
#[path = "operation_tests.rs"]
mod tests;
