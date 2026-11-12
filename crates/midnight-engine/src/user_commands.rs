//! Bounded Custom Commands actor. Preview and execution are distinct backend intents.
use crate::retirement::{RetirementDomain, RetirementLease, WorkerOwned};
use crate::{Outbox, user_command_prepare, user_command_runtime::Runtime};

use midnight_core::*;
use std::{
    collections::HashMap,
    io,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
    task::JoinHandle,
};

pub(crate) const MAX_JOBS: usize = 32;
pub(crate) struct Retirement {
    pub(crate) domain: RetirementDomain,
    pub(crate) service: RetirementLease,
}
struct Services {
    native: Option<Arc<dyn UserCommandService>>,
    files: Option<Arc<dyn TransferService>>,
}
struct PreparedOwner {
    plan: user_command_prepare::Prepared,
    services: Arc<WorkerOwned<Services>>,
    retention: Arc<UserCommandRetention>,
}
type Prepared = Arc<WorkerOwned<PreparedOwner>>;

#[derive(Default)]
struct State {
    highest: u64,
    requests: HashMap<UserCommandRequestId, Cancellation>,
}
struct Work {
    request: UserCommandRequest,
    preview: bool,
    cancel: Cancellation,
    retention: Arc<UserCommandRetention>,
}
struct Lease {
    _job: OwnedSemaphorePermit,
    _memory: OwnedSemaphorePermit,
}
fn jobs() -> Arc<Semaphore> {
    static JOBS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    JOBS.get_or_init(|| Arc::new(Semaphore::new(MAX_JOBS)))
        .clone()
}
fn memory() -> Arc<Semaphore> {
    static MEMORY: OnceLock<Arc<Semaphore>> = OnceLock::new();
    MEMORY
        .get_or_init(|| Arc::new(Semaphore::new(64 * 1024 * 1024)))
        .clone()
}
pub(crate) struct Controller {
    input: mpsc::Sender<WorkerOwned<Work>>,
    retirement: RetirementDomain,
    state: Arc<Mutex<State>>,
    stop: Cancellation,
    available: bool,
    task: Option<JoinHandle<()>>,
}
impl Controller {
    pub(crate) fn new(
        retirement: Retirement,
        native: Option<Arc<dyn UserCommandService>>,
        files: Option<Arc<dyn TransferService>>,
        runtime: Runtime,
        outbox: Outbox,
    ) -> Self {
        let (input, incoming) = mpsc::channel(MAX_JOBS);
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Cancellation::default();
        let available = native.is_some() && files.is_some();
        let services = Arc::new(retirement.service.own(Services { native, files }));
        let task = tokio::spawn(run(
            services,
            runtime,
            outbox,
            incoming,
            state.clone(),
            stop.clone(),
        ));
        Self {
            input,
            retirement: retirement.domain,
            state,
            stop,
            available,
            task: Some(task),
        }
    }
    pub(crate) fn start(&self, request: UserCommandRequest, preview: bool) -> Result<(), FsError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if request.id.0 == 0 || request.id.0 <= state.highest {
            return Err(failure(
                io::ErrorKind::InvalidInput,
                "Command request IDs must increase and never be reused.",
            ));
        }
        state.highest = request.id.0;
        request
            .validate()
            .map_err(|message| failure(io::ErrorKind::InvalidInput, message))?;
        if request.configuration.iter().any(|command| {
            command.id == request.command
                && matches!(command.invocation, UserCommandInvocation::Separator)
        }) {
            return Err(failure(
                io::ErrorKind::InvalidInput,
                "Separators cannot be previewed or executed.",
            ));
        }
        if !self.available {
            return Err(failure(
                io::ErrorKind::Unsupported,
                "Native Custom Commands are unavailable.",
            ));
        }
        if self.stop.is_cancelled() {
            return Err(cancelled());
        }
        let job = jobs().try_acquire_owned().map_err(|_| {
            failure(
                io::ErrorKind::WouldBlock,
                "Custom Command job/result budget is full (32).",
            )
        })?;
        let memory = memory()
            .try_acquire_many_owned(16 * 1024 * 1024)
            .map_err(|_| {
                failure(
                    io::ErrorKind::WouldBlock,
                    "Custom Command target/result budget is full (64 MiB).",
                )
            })?;
        let ownership = self.retirement.try_reserve().map_err(|_| {
            failure(
                io::ErrorKind::WouldBlock,
                "Custom Command cleanup capacity is full.",
            )
        })?;
        let cancel = self.stop.child();
        let id = request.id;
        self.input
            .try_send(ownership.own(Work {
                request,
                preview,
                cancel: cancel.clone(),
                retention: UserCommandRetention::new(Lease {
                    _job: job,
                    _memory: memory,
                }),
            }))
            .map_err(|_| {
                failure(
                    io::ErrorKind::WouldBlock,
                    "Custom Command admission queue is full or closed.",
                )
            })?;
        state.requests.insert(id, cancel);
        Ok(())
    }
    pub(crate) fn cancel(&self, id: UserCommandRequestId) {
        if let Some(cancel) = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .requests
            .get(&id)
        {
            cancel.cancel();
        }
    }
    pub(crate) async fn shutdown(mut self) {
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
    pub(crate) fn stop(&self) {
        self.stop.cancel();
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
struct Context {
    native: Arc<dyn UserCommandService>,
    files: Arc<dyn TransferService>,
    runtime: Runtime,
    outbox: Outbox,
    stop: Cancellation,
    services: Arc<WorkerOwned<Services>>,
}
async fn run(
    services: Arc<WorkerOwned<Services>>,
    runtime: Runtime,
    outbox: Outbox,
    mut input: mpsc::Receiver<WorkerOwned<Work>>,
    state: Arc<Mutex<State>>,
    stop: Cancellation,
) {
    let (Some(native), Some(files)) = (services.native.clone(), services.files.clone()) else {
        return;
    };
    let context = Arc::new(Context {
        native,
        files,
        runtime,
        outbox,
        stop,
        services,
    });
    loop {
        if context.stop.is_cancelled() {
            break;
        }
        match tokio::time::timeout(Duration::from_millis(25), input.recv()).await {
            Ok(Some(work)) => {
                let id = work.request.id;
                execute(work, &context).await;
                state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .requests
                    .remove(&id);
            }
            Ok(None) => break,
            Err(_) => {}
        }
    }
    input.close();
    while input.try_recv().is_ok() {}
    state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .requests
        .clear();
}
async fn execute(work: WorkerOwned<Work>, context: &Arc<Context>) {
    let id = work.request.id;
    let command = work.request.command.clone();
    let cancel = work.cancel.clone();
    let retention = work.retention.clone();
    let preview = work.preview;
    send(context, id, &command, UserCommandUpdate::Queued, None).await;
    send(context, id, &command, UserCommandUpdate::Preparing, None).await;
    let context_owner = context.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        let (payload, ownership) = work.into_parts();
        let work = payload;
        let prepared = if work.preview {
            user_command_prepare::prepare_preview(
                &work.request,
                context_owner.files.as_ref(),
                &work.cancel,
            )
        } else {
            user_command_prepare::prepare(&work.request, context_owner.files.as_ref(), &work.cancel)
        }?;
        for process in prepared.processes.iter() {
            let _permit = midnight_work_budget::shared_io()
                .acquire(midnight_work_budget::IO_SLOTS, || {
                    work.cancel.is_cancelled()
                })
                .ok_or_else(cancelled)?;
            context_owner
                .native
                .validate_user_command(process, &work.cancel)?;
        }
        prepared.revalidate(&work.cancel)?;
        Ok::<_, FsError>(Arc::new(ownership.own(PreparedOwner {
            plan: prepared,
            services: context_owner.services.clone(),
            retention: work.retention.clone(),
        })))
    })
    .await;
    let prepared: Prepared = match prepared {
        Ok(Ok(prepared)) => prepared,
        result => {
            let error = match result {
                Ok(Err(error)) => error,
                _ => failure(io::ErrorKind::Other, "Command preparation worker failed."),
            };
            finish(
                context,
                id,
                &command,
                0,
                0,
                cancel.is_cancelled(),
                Some(error),
            )
            .await;
            return;
        }
    };
    let planned = prepared.plan.processes.len() as u64;
    if preview {
        let result = tokio::task::spawn_blocking(move || {
            let owner = Arc::try_unwrap(prepared).map_err(|_| {
                failure(
                    io::ErrorKind::Other,
                    "Preview ownership changed unexpectedly.",
                )
            })?;
            let (owner, ownership) = owner.into_parts();
            let PreparedOwner {
                plan,
                services,
                retention: held,
            } = owner;
            let preview = plan.into_preview(retention);
            drop(services);
            drop(held);
            drop(ownership);
            preview
        })
        .await;
        match result {
            Ok(Ok(preview)) if !cancel.is_cancelled() => {
                if send(
                    context,
                    id,
                    &command,
                    UserCommandUpdate::Preview(preview),
                    Some(&cancel),
                )
                .await
                {
                    return;
                }
                finish(context, id, &command, planned, 0, true, None).await;
            }
            Ok(Ok(_)) => finish(context, id, &command, planned, 0, true, None).await,
            result => {
                let error = match result {
                    Ok(Err(error)) => error,
                    _ => failure(io::ErrorKind::Other, "Preview cleanup worker failed."),
                };
                finish(
                    context,
                    id,
                    &command,
                    planned,
                    0,
                    cancel.is_cancelled(),
                    Some(error),
                )
                .await;
            }
        }
        return;
    }
    let mut started = 0u64;
    let mut error = None;
    for index in 0..prepared.plan.processes.len() {
        if cancel.is_cancelled() {
            break;
        }
        let reservation = match context.runtime.reserve(&cancel).await {
            Ok(reservation) => reservation,
            Err(cause) => {
                error = Some(cause);
                break;
            }
        };
        let pinned = prepared.clone();
        let context_owner = context.clone();
        let stopping = cancel.clone();
        let launched = tokio::task::spawn_blocking(move || {
            // Both permits and metadata pins survive an async cancellation until native return.
            let _launch = reservation.launch;
            pinned.plan.revalidate_for(index, &stopping)?;
            context_owner
                .native
                .launch_user_command(&pinned.plan.processes[index], &stopping)
                .map(|child| {
                    reservation.ticket.own(
                        child,
                        reservation.process,
                        context_owner.services.clone(),
                    )
                })
        })
        .await;
        match launched {
            Ok(Ok(child)) => {
                let process_id = child.id();
                started += 1;
                let tracking = context.runtime.track(child).await;
                send(
                    context,
                    id,
                    &command,
                    UserCommandUpdate::Started {
                        index: index as u64,
                        process_id,
                    },
                    None,
                )
                .await;
                if let Err(cause) = tracking {
                    error = Some(cause);
                    break;
                }
            }
            Ok(Err(cause)) => {
                error = Some(cause);
                break;
            }
            Err(_) => {
                error = Some(failure(
                    io::ErrorKind::Other,
                    "Native command worker failed.",
                ));
                break;
            }
        }
    }
    if let Ok(prepared) = Arc::try_unwrap(prepared) {
        let _ = prepared.retire().await;
    }
    drop(retention);
    finish(
        context,
        id,
        &command,
        planned,
        started,
        cancel.is_cancelled(),
        error,
    )
    .await;
}
async fn finish(
    context: &Context,
    id: UserCommandRequestId,
    command: &UserCommandId,
    planned: u64,
    started: u64,
    cancelled: bool,
    error: Option<FsError>,
) {
    send(
        context,
        id,
        command,
        UserCommandUpdate::Finished(UserCommandSummary {
            planned,
            started,
            not_started: planned.saturating_sub(started),
            cancelled,
            error,
        }),
        None,
    )
    .await;
}
async fn send(
    context: &Context,
    id: UserCommandRequestId,
    command: &UserCommandId,
    update: UserCommandUpdate,
    cancel: Option<&Cancellation>,
) -> bool {
    let mut pending = Event::UserCommand(Box::new(UserCommandEvent {
        request: id,
        command: command.clone(),
        update,
    }));
    loop {
        if context.stop.is_cancelled() || cancel.is_some_and(Cancellation::is_cancelled) {
            return false;
        }
        match context.outbox.sender.try_send(pending) {
            Ok(()) => {
                (context.outbox.wakeup)();
                return true;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                context.stop.cancel();
                return false;
            }
            Err(mpsc::error::TrySendError::Full(event)) => pending = event,
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
fn cancelled() -> FsError {
    failure(io::ErrorKind::Interrupted, "Custom Command cancelled.")
}
fn failure(kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io(
        "Custom Command",
        Path::new(""),
        io::Error::new(kind, message),
    )
}

#[cfg(test)]
#[path = "user_command_actor_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "user_command_capture_retirement_tests.rs"]
mod capture_retirement_tests;

#[cfg(test)]
#[path = "user_command_pin_tests.rs"]
mod pin_tests;

#[cfg(test)]
impl Retirement {
    fn testing() -> Self {
        let domain = RetirementDomain::try_start(MAX_JOBS + 1, 0).unwrap();
        let service = domain.try_reserve().unwrap();
        Self { domain, service }
    }
}
