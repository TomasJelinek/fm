//! Bounded Custom Commands actor. Preview and execution are distinct backend intents.
use crate::retirement::{RetirementDomain, RetirementLease, WorkerOwned};
use crate::{
    Outbox, user_command_prepare,
    user_command_runtime::{Runtime, TrackedProcess},
};

use midnight_core::*;
use std::{
    collections::HashMap,
    io,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc},
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
const MEMORY_BUDGET: usize = 64 * 1024 * 1024;
pub(crate) struct Controller {
    input: mpsc::Sender<WorkerOwned<Work>>,
    retirement: RetirementDomain,
    state: Arc<Mutex<State>>,
    stop: Cancellation,
    /// Wakes the idle actor for `stop`; it otherwise sleeps until work arrives.
    stopped: Arc<Notify>,
    /// This actor's admitted/retained work leases and target/result budget.
    jobs: Arc<Semaphore>,
    memory: Arc<Semaphore>,
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
        let stopped = Arc::new(Notify::new());
        let available = native.is_some() && files.is_some();
        let services = Arc::new(retirement.service.own(Services { native, files }));
        let task = tokio::spawn(run(
            services,
            runtime,
            outbox,
            incoming,
            state.clone(),
            stop.clone(),
            stopped.clone(),
        ));
        Self {
            input,
            retirement: retirement.domain,
            state,
            stop,
            stopped,
            jobs: Arc::new(Semaphore::new(MAX_JOBS)),
            memory: Arc::new(Semaphore::new(MEMORY_BUDGET)),
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
        let job = self.jobs.clone().try_acquire_owned().map_err(|_| {
            failure(
                io::ErrorKind::WouldBlock,
                "Custom Command job/result budget is full (32).",
            )
        })?;
        let memory = self
            .memory
            .clone()
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
        self.stop();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
    pub(crate) fn stop(&self) {
        self.stop.cancel();
        // A stored permit wakes the actor even if it is not waiting yet.
        self.stopped.notify_one();
    }
    /// A controller whose admitted work stays in `input` for inspection.
    #[cfg(test)]
    fn queue_only(
        input: mpsc::Sender<WorkerOwned<Work>>,
        retirement: RetirementDomain,
        available: bool,
    ) -> Self {
        Self {
            input,
            retirement,
            state: Arc::default(),
            stop: Cancellation::default(),
            stopped: Arc::default(),
            jobs: Arc::new(Semaphore::new(MAX_JOBS)),
            memory: Arc::new(Semaphore::new(MEMORY_BUDGET)),
            available,
            task: None,
        }
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.stop();
    }
}
struct Context {
    native: Arc<dyn UserCommandService>,
    files: Arc<dyn TransferService>,
    runtime: Runtime,
    outbox: Outbox,
    stop: Cancellation,
    services: Arc<WorkerOwned<Services>>,
    state: Arc<Mutex<State>>,
}
async fn run(
    services: Arc<WorkerOwned<Services>>,
    runtime: Runtime,
    outbox: Outbox,
    mut input: mpsc::Receiver<WorkerOwned<Work>>,
    state: Arc<Mutex<State>>,
    stop: Cancellation,
    stopped: Arc<Notify>,
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
        state: state.clone(),
    });
    loop {
        if context.stop.is_cancelled() {
            break;
        }
        // Idle without a timer: work or an explicit stop wakes the actor.
        let work = tokio::select! {
            work = input.recv() => work,
            () = stopped.notified() => continue,
        };
        let Some(work) = work else {
            break;
        };
        let id = work.request.id;
        // An output window's runs continue on their own task, which forgets the
        // request when they end; the actor takes the next request meanwhile.
        if !execute(work, &context).await {
            forget(&context, id);
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
fn forget(context: &Context, id: UserCommandRequestId) {
    context
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .requests
        .remove(&id);
}
/// True when the runs continue on their own task (an output window's), which forgets
/// the request itself.
async fn execute(work: WorkerOwned<Work>, context: &Arc<Context>) -> bool {
    let id = work.request.id;
    let command = work.request.command.clone();
    let cancel = work.cancel.clone();
    let retention = work.retention.clone();
    let preview = work.preview;
    let (output, name) = work
        .request
        .configuration
        .iter()
        .find(|configured| configured.id == command)
        .map(|configured| (configured.output, configured.name.clone()))
        .unwrap_or_default();
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
                .acquire(midnight_work_budget::Admission::Exclusive, || {
                    work.cancel.is_cancelled()
                })
                .ok_or_else(cancelled)?;
            context_owner
                .native
                .validate_user_command(process, &work.cancel)?;
        }
        prepared.revalidate(context_owner.files.as_ref(), &work.cancel)?;
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
            return false;
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
                    return false;
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
        return false;
    }
    if output.mode == UserCommandOutputMode::Window {
        let session = UserCommandOutputSession::new(id, command.clone(), name, planned);
        if !send_output(context, &session, true).await {
            finish(context, id, &command, planned, 0, true, None).await;
            return false;
        }
        tokio::spawn(run_with_output(
            context.clone(),
            prepared,
            session,
            command,
            cancel,
            retention,
        ));
        return true;
    }
    let mut started = 0u64;
    let mut error = None;
    for index in 0..prepared.plan.processes.len() {
        if cancel.is_cancelled() {
            break;
        }
        let launched = match launch(context, &prepared, index, output.mode, &cancel).await {
            Ok(Launched::Tracked(child)) => Ok(child),
            // Only an output window captures; anything else is a contract breach.
            Ok(Launched::Captured(mut program, _)) => {
                let _ = program.terminate_tree(true);
                Err(failure(
                    io::ErrorKind::Other,
                    "The program started with captured output without an output window.",
                ))
            }
            Err(cause) => Err(cause),
        };
        match launched {
            Ok(child) => {
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
            Err(cause) => {
                error = Some(cause);
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
    false
}
enum Launched {
    Tracked(TrackedProcess),
    /// With the process permit it holds until it exits.
    Captured(Box<dyn CapturedUserProgram>, Arc<OwnedSemaphorePermit>),
}
/// Admit and start planned process `index` on a backend worker.
async fn launch(
    context: &Arc<Context>,
    prepared: &Prepared,
    index: usize,
    output: UserCommandOutputMode,
    cancel: &Cancellation,
) -> Result<Launched, FsError> {
    let reservation = context.runtime.reserve(cancel).await?;
    let pinned = prepared.clone();
    let context_owner = context.clone();
    let stopping = cancel.clone();
    tokio::task::spawn_blocking(move || {
        // Both permits and metadata pins survive an async cancellation until native return.
        let _launch = reservation.launch;
        pinned
            .plan
            .revalidate_for(index, context_owner.files.as_ref(), &stopping)?;
        match context_owner.native.launch_user_command_with_output(
            &pinned.plan.processes[index],
            output,
            &stopping,
        )? {
            LaunchedUserCommand::Tracked(child) => Ok(Launched::Tracked(reservation.ticket.own(
                child,
                reservation.process,
                context_owner.services.clone(),
            ))),
            // Supervised until its exit by the output window's task, not the tracker.
            LaunchedUserCommand::Captured(program) => {
                drop(reservation.ticket);
                Ok(Launched::Captured(program, reservation.process))
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        Err(failure(
            io::ErrorKind::Other,
            "Native command worker failed.",
        ))
    })
}
/// An output window's runs, one after another (a per-item command shows each item's run
/// in turn), each supervised to its exit; Stop or Cancel ends the running one and starts
/// no further one.
async fn run_with_output(
    context: Arc<Context>,
    prepared: Prepared,
    session: Arc<UserCommandOutputSession>,
    command: UserCommandId,
    cancel: Cancellation,
    retention: Arc<UserCommandRetention>,
) {
    let id = session.request;
    let planned = prepared.plan.processes.len() as u64;
    let publish = publisher(&context);
    let mut started = 0u64;
    let mut error = None;
    for index in 0..prepared.plan.processes.len() {
        if cancel.is_cancelled() || session.stop_requested() {
            break;
        }
        let process = &prepared.plan.processes[index];
        let (program, permit) = match launch(
            &context,
            &prepared,
            index,
            UserCommandOutputMode::Window,
            &cancel,
        )
        .await
        {
            Ok(Launched::Captured(program, permit)) => (program, permit),
            Ok(Launched::Tracked(child)) => {
                // Never for an output window: keep the tracker's ownership.
                let _ = context.runtime.track(child).await;
                error = Some(failure(
                    io::ErrorKind::Other,
                    "The program started without captured output.",
                ));
                break;
            }
            Err(cause) => {
                error = Some(cause);
                break;
            }
        };
        started += 1;
        let process_id = program.id();
        let command_line = match &process.invocation {
            UserCommandProcessInvocation::Program {
                executable,
                arguments,
            } => display_command_line(executable, arguments),
            UserCommandProcessInvocation::Terminal { script, .. } => script.clone(),
        };
        session.begin_run(OutputRun {
            index: index as u64,
            command_line,
            directory: process.working_directory.to_string_lossy().into_owned(),
            started: std::time::SystemTime::now(),
            process_id,
            exit: None,
            duration: None,
            stopped: false,
        });
        send(
            &context,
            id,
            &command,
            UserCommandUpdate::Started {
                index: index as u64,
                process_id,
            },
            None,
        )
        .await;
        let supervised = crate::user_command_output::supervise_on_thread(
            program,
            crate::user_command_output::Supervision {
                session: session.clone(),
                index: index as u64,
                cancel: cancel.clone(),
                publish: publish.clone(),
                // The process permit and the services stay held until the program exited.
                _pins: Box::new((permit, context.services.clone())),
            },
        )
        .await;
        if supervised.is_err() {
            error = Some(failure(
                io::ErrorKind::Other,
                "The output window's worker failed.",
            ));
            break;
        }
    }
    let cancelled = cancel.is_cancelled() || session.stop_requested();
    session.finish(
        cancelled,
        error
            .as_ref()
            .map(|error| format!("Not started: {}", error.message)),
    );
    crate::user_command_output::notify(&session, &publish);
    if let Ok(prepared) = Arc::try_unwrap(prepared) {
        let _ = prepared.retire().await;
    }
    drop(retention);
    finish(&context, id, &command, planned, started, cancelled, error).await;
    forget(&context, id);
}
/// Queue `UserCommandOutput` events without waiting: a full queue retries later.
fn publisher(context: &Arc<Context>) -> crate::user_command_output::Publish {
    let outbox = context.outbox.clone();
    Arc::new(move |session| {
        let event = Event::UserCommandOutput(Box::new(UserCommandOutputEvent {
            session: session.clone(),
            opened: false,
        }));
        if outbox.sender.try_send(event).is_ok() {
            (outbox.wakeup)();
            true
        } else {
            false
        }
    })
}
/// Ask the UI for the output window, waiting while its queue is full.
async fn send_output(
    context: &Context,
    session: &Arc<UserCommandOutputSession>,
    opened: bool,
) -> bool {
    let mut pending = Event::UserCommandOutput(Box::new(UserCommandOutputEvent {
        session: session.clone(),
        opened,
    }));
    loop {
        if context.stop.is_cancelled() {
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
