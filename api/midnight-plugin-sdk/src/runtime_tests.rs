use super::*;
use crate::host::tests::fixture;
use crate::outbox::Output;

fn origin(id: u64) -> Origin {
    Origin {
        id,
        session: 9,
        generation: 3,
    }
}

fn terminal(runtime: &Runtime, kind: u32) {
    let event = runtime.drain().unwrap().unwrap();
    assert_eq!(event.kind, kind);
    runtime.release(event.payload.id).unwrap();
    assert!(runtime.drain().unwrap().is_none());
}

#[test]
fn rejected_admission_has_no_event_and_accepted_work_finishes_once() {
    let (host, state, _) = fixture();
    let runtime = Runtime::new(host, Arc::new(AtomicU64::new(1)));
    assert_eq!(
        runtime.submit(origin(1), Box::new(|ctx| ctx.complete().unwrap())),
        abi::QUEUE_FULL
    );
    assert!(runtime.drain().unwrap().is_none());
    state.accept_work.store(true, Ordering::Relaxed);
    assert_eq!(
        runtime.submit(
            origin(1),
            Box::new(|ctx| {
                ctx.complete().unwrap();
                assert!(ctx.complete().is_err());
            })
        ),
        abi::OK
    );
    assert!(
        runtime.drain().unwrap().is_none(),
        "submit must not execute plugin work"
    );
    assert!(state.run_one());
    terminal(&runtime, abi::EVENT_COMPLETED);
    runtime.shutdown();
    assert!(runtime.quiescent());
}

#[test]
fn cancellation_before_run_sends_terminal_but_waits_for_job_cleanup() {
    let (host, state, _) = fixture();
    let runtime = Runtime::new(host, Arc::new(AtomicU64::new(1)));
    state.accept_work.store(true, Ordering::Relaxed);
    assert_eq!(
        runtime.submit(origin(1), Box::new(|_| panic!("cancelled work ran"))),
        abi::OK
    );
    runtime.cancel(1).unwrap();
    runtime.cancel(1).unwrap();
    terminal(&runtime, abi::EVENT_CANCELLED);
    runtime.shutdown();
    assert!(!runtime.quiescent());
    assert!(state.run_one());
    assert!(runtime.quiescent());
    assert_eq!(
        runtime.submit(origin(2), Box::new(|_| {})),
        abi::SHUTTING_DOWN
    );
}

#[test]
fn missing_completion_and_panics_produce_failure_instead_of_orphaned_requests() {
    let (host, state, _) = fixture();
    let runtime = Runtime::new(host, Arc::new(AtomicU64::new(1)));
    state.accept_work.store(true, Ordering::Relaxed);
    for (id, panic) in [(1, false), (2, true)] {
        assert_eq!(
            runtime.submit(
                origin(id),
                Box::new(move |_| {
                    assert!(!panic, "fixture plugin panic");
                })
            ),
            abi::OK
        );
        assert!(state.run_one());
        terminal(&runtime, abi::EVENT_FAILED);
    }
    runtime.shutdown();
    assert!(runtime.quiescent());
}

#[test]
fn asynchronous_read_releases_worker_and_resumes_only_after_validated_reply() {
    let (host, state, _) = fixture();
    let runtime = Runtime::new(host, Arc::new(AtomicU64::new(1)));
    state.accept_work.store(true, Ordering::Relaxed);
    state.accept_service.store(true, Ordering::Relaxed);
    assert_eq!(
        runtime.submit(
            origin(1),
            Box::new(|ctx| {
                ctx.read(
                    messages::ReadAt {
                        reader: 7,
                        revision: 8,
                        offset: 1u64 << 34,
                        length: 4,
                    },
                    Box::new(|ctx, reply, content| {
                        assert_eq!(reply.offset, 1u64 << 34);
                        assert!(content.as_ref().is_empty());
                        ctx.complete().unwrap();
                    }),
                )
                .unwrap();
            })
        ),
        abi::OK
    );
    assert!(state.run_one());
    assert_eq!(
        runtime.0.jobs.active(),
        0,
        "a pending read must not block a worker"
    );
    assert!(runtime.drain().unwrap().is_none());
    let (id, _) = lock(&state.services).pop().unwrap();
    let reply = messages::ReadReply {
        revision: 8,
        offset: 1u64 << 34,
        bytes: 0,
        eof: true,
        status: 0,
    };
    assert_eq!(
        runtime.reply(Origin { id, ..origin(1) }, reply, Content::default()),
        abi::OK
    );
    assert!(runtime.drain().unwrap().is_none());
    assert!(state.run_one());
    terminal(&runtime, abi::EVENT_COMPLETED);
    runtime.shutdown();
    assert!(runtime.quiescent());
}

fn progress() -> Output {
    Output::progress(&messages::Progress {
        completed: 1,
        total: Some(2),
        phase: 0,
    })
    .unwrap()
}

#[test]
fn output_backpressure_suspends_producers_and_resumes_after_drain_without_lost_wake() {
    let (host, state, _) = fixture();
    let runtime = Runtime::new(host, Arc::new(AtomicU64::new(1)));
    state.accept_work.store(true, Ordering::Relaxed);
    for id in 1..=33 {
        assert_eq!(
            runtime.submit(
                origin(id),
                Box::new(|ctx| {
                    ctx.send(progress(), Box::new(|ctx| ctx.complete().unwrap()))
                        .unwrap();
                })
            ),
            abi::OK
        );
        while state.run_one() {}
    }
    assert_eq!(
        runtime.0.jobs.active(),
        0,
        "blocked output cannot occupy a worker"
    );
    let first = runtime.drain().unwrap().unwrap();
    assert_eq!(first.kind, abi::EVENT_PROGRESS);
    runtime.release(first.payload.id).unwrap();
    assert!(
        state.run_one(),
        "draining data must resume the suspended producer"
    );
    let mut completed = 0;
    let mut data = 1;
    while let Some(event) = runtime.drain().unwrap() {
        if event.kind == abi::EVENT_COMPLETED {
            completed += 1;
        } else {
            data += 1;
        }
        runtime.release(event.payload.id).unwrap();
    }
    assert_eq!((data, completed), (33, 33));
    runtime.shutdown();
    assert!(runtime.quiescent());
}

#[test]
fn cancelled_pending_read_consumes_late_reply_without_running_continuation() {
    let (host, state, _) = fixture();
    let runtime = Runtime::new(host, Arc::new(AtomicU64::new(1)));
    state.accept_work.store(true, Ordering::Relaxed);
    state.accept_service.store(true, Ordering::Relaxed);
    assert_eq!(
        runtime.submit(
            origin(1),
            Box::new(|ctx| {
                ctx.read(
                    messages::ReadAt {
                        reader: 1,
                        revision: 2,
                        offset: 0,
                        length: 4,
                    },
                    Box::new(|_, _, _| panic!("cancelled continuation ran")),
                )
                .unwrap();
            })
        ),
        abi::OK
    );
    assert!(state.run_one());
    runtime.shutdown();
    terminal(&runtime, abi::EVENT_CANCELLED);
    assert!(!runtime.quiescent());
    let (id, _) = lock(&state.services).pop().unwrap();
    let mut reply = messages::ReadReply {
        revision: 99,
        offset: 0,
        bytes: 0,
        eof: true,
        status: 0,
    };
    assert_eq!(
        runtime.reply(
            Origin { id, ..origin(1) },
            reply.clone(),
            Content::default()
        ),
        abi::INVALID
    );
    assert!(!runtime.quiescent());
    reply.revision = 2;
    assert_eq!(
        runtime.reply(Origin { id, ..origin(1) }, reply, Content::default()),
        abi::OK
    );
    assert!(!state.run_one());
    assert!(runtime.quiescent());
}

#[test]
fn reused_request_id_cannot_hide_a_retained_old_context_from_shutdown() {
    let (host, state, _) = fixture();
    let runtime = Runtime::new(host, Arc::new(AtomicU64::new(1)));
    state.accept_work.store(true, Ordering::Relaxed);
    let held = Arc::new(Mutex::new(None));
    let captured = held.clone();
    assert_eq!(
        runtime.submit(
            origin(1),
            Box::new(move |ctx| {
                ctx.complete().unwrap();
                *lock(&captured) = Some(ctx);
            })
        ),
        abi::OK
    );
    state.run_one();
    terminal(&runtime, abi::EVENT_COMPLETED);
    assert_eq!(
        runtime.submit(origin(1), Box::new(|ctx| ctx.complete().unwrap())),
        abi::OK
    );
    state.run_one();
    terminal(&runtime, abi::EVENT_COMPLETED);
    runtime.shutdown();
    assert!(
        !runtime.quiescent(),
        "old context can still call host services"
    );
    lock(&held).take();
    assert!(runtime.quiescent());
}

#[test]
fn completed_but_retained_contexts_are_bounded_even_after_terminal_release() {
    let (host, state, _) = fixture();
    let runtime = Runtime::new(host, Arc::new(AtomicU64::new(1)));
    state.accept_work.store(true, Ordering::Relaxed);
    let held = Arc::new(Mutex::new(Vec::new()));
    for _ in 0..128 {
        let captured = held.clone();
        assert_eq!(
            runtime.submit(
                origin(1),
                Box::new(move |ctx| {
                    ctx.complete().unwrap();
                    lock(&captured).push(ctx);
                })
            ),
            abi::OK
        );
        assert!(state.run_one());
        terminal(&runtime, abi::EVENT_COMPLETED);
    }
    assert_eq!(runtime.submit(origin(1), Box::new(|_| {})), abi::QUEUE_FULL);
    lock(&held).clear();
    runtime.shutdown();
    assert!(runtime.quiescent());
}
