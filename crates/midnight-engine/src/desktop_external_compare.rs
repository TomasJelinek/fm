//! External comparison capture and transport. No decoder or content-read path.
use crate::{
    copy_nested_retirement,
    retirement::{
        CheckedFinish, CheckedPayload, CheckedRelease, CheckedWorkerOwned, RetirementDomain,
    },
    task_lifetime::TaskLifetime,
    user_command_runtime::{ProcessTicket, ProcessTracker},
};
use midnight_core::{
    Cancellation, ExternalCompareOutcome, ExternalCompareOwnedPayload, ExternalCompareProcess,
    ExternalCompareRequest, ExternalCompareSessionCredit, FsError, TerminalProcess,
    UserCommandProcessInvocation, UserCommandService,
};
use midnight_plugin_host::CapturedFile;
use std::{
    any::{Any, TypeId},
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{Arc, mpsc::SyncSender},
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
    task::JoinSet,
};

/// Failure metadata from the installed capture authority. A native tail retained
/// by that authority must also retain this request's open/session/cleanup credits.
#[derive(Debug)]
pub struct ExternalCompareCaptureError {
    pub error: FsError,
    pub cleanup_unconfirmed: bool,
}

/// Trusted host composition only, called synchronously on the comparator worker.
/// The adapter must preserve typed cleanup status and use the installed authority.
/// It owns no already-produced captures; arbitrary plugin/UI callbacks are not a
/// supported construction path. No callback is stored in a command or tracker.
pub type ExternalCompareNativeCapture = Arc<
    dyn Fn(&Path, &Cancellation) -> Result<CapturedFile, ExternalCompareCaptureError> + Send + Sync,
>;

#[derive(Clone)]
pub struct ExternalComparisonFrontDoor {
    admission: Arc<Admission>,
}
struct Admission {
    open: Arc<Semaphore>,
    retirement: RetirementDomain,
}

struct Pair {
    process: ExternalCompareProcess,
    paths: [PathBuf; 2],
    captures: [Option<CapturedFile>; 2],
    cancel: Cancellation,
    capture_cleanup_unconfirmed: bool,
}
struct Accounting {
    _open: OwnedSemaphorePermit,
    _session: Arc<dyn ExternalCompareSessionCredit>,
}
pub(crate) struct ArmedExternalComparison {
    origin: Arc<Admission>,
    work: CheckedWorkerOwned<Pair, Accounting>,
    // This sender never enters quarantine. A dropped request disconnects the
    // observer even when native cleanup keeps the original frame indefinitely.
    reply: SyncSender<ExternalCompareOutcome>,
}
impl ExternalCompareOwnedPayload for ArmedExternalComparison {
    fn as_any(&self) -> &(dyn Any + Send) {
        self
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}

impl ExternalComparisonFrontDoor {
    pub(crate) fn new(retirement: RetirementDomain) -> Self {
        Self {
            admission: Arc::new(Admission {
                open: Arc::new(Semaphore::new(crate::desktop::MAX_OPEN_LAUNCHES)),
                retirement,
            }),
        }
    }
    pub(crate) fn open_budget(&self) -> Arc<Semaphore> {
        self.admission.open.clone()
    }

    /// Reserve shared open capacity and the actual cleanup slot before capturing.
    /// The returned request, and every future/error subsequently containing it,
    /// already owns worker-side checked Discard. Call only on a backend worker.
    pub fn prepare(
        &self,
        process: ExternalCompareProcess,
        original_paths: [PathBuf; 2],
        capture: &ExternalCompareNativeCapture,
        session: Arc<dyn ExternalCompareSessionCredit>,
        cancel: Cancellation,
        reply: SyncSender<ExternalCompareOutcome>,
    ) -> Result<ExternalCompareRequest, FsError> {
        process.as_process().validate().map_err(failure)?;
        if original_paths
            .iter()
            .any(|path| !path.is_absolute() || path.as_os_str().as_encoded_bytes().contains(&0))
        {
            return Err(failure(
                "External comparison requires absolute native paths.",
            ));
        }
        check_cancel(&cancel)?;
        let open = self
            .admission
            .open
            .clone()
            .try_acquire_owned()
            .map_err(|_| failure("File launch capacity is occupied."))?;
        let lease = self
            .admission
            .retirement
            .try_reserve_nowait()
            .map_err(|_| failure("File cleanup capacity is occupied."))?;
        // No captures exist before this allocation/arming step. The callback's
        // successful return is assigned before any cancellation or second capture.
        let mut work = lease.own_checked(
            Pair {
                process,
                paths: original_paths,
                captures: [None, None],
                cancel,
                capture_cleanup_unconfirmed: false,
            },
            Accounting {
                _open: open,
                _session: session,
            },
        );
        let captured = work
            .with_worker(|pair| {
                for side in 0..2 {
                    check_cancel(&pair.cancel)?;
                    match capture(&pair.paths[side], &pair.cancel) {
                        Ok(captured) => pair.captures[side] = Some(captured),
                        Err(error) => {
                            pair.capture_cleanup_unconfirmed |= error.cleanup_unconfirmed;
                            return Err(error.error);
                        }
                    }
                    check_cancel(&pair.cancel)?;
                }
                Ok(())
            })
            .unwrap_or_else(|_| Err(failure("Native capture panicked; cleanup is unconfirmed.")));
        if let Err(error) = captured {
            return Err(
                match work.finish_on_worker(|pair| finish_pair(pair, false)) {
                    CheckedFinish::Confirmed { .. } => error,
                    CheckedFinish::Unconfirmed(cleanup) => cleanup,
                },
            );
        }
        // Readiness is immutable and certified by this constructor, never queried
        // by running a native callback at sender admission.
        Ok(ExternalCompareRequest::from_backend_owner(Box::new(
            ArmedExternalComparison {
                origin: self.admission.clone(),
                work,
                reply,
            },
        )))
    }

    pub(crate) fn classify(
        &self,
        request: ExternalCompareRequest,
    ) -> Result<ArmedExternalComparison, ExternalCompareRequest> {
        let owner = request.into_backend_owner();
        // Any's actual dynamic identity is used, not a potentially unrelated
        // object returned by a foreign as_any implementation.
        if owner.as_ref().type_id() != TypeId::of::<ArmedExternalComparison>() {
            return Err(ExternalCompareRequest::from_backend_owner(owner));
        }
        let owner = owner
            .into_any()
            .downcast::<ArmedExternalComparison>()
            .unwrap_or_else(|_| unreachable!("private Engine bridge preserves its concrete type"));
        if !Arc::ptr_eq(&owner.origin, &self.admission) {
            return Err(ExternalCompareRequest::from_backend_owner(owner));
        }
        Ok(*owner)
    }
}

impl CheckedPayload for Pair {
    fn retirement_is_checked(&self) -> bool {
        true
    }
    fn discard_on_worker(&mut self) -> CheckedRelease<()> {
        match finish_pair(self, false) {
            CheckedRelease::Confirmed(_) => CheckedRelease::Confirmed(()),
            CheckedRelease::Unconfirmed(error) => CheckedRelease::Unconfirmed(error),
        }
    }
}

fn finish_pair(pair: &mut Pair, before_launch: bool) -> CheckedRelease<Result<(), FsError>> {
    if pair.capture_cleanup_unconfirmed {
        return CheckedRelease::Unconfirmed(failure("Capture cleanup is unconfirmed."));
    }
    let mut validation = Ok(());
    if before_launch {
        validation = check_cancel(&pair.cancel);
        for capture in &pair.captures {
            if let Err(error) = check_cancel(&pair.cancel) {
                validation = Err(error);
                break;
            }
            let result = capture
                .as_ref()
                .ok_or_else(|| failure("Comparison capture is missing."))
                .and_then(|file| {
                    file.revalidate()
                        .map_err(|_| failure("Comparison input changed."))
                });
            if validation.is_ok() {
                validation = result;
            }
            if let Err(error) = check_cancel(&pair.cancel) {
                validation = Err(error);
                break;
            }
        }
    }
    for capture in &mut pair.captures {
        if let Some(file) = capture.take() {
            // PendingContent retains its own remainder; every not-yet-consumed
            // side remains in this outer armed frame on error or panic.
            if let Err(error) = copy_nested_retirement::content_step(|| file.release_on_worker()) {
                return CheckedRelease::Unconfirmed(failure(&error.to_string()));
            }
        }
    }
    CheckedRelease::Confirmed(validation)
}
fn check_cancel(cancel: &Cancellation) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(failure("External comparison cancelled."))
    } else {
        Ok(())
    }
}
fn failure(message: &str) -> FsError {
    FsError::from_io(
        "external comparison",
        Path::new(""),
        io::Error::other(message),
    )
}

pub(crate) struct LaunchContext {
    pub(crate) scope: TaskLifetime,
    pub(crate) processes: Arc<Semaphore>,
    pub(crate) tracker: ProcessTracker,
    pub(crate) children: mpsc::Sender<crate::desktop::TrackedChild>,
}
struct NativeLaunch {
    service: Arc<dyn UserCommandService>,
    ticket: ProcessTicket,
    quota: Arc<OwnedSemaphorePermit>,
    scope: TaskLifetime,
    children: mpsc::Sender<crate::desktop::TrackedChild>,
}

/// Engine-loop admission is synchronous and bounded. Native work receives an
/// already-armed pair, the existing process quota and its pre-reserved ticket.
pub(crate) fn launch(
    context: LaunchContext,
    service: Option<Arc<dyn UserCommandService>>,
    request: ArmedExternalComparison,
    tasks: &mut JoinSet<()>,
) {
    let Some(service) = service else {
        reject(
            request,
            failure("External program integration is unavailable."),
            tasks,
        );
        return;
    };
    let Ok(quota) = context.processes.clone().try_acquire_owned() else {
        reject(
            request,
            failure("Eight terminal, editor or Custom Command processes are active."),
            tasks,
        );
        return;
    };
    let Ok(ticket) = context.tracker.try_reserve() else {
        reject(
            request,
            failure("Program cleanup capacity is occupied."),
            tasks,
        );
        return;
    };
    let scope = context.scope.clone();
    let native = NativeLaunch {
        service,
        ticket,
        quota: Arc::new(quota),
        scope: context.scope,
        children: context.children,
    };
    // The closure itself owns checked Discard before spawn_blocking can accept
    // it; executor cancellation before invocation cannot raw-drop captures.
    let worker =
        tokio::task::spawn_blocking(scope.blocking(move || launch_on_worker(request, native)));
    tasks.spawn(async move {
        let _ = worker.await;
    });
}

fn reject(request: ArmedExternalComparison, reason: FsError, tasks: &mut JoinSet<()>) {
    let ArmedExternalComparison { work, reply, .. } = request;
    // Enqueue now, before constructing a possibly unpolled result future.
    let retirement = work.retire();
    tasks.spawn(async move {
        let error = retirement.await.err().unwrap_or(reason);
        deliver(&reply, ExternalCompareOutcome::NotStarted(error));
    });
}

fn launch_on_worker(request: ArmedExternalComparison, native: NativeLaunch) {
    let ArmedExternalComparison { work, reply, .. } = request;
    let (pair, _accounting) = match work.finish_on_worker(|pair| finish_pair(pair, true)) {
        CheckedFinish::Confirmed {
            payload,
            accounting,
            result: Ok(()),
        } => (payload, accounting),
        CheckedFinish::Confirmed {
            result: Err(error), ..
        }
        | CheckedFinish::Unconfirmed(error) => {
            deliver(&reply, ExternalCompareOutcome::NotStarted(error));
            return;
        }
    };
    if let Err(error) = check_cancel(&pair.cancel) {
        deliver(&reply, ExternalCompareOutcome::NotStarted(error));
        return;
    }
    let process = pair.process.as_process();
    let UserCommandProcessInvocation::Program {
        executable,
        arguments,
    } = &process.invocation
    else {
        deliver(
            &reply,
            ExternalCompareOutcome::NotStarted(failure("Comparator requires a program.")),
        );
        return;
    };
    let process_scope: Arc<dyn Send + Sync> = Arc::new(native.scope);
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        match native.service.launch_user_program(
            executable,
            arguments,
            &process.working_directory,
            &pair.cancel,
        ) {
            Ok(child) => {
                // FIRST operation after native return: id() itself can panic,
                // so ProcessTicket must own the actual child before observing it.
                let child = native.ticket.own(child, native.quota, process_scope);
                let pid = child.id();
                let tracking_warning = native
                    .children
                    .try_send(crate::desktop::TrackedChild {
                        _slot: child.quota(),
                        child: Box::new(child),
                        completion: None,
                    })
                    .err()
                    .map(|undelivered| {
                        // TrackedProcess Drop transfers the real child/quota to the
                        // existing independent tracker on both Full and Closed.
                        drop(undelivered);
                        "Program started; independent process cleanup remains active.".into()
                    });
                ExternalCompareOutcome::Started {
                    pid,
                    tracking_warning,
                }
            }
            Err(error) => ExternalCompareOutcome::NotStarted(error),
        }
    }))
    .unwrap_or_else(|panic| {
        std::mem::forget(panic);
        ExternalCompareOutcome::Unknown {
            message: "Program launch outcome is uncertain; do not retry automatically.".into(),
        }
    });
    deliver(&reply, outcome);
}

fn deliver(reply: &SyncSender<ExternalCompareOutcome>, outcome: ExternalCompareOutcome) {
    if let Err(panic) = catch_unwind(AssertUnwindSafe(|| {
        let _ = reply.try_send(outcome);
    })) {
        std::mem::forget(panic);
    }
}

#[cfg(test)]
#[path = "desktop_external_compare_tests.rs"]
mod tests;
