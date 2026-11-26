use super::*;
use std::{
    panic::panic_any,
    rc::Rc,
    sync::mpsc::{self, Receiver, Sender},
    task::{Wake, Waker},
    thread::{self, ThreadId},
    time::Duration,
};
use tokio::runtime::{Id, Runtime};

const LIMIT: Duration = Duration::from_secs(5);

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_time()
        .build()
        .unwrap()
}

fn wait<T>(runtime: &Runtime, future: impl Future<Output = T>) -> T {
    runtime.block_on(async { tokio::time::timeout(LIMIT, future).await.unwrap() })
}

fn start_blocked(spawner: &Spawner, runtime: &Runtime) -> (Ticket, ThreadId, Sender<()>) {
    let (started, observed) = mpsc::channel();
    let (release, hold) = mpsc::channel();
    let ticket = spawner
        .spawn(
            runtime.handle(),
            Box::new(move || {
                started.send(thread::current().id()).unwrap();
                let _ = hold.recv();
            }),
        )
        .unwrap();
    let worker = observed.recv_timeout(LIMIT).unwrap();
    (ticket, worker, release)
}

fn start_with_blocked_epilogue() -> (Owner, Receiver<()>, Sender<()>) {
    let (epilogue_started, observed_epilogue) = mpsc::channel();
    let (release, hold) = mpsc::channel();
    let mut hold = Some(hold);
    let (owner, _) = Owner::start_with("exit-epilogue", 1, None, |builder, run| {
        let epilogue_started = epilogue_started.clone();
        let hold = hold.take().unwrap();
        builder.spawn(move || {
            run();
            epilogue_started.send(()).unwrap();
            let _ = hold.recv();
        })
    })
    .unwrap();
    (owner, observed_epilogue, release)
}

struct ObserveDrop(Sender<(ThreadId, Option<Id>)>);

struct FailingPayloadDrop;

impl Drop for FailingPayloadDrop {
    fn drop(&mut self) {
        panic!("panic payload destructor failure");
    }
}

impl Drop for ObserveDrop {
    fn drop(&mut self) {
        let runtime = Handle::try_current().ok().map(|handle| handle.id());
        let _ = self.0.send((thread::current().id(), runtime));
    }
}

fn drop_probe() -> (ObserveDrop, Receiver<(ThreadId, Option<Id>)>) {
    let (drop_sender, drop_receiver) = mpsc::channel();
    (ObserveDrop(drop_sender), drop_receiver)
}

#[test]
fn first_and_partial_startup_failure_leave_no_initialization_worker() {
    assert!(Handle::try_current().is_err());

    for fail_at in [0, 2] {
        let mut attempt = 0;
        let (retired, observed) = mpsc::channel();
        let result = Owner::start_with("startup", 3, None, |builder, run| {
            let index = attempt;
            attempt += 1;
            if index == fail_at {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "injected"));
            }
            let retired = retired.clone();
            builder.spawn(move || {
                run();
                retired.send(index).unwrap();
            })
        });

        assert_eq!(
            result.err().unwrap().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(attempt, fail_at + 1);
        let mut retired = observed.try_iter().collect::<Vec<_>>();
        retired.sort_unstable();
        assert_eq!(retired, (0..fail_at).collect::<Vec<_>>());
    }
}

#[test]
fn rejects_zero_threads_without_starting_a_worker() {
    let result = Owner::start_with("zero", 0, None, |_, _| {
        panic!("must not try to start a worker")
    });
    assert_eq!(result.err().unwrap().kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn a_launched_thread_that_exits_before_readiness_fails_startup_and_retires_peers() {
    let mut attempt = 0;
    let (retired, observed) = mpsc::channel();
    let startup = Owner::start_with("missing-readiness", 3, None, |builder, run| {
        let index = attempt;
        attempt += 1;
        let retired = retired.clone();
        builder.spawn(move || {
            if index == 1 {
                drop(run);
            } else {
                run();
            }
            retired.send(index).unwrap();
        })
    });

    assert!(startup.is_err());
    let mut retired = observed.try_iter().collect::<Vec<_>>();
    retired.sort_unstable();
    assert_eq!(retired, [0, 1, 2]);
}

#[test]
fn full_queue_returns_intact_work_and_capacity_remains_usable() {
    let (owner, spawner) = Owner::start("full", 1).unwrap();
    let runtime = runtime();
    let (active, worker, release) = start_blocked(&spawner, &runtime);
    let queued = spawner.spawn(runtime.handle(), Box::new(|| {})).unwrap();
    let (capture, observed_drop) = drop_probe();
    let (ran, observed_run) = mpsc::channel();
    let refused = spawner
        .spawn(
            runtime.handle(),
            Box::new(move || {
                ran.send(()).unwrap();
                drop(capture);
            }),
        )
        .err()
        .unwrap();

    assert_eq!(refused.reason(), Rejection::Full);
    assert!(matches!(
        observed_drop.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    assert!(matches!(
        observed_run.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    assert!(!active.is_finished());
    assert!(!queued.is_finished());
    drop(release);
    assert_eq!(wait(&runtime, active), Ok(()));
    assert_eq!(wait(&runtime, queued), Ok(()));

    let retried = spawner
        .spawn(runtime.handle(), refused.into_work())
        .unwrap();
    assert_eq!(wait(&runtime, retried), Ok(()));
    assert_eq!(observed_run.recv_timeout(LIMIT), Ok(()));
    assert_eq!(
        observed_drop.recv_timeout(LIMIT),
        Ok((worker, Some(runtime.handle().id())))
    );
    assert!(wait(&runtime, owner.shutdown(runtime.handle())).is_ok());
}

#[test]
fn panic_payload_is_destroyed_in_context_and_worker_accepts_more_work() {
    let (owner, spawner) = Owner::start("panic", 1).unwrap();
    let runtime = runtime();
    let (payload, payload_drop) = drop_probe();
    let (started, observed_start) = mpsc::channel();
    let mut failed = spawner
        .spawn(
            runtime.handle(),
            Box::new(move || {
                started.send(thread::current().id()).unwrap();
                panic_any(payload);
            }),
        )
        .unwrap();
    let worker = observed_start.recv_timeout(LIMIT).unwrap();
    assert_eq!(wait(&runtime, &mut failed), Err(TaskError::Panicked));
    assert!(failed.is_finished());
    assert_eq!(
        payload_drop.recv_timeout(LIMIT),
        Ok((worker, Some(runtime.handle().id())))
    );

    let (next, observed_next) = mpsc::channel();
    let next_ticket = spawner
        .spawn(
            runtime.handle(),
            Box::new(move || next.send(thread::current().id()).unwrap()),
        )
        .unwrap();
    assert_eq!(wait(&runtime, next_ticket), Ok(()));
    assert_eq!(observed_next.recv_timeout(LIMIT), Ok(worker));
    assert!(wait(&runtime, owner.shutdown(runtime.handle())).is_ok());
}

#[test]
fn known_worker_failure_rejects_reentrant_admission_from_its_completion_waker() {
    struct SubmitOnWake {
        spawner: Spawner,
        transient: Handle,
        submitted: Sender<Result<Ticket, Rejected>>,
    }

    impl Wake for SubmitOnWake {
        fn wake(self: Arc<Self>) {
            let submitted = self.spawner.spawn(&self.transient, Box::new(|| {}));
            let _ = self.submitted.send(submitted);
        }
    }

    let (owner, spawner) = Owner::start("failure-reentry", 1).unwrap();
    let runtime = runtime();
    let (release, hold) = mpsc::channel::<()>();
    let mut failing = spawner
        .spawn(
            runtime.handle(),
            Box::new(move || {
                let _ = hold.recv();
                panic_any(FailingPayloadDrop);
            }),
        )
        .unwrap();
    let (submitted, observed) = mpsc::channel();
    let waker = Waker::from(Arc::new(SubmitOnWake {
        spawner,
        transient: runtime.handle().clone(),
        submitted,
    }));
    assert!(
        Pin::new(&mut failing)
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    drop(release);

    let refused = observed.recv_timeout(LIMIT).unwrap().err().unwrap();
    assert_eq!(refused.reason(), Rejection::WorkerFailed);
    assert_eq!(wait(&runtime, failing), Err(TaskError::Panicked));
    assert_eq!(
        wait(&runtime, owner.shutdown(runtime.handle())),
        Err(ShutdownError(ShutdownFailure::WorkerFailed))
    );
}

#[test]
fn ticket_waker_panic_payload_is_destroyed_in_the_work_runtime_on_its_worker() {
    struct PanicOnWake(Mutex<Option<ObserveDrop>>);

    impl Wake for PanicOnWake {
        fn wake(self: Arc<Self>) {
            let payload = self.0.lock().unwrap().take().unwrap();
            panic_any(payload);
        }
    }

    let (owner, spawner) = Owner::start("ticket-waker-panic", 1).unwrap();
    let runtime = runtime();
    let (mut ticket, worker, release) = start_blocked(&spawner, &runtime);
    let (payload, disposed) = drop_probe();
    let waker = Waker::from(Arc::new(PanicOnWake(Mutex::new(Some(payload)))));
    assert!(
        Pin::new(&mut ticket)
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    drop(release);

    assert_eq!(
        disposed.recv_timeout(LIMIT),
        Ok((worker, Some(runtime.handle().id())))
    );
    assert_eq!(wait(&runtime, ticket), Ok(()));
    assert_eq!(
        wait(&runtime, owner.shutdown(runtime.handle())),
        Err(ShutdownError(ShutdownFailure::WorkerFailed))
    );
}

#[test]
fn abort_before_claim_skips_the_body_and_drops_capture_in_the_original_context() {
    let (owner, spawner) = Owner::start("abort-queued", 1).unwrap();
    let original = runtime();
    let submitter = runtime();
    let (active, worker, release) = start_blocked(&spawner, &original);
    let (capture, observed_drop) = drop_probe();
    let ticket = {
        let _current = submitter.enter();
        spawner
            .spawn(
                original.handle(),
                Box::new(move || {
                    drop(capture);
                    panic!("abort won claim");
                }),
            )
            .unwrap()
    };
    ticket.abort();
    drop(release);

    assert_eq!(wait(&submitter, ticket), Err(TaskError::Cancelled));
    assert_eq!(wait(&original, active), Ok(()));
    assert_eq!(
        observed_drop.recv_timeout(LIMIT),
        Ok((worker, Some(original.handle().id())))
    );
    assert!(wait(&original, owner.shutdown(original.handle())).is_ok());
}

#[test]
fn claim_before_abort_allows_the_body_to_complete() {
    let (owner, spawner) = Owner::start("abort-running", 1).unwrap();
    let runtime = runtime();
    let (active, _, release) = start_blocked(&spawner, &runtime);
    active.abort();
    assert!(!active.is_finished());
    drop(release);
    assert_eq!(wait(&runtime, active), Ok(()));
    assert!(wait(&runtime, owner.shutdown(runtime.handle())).is_ok());
}

#[test]
fn dropping_plain_ticket_detaches_queued_work() {
    let (owner, spawner) = Owner::start("detach", 1).unwrap();
    let runtime = runtime();
    let (active, worker, release) = start_blocked(&spawner, &runtime);
    let (ran, observed_run) = mpsc::channel();
    let ticket = spawner
        .spawn(
            runtime.handle(),
            Box::new(move || ran.send(thread::current().id()).unwrap()),
        )
        .unwrap();
    drop(ticket);
    drop(release);
    assert_eq!(wait(&runtime, active), Ok(()));
    assert_eq!(observed_run.recv_timeout(LIMIT), Ok(worker));
    assert!(wait(&runtime, owner.shutdown(runtime.handle())).is_ok());
}

#[test]
fn invocation_and_nested_async_blocking_and_timer_work_use_the_supplied_runtime() {
    let (owner, spawner) = Owner::start("nested", 1).unwrap();
    let original = runtime();
    let submitter = runtime();
    let expected = original.handle().id();
    let ticket = {
        let _current = submitter.enter();
        spawner
            .spawn(
                original.handle(),
                Box::new(move || {
                    let resident = thread::current().id();
                    assert_eq!(Handle::current().id(), expected);
                    let nested_async = tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                        (Handle::current().id(), thread::current().id())
                    });
                    let nested_blocking = tokio::task::spawn_blocking(move || {
                        (Handle::current().id(), thread::current().id())
                    });
                    Handle::current().block_on(async {
                        for nested in [nested_async, nested_blocking] {
                            let (runtime, worker) = nested.await.unwrap();
                            assert_eq!(runtime, expected);
                            assert_ne!(worker, resident);
                        }
                    });
                }),
            )
            .unwrap()
    };
    assert_eq!(wait(&submitter, ticket), Ok(()));
    assert!(wait(&original, owner.shutdown(original.handle())).is_ok());
}

#[test]
fn affine_resource_is_created_and_destroyed_by_the_same_worker() {
    struct Affine {
        _non_send: Rc<()>,
        created: ThreadId,
        retired: Sender<(ThreadId, ThreadId)>,
    }

    impl Drop for Affine {
        fn drop(&mut self) {
            self.retired
                .send((self.created, thread::current().id()))
                .unwrap();
        }
    }

    let (owner, spawner) = Owner::start("affine", 1).unwrap();
    let runtime = runtime();
    let caller = thread::current().id();
    let (retired, observed) = mpsc::channel();
    let ticket = spawner
        .spawn(
            runtime.handle(),
            Box::new(move || {
                let _resource = Affine {
                    _non_send: Rc::new(()),
                    created: thread::current().id(),
                    retired,
                };
            }),
        )
        .unwrap();
    assert_eq!(wait(&runtime, ticket), Ok(()));
    let (created, destroyed) = observed.recv_timeout(LIMIT).unwrap();
    assert_eq!(created, destroyed);
    assert_ne!(destroyed, caller);
    assert!(wait(&runtime, owner.shutdown(runtime.handle())).is_ok());
}

#[test]
fn final_owner_drop_returns_while_blocked_and_neutral_spawners_preserve_no_stop_authority() {
    let (exited, exit_observed) = mpsc::channel();
    let (owner, spawner) = Owner::start_with("owner-drop", 1, None, |builder, run| {
        let exited = exited.clone();
        builder.spawn(move || {
            run();
            exited.send(()).unwrap();
        })
    })
    .unwrap();
    let runtime = runtime();
    let kept = spawner.clone();
    let (active, worker, release) = start_blocked(&spawner, &runtime);
    let (capture, observed_drop) = drop_probe();
    let in_work = kept.clone();
    let queued = spawner
        .spawn(
            runtime.handle(),
            Box::new(move || {
                drop((capture, in_work));
                panic!("owner drop should skip the queue");
            }),
        )
        .unwrap();
    let (dropped, observed_owner) = mpsc::channel();
    thread::spawn(move || {
        drop(owner);
        dropped.send(()).unwrap();
    });

    assert_eq!(observed_owner.recv_timeout(LIMIT), Ok(()));
    assert!(matches!(
        observed_drop.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    assert_eq!(
        kept.spawn(runtime.handle(), Box::new(|| {}))
            .err()
            .unwrap()
            .reason(),
        Rejection::Sealed
    );
    drop(release);
    assert_eq!(wait(&runtime, active), Ok(()));
    assert_eq!(wait(&runtime, queued), Err(TaskError::Cancelled));
    assert_eq!(
        observed_drop.recv_timeout(LIMIT),
        Ok((worker, Some(runtime.handle().id())))
    );
    assert_eq!(exit_observed.recv_timeout(LIMIT), Ok(()));
    drop(kept);
}

#[test]
fn a_worker_can_handoff_shutdown_to_the_backend() {
    let (owner, spawner) = Owner::start("shutdown-handoff", 1).unwrap();
    let runtime = runtime();
    let transient = runtime.handle().clone();
    let (ready, observed) = mpsc::channel();
    let ticket = spawner
        .spawn(
            runtime.handle(),
            Box::new(move || {
                ready.send(owner.shutdown(&transient)).ok().unwrap();
            }),
        )
        .unwrap();

    let shutdown = observed.recv_timeout(LIMIT).unwrap();
    assert!(wait(&runtime, shutdown).is_ok());
    assert_eq!(wait(&runtime, ticket), Ok(()));
}

#[test]
fn same_pool_worker_waiting_on_shutdown_gets_a_checked_error() {
    let (owner, spawner) = Owner::start("shutdown-self", 1).unwrap();
    let runtime = runtime();
    let transient = runtime.handle().clone();
    let (reported, observed) = mpsc::channel();
    let ticket = spawner
        .spawn(
            runtime.handle(),
            Box::new(move || {
                let result = transient.block_on(async {
                    let shutdown = owner.shutdown(&transient);
                    tokio::time::timeout(LIMIT, shutdown).await
                });
                reported.send(result).unwrap();
            }),
        )
        .unwrap();

    assert_eq!(
        observed.recv_timeout(LIMIT).unwrap().unwrap(),
        Err(ShutdownError(ShutdownFailure::CalledFromWorker))
    );
    assert_eq!(wait(&runtime, ticket), Ok(()));
}

#[test]
fn slow_cleanup_keeps_shutdown_pending_until_resident_resource_drops() {
    let (owner, spawner) = Owner::start("slow-cleanup", 1).unwrap();
    let runtime = runtime();
    let (capture, observed_drop) = drop_probe();
    let (started, observed_start) = mpsc::channel();
    let (release, hold) = mpsc::channel::<()>();
    let ticket = spawner
        .spawn(
            runtime.handle(),
            Box::new(move || {
                started.send(thread::current().id()).unwrap();
                let _ = hold.recv();
                drop(capture);
            }),
        )
        .unwrap();
    let worker = observed_start.recv_timeout(LIMIT).unwrap();
    let mut shutdown = Box::pin(owner.shutdown(runtime.handle()));
    let mut context = Context::from_waker(std::task::Waker::noop());
    assert!(shutdown.as_mut().poll(&mut context).is_pending());
    assert!(matches!(
        observed_drop.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    drop(release);

    assert!(wait(&runtime, shutdown).is_ok());
    assert_eq!(wait(&runtime, ticket), Ok(()));
    assert_eq!(
        observed_drop.recv_timeout(LIMIT),
        Ok((worker, Some(runtime.handle().id())))
    );
}

#[test]
fn a_run_loop_receipt_cannot_reserve_a_transient_join_slot_during_the_exit_epilogue() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_time()
        .build()
        .unwrap();
    let (owner, observed_epilogue, release) = start_with_blocked_epilogue();
    let mut shutdown = Box::pin(owner.shutdown(runtime.handle()));
    assert_eq!(observed_epilogue.recv_timeout(LIMIT), Ok(()));

    let mut context = Context::from_waker(Waker::noop());
    assert!(shutdown.as_mut().poll(&mut context).is_pending());
    let (sentinel, observed_sentinel) = mpsc::channel();
    let sentinel = runtime.spawn_blocking(move || sentinel.send(()).unwrap());
    assert_eq!(observed_sentinel.recv_timeout(LIMIT), Ok(()));
    drop(release);

    wait(&runtime, sentinel).unwrap();
    assert_eq!(wait(&runtime, shutdown), Ok(()));
}

fn assert_exit_timer_driver_survives(make_waker: impl FnOnce(oneshot::Sender<()>) -> Waker) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let (owner, observed_epilogue, release) = start_with_blocked_epilogue();
    let mut shutdown = Box::pin(owner.shutdown(runtime.handle()));
    assert_eq!(observed_epilogue.recv_timeout(LIMIT), Ok(()));

    let (raised, observed_panic_path) = oneshot::channel();
    let waker = make_waker(raised);
    assert!(
        shutdown
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    drop(waker);

    let sentinel = catch_unwind(AssertUnwindSafe(|| {
        wait(&runtime, async {
            observed_panic_path.await.unwrap();
            tokio::time::sleep(Duration::from_millis(1)).await;
            7
        })
    }));
    assert_eq!(sentinel.ok(), Some(7));

    assert_eq!(
        wait(&runtime, shutdown),
        Err(ShutdownError(ShutdownFailure::JoinUnavailable))
    );
    drop(release);
}

#[test]
fn panicking_shutdown_waker_cannot_unwind_the_transient_timer_driver() {
    struct PanicOnWake(Mutex<Option<oneshot::Sender<()>>>);

    impl Wake for PanicOnWake {
        fn wake(self: Arc<Self>) {
            let raised = self.0.lock().unwrap().take().unwrap();
            raised.send(()).unwrap();
            panic!("shutdown wake");
        }
    }

    assert_exit_timer_driver_survives(|raised| {
        Waker::from(Arc::new(PanicOnWake(Mutex::new(Some(raised)))))
    });
}

fn shutdown_waker_that_panics_on_drop(raised: oneshot::Sender<()>) -> Waker {
    struct PanicOnDrop(Option<oneshot::Sender<()>>);

    impl Wake for PanicOnDrop {
        fn wake(self: Arc<Self>) {
            drop(self);
        }
    }

    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            self.0.take().unwrap().send(()).unwrap();
            panic!("shutdown waker drop");
        }
    }

    Waker::from(Arc::new(PanicOnDrop(Some(raised))))
}

#[test]
fn panicking_shutdown_waker_drop_cannot_unwind_the_transient_timer_driver() {
    assert_exit_timer_driver_survives(shutdown_waker_that_panics_on_drop);
}

fn shutdown_wake_signal() -> (Waker, oneshot::Receiver<()>) {
    struct SignalWake(Mutex<Option<oneshot::Sender<()>>>);

    impl Wake for SignalWake {
        fn wake(self: Arc<Self>) {
            if let Some(signal) = self.0.lock().unwrap().take() {
                let _ = signal.send(());
            }
        }
    }

    let (signal, observed) = oneshot::channel();
    let waker = Waker::from(Arc::new(SignalWake(Mutex::new(Some(signal)))));
    (waker, observed)
}

#[test]
fn replacing_a_shutdown_waker_that_panics_on_drop_delivers_checked_failure() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let (owner, observed_epilogue, release) = start_with_blocked_epilogue();
    let mut shutdown = Box::pin(owner.shutdown(runtime.handle()));
    assert_eq!(observed_epilogue.recv_timeout(LIMIT), Ok(()));

    let (raised, mut observed_drop) = oneshot::channel();
    let old_waker = shutdown_waker_that_panics_on_drop(raised);
    assert!(
        shutdown
            .as_mut()
            .poll(&mut Context::from_waker(&old_waker))
            .is_pending()
    );
    drop(old_waker);
    assert!(matches!(
        observed_drop.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));

    let (replacement, observed_wake) = shutdown_wake_signal();
    let polled = shutdown
        .as_mut()
        .poll(&mut Context::from_waker(&replacement));
    // No thread has driven this runtime yet. The repoll released the old registration.
    observed_drop.try_recv().unwrap();

    let result = match polled {
        Poll::Ready(result) => result,
        Poll::Pending => {
            wait(&runtime, observed_wake).unwrap();
            wait(&runtime, shutdown)
        }
    };
    assert_eq!(result, Err(ShutdownError(ShutdownFailure::JoinUnavailable)));
    drop(release);
}

#[test]
fn a_shutdown_waker_panicking_after_a_concurrent_repoll_notifies_its_replacement() {
    struct PauseBeforePanic {
        entered: Sender<()>,
        resume: Mutex<Receiver<()>>,
    }

    impl Wake for PauseBeforePanic {
        fn wake(self: Arc<Self>) {
            self.entered.send(()).unwrap();
            self.resume.lock().unwrap().recv_timeout(LIMIT).unwrap();
            panic!("shutdown wake after replacement registration");
        }
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let (owner, observed_epilogue, release) = start_with_blocked_epilogue();
    let mut shutdown = Box::pin(owner.shutdown(runtime.handle()));
    assert_eq!(observed_epilogue.recv_timeout(LIMIT), Ok(()));

    let (entered, observed_old_wake) = mpsc::channel();
    let (resume_panic, resume) = mpsc::channel();
    let old_waker = Waker::from(Arc::new(PauseBeforePanic {
        entered,
        resume: Mutex::new(resume),
    }));
    assert!(
        shutdown
            .as_mut()
            .poll(&mut Context::from_waker(&old_waker))
            .is_pending()
    );
    drop(old_waker);

    thread::scope(|scope| {
        let resume_panic = resume_panic;
        let (done, drive) = oneshot::channel();
        let transient = &runtime;
        let driver = scope.spawn(move || {
            transient.block_on(async {
                let _ = drive.await;
                tokio::time::sleep(Duration::from_millis(1)).await;
                7
            })
        });
        assert_eq!(observed_old_wake.recv_timeout(LIMIT), Ok(()));

        let (replacement, mut observed_wake) = shutdown_wake_signal();
        let mut context = Context::from_waker(&replacement);
        assert!(shutdown.as_mut().poll(&mut context).is_pending());
        assert!(matches!(
            observed_wake.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        resume_panic.send(()).unwrap();

        wait(&runtime, observed_wake).unwrap();
        assert_eq!(
            shutdown.as_mut().poll(&mut context),
            Poll::Ready(Err(ShutdownError(ShutdownFailure::JoinUnavailable)))
        );
        done.send(()).unwrap();
        assert_eq!(driver.join().unwrap(), 7);
    });
    drop(release);
}

#[test]
fn a_self_scheduling_panicking_exit_waker_allocates_only_one_payload_per_shutdown() {
    use std::sync::atomic::AtomicUsize;

    struct ScheduleAndPanic {
        waiter: Waker,
        allocations: Arc<AtomicUsize>,
    }

    impl Wake for ScheduleAndPanic {
        fn wake(self: Arc<Self>) {
            self.waiter.wake_by_ref();
            let payload = vec![23_u8; 64];
            self.allocations.fetch_add(1, Ordering::Relaxed);
            panic_any(payload);
        }
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let (owner, observed_epilogue, release) = start_with_blocked_epilogue();
    let mut shutdown = Box::pin(owner.shutdown(runtime.handle()));
    assert_eq!(observed_epilogue.recv_timeout(LIMIT), Ok(()));
    let allocations = Arc::new(AtomicUsize::new(0));

    let outcome = wait(
        &runtime,
        poll_fn(|cx| {
            let waker = Waker::from(Arc::new(ScheduleAndPanic {
                waiter: cx.waker().clone(),
                allocations: Arc::clone(&allocations),
            }));
            match shutdown.as_mut().poll(&mut Context::from_waker(&waker)) {
                Poll::Ready(outcome) => Poll::Ready(Some(outcome)),
                Poll::Pending if allocations.load(Ordering::Relaxed) >= 4 => Poll::Ready(None),
                Poll::Pending => Poll::Pending,
            }
        }),
    );
    assert_eq!(
        outcome,
        Some(Err(ShutdownError(ShutdownFailure::JoinUnavailable)))
    );
    assert_eq!(allocations.load(Ordering::Relaxed), 1);
    let sentinel = runtime.spawn_blocking(|| 7);
    assert_eq!(wait(&runtime, sentinel).unwrap(), 7);
    drop(release);
}

#[test]
fn a_shutdown_polled_on_another_runtime_uses_the_driven_supplied_timer() {
    let transient = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let ambient = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (owner, observed_epilogue, release) = start_with_blocked_epilogue();
    let mut shutdown = Box::pin(owner.shutdown(transient.handle()));
    assert_eq!(observed_epilogue.recv_timeout(LIMIT), Ok(()));

    let (waker, observed_wake) = shutdown_wake_signal();
    {
        let _context = ambient.enter();
        assert!(
            shutdown
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
    }

    wait(&transient, observed_wake).unwrap();
    let sentinel = transient.spawn_blocking(|| 7);
    assert_eq!(wait(&transient, sentinel).unwrap(), 7);
    drop(release);
    assert_eq!(wait(&transient, shutdown), Ok(()));
}

#[test]
fn a_timerless_transient_runtime_reports_unavailable_while_the_epilogue_is_blocked() {
    let transient = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let waiter = runtime();
    let (owner, observed_epilogue, release) = start_with_blocked_epilogue();
    let shutdown = owner.shutdown(transient.handle());
    assert_eq!(observed_epilogue.recv_timeout(LIMIT), Ok(()));

    assert_eq!(
        wait(&waiter, shutdown),
        Err(ShutdownError(ShutdownFailure::JoinUnavailable))
    );
    let sentinel = transient.spawn_blocking(|| 7);
    assert_eq!(wait(&waiter, sentinel).unwrap(), 7);
    drop(release);
}

#[test]
fn a_stopped_transient_timer_reports_unavailable_while_the_epilogue_is_blocked() {
    let transient = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let waiter = runtime();
    let (owner, observed_epilogue, release) = start_with_blocked_epilogue();
    let shutdown = owner.shutdown(transient.handle());
    assert_eq!(observed_epilogue.recv_timeout(LIMIT), Ok(()));
    drop(transient);

    assert_eq!(
        wait(&waiter, shutdown),
        Err(ShutdownError(ShutdownFailure::JoinUnavailable))
    );
    drop(release);
}

#[test]
fn a_healthy_pool_reports_join_unavailable_with_a_shut_down_transient_runtime() {
    let (owner, _) = Owner::start("join-unavailable", 1).unwrap();
    let transient = runtime();
    let stopped = transient.handle().clone();
    drop(transient);
    let waiter = runtime();

    assert_eq!(
        wait(&waiter, owner.shutdown(&stopped)),
        Err(ShutdownError(ShutdownFailure::JoinUnavailable))
    );
}

#[test]
fn dropping_the_shutdown_waiter_does_not_move_queued_cleanup_to_its_caller() {
    let (owner, spawner) = Owner::start("shutdown-drop", 1).unwrap();
    let runtime = runtime();
    let (active, worker, release) = start_blocked(&spawner, &runtime);
    let (capture, observed_drop) = drop_probe();
    let queued = spawner
        .spawn(
            runtime.handle(),
            Box::new(move || {
                drop(capture);
                panic!("shutdown sealed the queue");
            }),
        )
        .unwrap();

    drop(owner.shutdown(runtime.handle()));
    assert!(matches!(
        observed_drop.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    drop(release);

    assert_eq!(wait(&runtime, active), Ok(()));
    assert_eq!(wait(&runtime, queued), Err(TaskError::Cancelled));
    assert_eq!(
        observed_drop.recv_timeout(LIMIT),
        Ok((worker, Some(runtime.handle().id())))
    );
}

#[test]
fn worker_failure_quarantines_pending_work_and_cannot_report_a_clean_shutdown() {
    let (owner, spawner) = Owner::start("failure", 1).unwrap();
    let runtime = runtime();
    let (started, observed_start) = mpsc::channel();
    let (release, hold) = mpsc::channel::<()>();
    let failing = spawner
        .spawn(
            runtime.handle(),
            Box::new(move || {
                started.send(()).unwrap();
                let _ = hold.recv();
                panic_any(FailingPayloadDrop);
            }),
        )
        .unwrap();
    assert_eq!(observed_start.recv_timeout(LIMIT), Ok(()));
    let (capture, observed_drop) = drop_probe();
    let pending = spawner
        .spawn(
            runtime.handle(),
            Box::new(move || {
                drop(capture);
                panic!("failed last worker cannot run pending work");
            }),
        )
        .unwrap();
    drop(release);

    assert_eq!(wait(&runtime, failing), Err(TaskError::Panicked));
    assert_eq!(wait(&runtime, pending), Err(TaskError::WorkerFailed));
    assert_eq!(
        spawner
            .spawn(runtime.handle(), Box::new(|| {}))
            .err()
            .unwrap()
            .reason(),
        Rejection::WorkerFailed
    );
    assert_eq!(
        wait(&runtime, owner.shutdown(runtime.handle())),
        Err(ShutdownError(ShutdownFailure::WorkerFailed))
    );
    drop(spawner);
    assert!(matches!(
        observed_drop.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
}

/// An elastic pool runs every long job at once, beyond its resident workers: each
/// submission that finds no waiting worker starts one more, never refuses for a count.
/// Extra workers end once they find no pending work, and shutdown joins them all.
#[test]
fn elastic_pool_runs_more_long_jobs_than_its_resident_workers_and_retires_the_extras() {
    let (owner, spawner) = Owner::start_elastic("elastic", 2).unwrap();
    let runtime = runtime();
    let jobs: Vec<_> = (0..7).map(|_| start_blocked(&spawner, &runtime)).collect();
    let threads: std::collections::HashSet<_> = jobs.iter().map(|(_, thread, _)| *thread).collect();
    assert_eq!(threads.len(), 7, "each long job has its own worker");
    for (ticket, _, release) in jobs {
        drop(release);
        assert_eq!(wait(&runtime, ticket), Ok(()));
    }
    // The extra workers end on their own; a resident worker takes the next job.
    let deadline = std::time::Instant::now() + LIMIT;
    while {
        let queue = spawner.shared.lock();
        queue.live_workers > 2 || queue.idle < 2
    } {
        assert!(std::time::Instant::now() < deadline, "extra workers stay");
        thread::sleep(Duration::from_millis(1));
    }
    let (ticket, _, release) = start_blocked(&spawner, &runtime);
    assert_eq!(spawner.shared.lock().live_workers, 2);
    drop(release);
    assert_eq!(wait(&runtime, ticket), Ok(()));
    assert!(wait(&runtime, owner.shutdown(runtime.handle())).is_ok());
}

/// Shutdown seals the pool and waits for extra workers still running a job.
#[test]
fn elastic_shutdown_joins_a_running_extra_worker() {
    let (owner, spawner) = Owner::start_elastic("elastic-shutdown", 1).unwrap();
    let runtime = runtime();
    let (first, _, release_first) = start_blocked(&spawner, &runtime);
    let (second, _, release_second) = start_blocked(&spawner, &runtime);
    let mut shutdown = Box::pin(owner.shutdown(runtime.handle()));
    assert!(
        spawner.spawn(runtime.handle(), Box::new(|| {})).is_err(),
        "a sealed pool starts no worker"
    );
    drop(release_first);
    assert_eq!(wait(&runtime, first), Ok(()));
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(50), &mut shutdown).await
            })
            .is_err(),
        "the extra worker still runs its job"
    );
    drop(release_second);
    assert_eq!(wait(&runtime, second), Ok(()));
    assert!(wait(&runtime, shutdown).is_ok());
}

/// Twenty long jobs, as twenty open viewer windows hold, each run on their own worker of a
/// pool started with two; after shutdown the pool refuses work as sealed, not as full.
#[test]
fn elastic_pool_runs_twenty_long_jobs_at_once_and_refuses_only_when_sealed() {
    let (owner, spawner) = Owner::start_elastic("elastic-twenty", 2).unwrap();
    let runtime = runtime();
    let jobs: Vec<_> = (0..20).map(|_| start_blocked(&spawner, &runtime)).collect();
    let threads: std::collections::HashSet<_> = jobs.iter().map(|(_, thread, _)| *thread).collect();
    assert_eq!(threads.len(), 20, "each long job has its own worker");
    for (ticket, _, release) in jobs {
        drop(release);
        assert_eq!(wait(&runtime, ticket), Ok(()));
    }
    assert!(wait(&runtime, owner.shutdown(runtime.handle())).is_ok());
    let refused = spawner
        .spawn(runtime.handle(), Box::new(|| {}))
        .err()
        .expect("a shut down pool refuses work");
    assert_eq!(refused.reason(), Rejection::Sealed);
}

fn no_thread(_: Builder, _: Work) -> io::Result<JoinHandle<()>> {
    Err(io::Error::other("no more threads"))
}

/// An elastic pool that finds no waiting worker and cannot start a thread refuses the
/// submission as `ThreadUnavailable`, not as a full queue, and returns the work intact.
#[test]
fn elastic_pool_refuses_as_thread_unavailable_when_a_thread_cannot_start() {
    let (owner, spawner) =
        Owner::start_with("elastic-no-thread", 1, Some(no_thread), Builder::spawn).unwrap();
    let runtime = runtime();
    // The resident worker is busy, so the next submission needs another thread.
    let (busy, _, release) = start_blocked(&spawner, &runtime);
    let (ran, observed) = mpsc::channel();
    let refused = spawner
        .spawn(runtime.handle(), Box::new(move || ran.send(()).unwrap()))
        .err()
        .expect("no thread for the work");
    assert_eq!(refused.reason(), Rejection::ThreadUnavailable);
    assert_eq!(
        refused.to_string(),
        "a resident worker thread could not be started"
    );
    refused.into_work()();
    assert_eq!(observed.try_recv(), Ok(()), "the work comes back intact");
    {
        let queue = spawner.shared.lock();
        assert_eq!(queue.live_workers, 1);
        assert!(queue.pending.is_empty() && queue.extra.is_empty());
    }
    drop(release);
    assert_eq!(wait(&runtime, busy), Ok(()));
    assert!(wait(&runtime, owner.shutdown(runtime.handle())).is_ok());
}
