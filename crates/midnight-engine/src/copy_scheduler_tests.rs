use super::*;
#[path = "copy_scheduler_fixture.rs"]
mod fixture;
use fixture::{Case, Point, Release};

const OBSERVE: Duration = Duration::from_secs(8);
const QUIET: Duration = Duration::from_millis(350);

fn report_event(event: &CopyEvent, phase: &str) {
    match event {
        CopyEvent::Progress(p) if p.state.is_terminal() => {
            eprintln!("scheduler {phase} terminal: {p:?}");
        }
        CopyEvent::Failure(f) => eprintln!("scheduler {phase} failure: {f:?}"),
        CopyEvent::Conflict(c) => eprintln!("scheduler {phase} conflict: {c:?}"),
        _ => {}
    }
}

fn report_active(h: &Harness, phase: &str) {
    let active: Vec<_> = {
        let registry = h.controller.registry.lock().unwrap();
        registry
            .iter()
            .map(|(id, registered)| (*id, registered.status.clone()))
            .collect()
    };
    eprintln!(
        "scheduler {phase} available I/O: {}",
        h.io.available_permits()
    );
    for (id, status) in active {
        let snapshot = status.lock().unwrap().clone();
        eprintln!("scheduler {phase} active {id:?}: {snapshot:?}");
    }
}

async fn progress_where(
    h: &mut Harness,
    id: u64,
    predicate: impl Fn(&CopyProgress) -> bool,
) -> bool {
    tokio::time::timeout(OBSERVE, async {
        loop {
            match h.events.recv().await {
                Some(Event::Copy(event)) => {
                    report_event(&event, "observation");
                    if let CopyEvent::Progress(p) = event {
                        let matched = p.id == OperationId(id) && predicate(&p);
                        if p.state.is_terminal() {
                            h.terminals.insert(p.id, p);
                        }
                        if matched {
                            return true;
                        }
                    }
                }
                None => return false,
                _ => {}
            }
        }
    })
    .await
    .unwrap_or(false)
}

async fn drain(mut h: Harness, case: &Case, ids: &[u64]) -> Vec<Arc<CopyProgress>> {
    case.control.release_all();
    let mut done = Vec::new();
    for id in ids {
        if let Some(p) = h.terminals.remove(&OperationId(*id)) {
            eprintln!("scheduler drain cached terminal: {p:?}");
            if !done.iter().any(|old: &Arc<CopyProgress>| old.id == p.id) {
                done.push(p);
            }
        }
    }
    let drained = tokio::time::timeout(OBSERVE, async {
        while done.len() < ids.len() {
            match h.events.recv().await {
                Some(Event::Copy(event)) => {
                    report_event(&event, "drain");
                    match event {
                        CopyEvent::Progress(p)
                            if p.state.is_terminal()
                                && ids.contains(&p.id.0)
                                && !done.iter().any(|old: &Arc<CopyProgress>| old.id == p.id) =>
                        {
                            done.push(p)
                        }
                        CopyEvent::Failure(f) => h.controller.cancel(f.id),
                        CopyEvent::Conflict(c) => h.controller.cancel(c.id),
                        _ => {}
                    }
                }
                None => break,
                _ => {}
            }
        }
    })
    .await
    .is_ok();
    eprintln!("scheduler drain requested={ids:?} drained={drained} terminals={done:?}");
    report_active(&h, "after drain");
    h.close().await;
    assert!(
        drained,
        "native calls were released but terminal owners did not drain"
    );
    assert_eq!(done.len(), ids.len());
    assert!(
        !case.control.timed_out(),
        "a native hold reached its emergency timeout"
    );
    done
}

fn completed(done: &[Arc<CopyProgress>], id: u64) -> &CopyProgress {
    let result = done.iter().find(|p| p.id == OperationId(id)).unwrap();
    assert_eq!(result.state, CopyOperationState::Completed, "{result:?}");
    assert!(result.errors.is_empty());
    result
}

async fn overlap(moves: [bool; 3], names: [&str; 3]) {
    let case = Case::new(names, false, None);
    let _release = Release(case.control.clone());
    let mut h = Harness::with_slots(Some(case.service()), 2);
    for (index, moving) in moves.iter().enumerate() {
        case.control
            .arm(index, if *moving { Point::Moved } else { Point::Read });
    }
    let mut submitted = Vec::new();
    let mut checkpoint = "before observation";
    let observations = tokio::time::timeout(OBSERVE, async {
        checkpoint = "enqueue first";
        case.enqueue(&h, 0, 1, moves[0], false).await;
        submitted.push(1);
        checkpoint = "wait first native hold";
        let first = case
            .control
            .wait(
                0,
                if moves[0] { Point::Moved } else { Point::Read },
                OBSERVE,
            )
            .await;
        eprintln!("scheduler first native hold reached={first}");
        checkpoint = "enqueue second";
        case.enqueue(&h, 1, 2, moves[1], false).await;
        submitted.push(2);
        checkpoint = "wait second native hold";
        let second = case
            .control
            .wait(
                1,
                if moves[1] { Point::Moved } else { Point::Read },
                OBSERVE,
            )
            .await;
        eprintln!("scheduler second native hold reached={second}");
        checkpoint = "enqueue third";
        case.enqueue(&h, 2, 3, moves[2], false).await;
        submitted.push(3);
        checkpoint = "wait third Resources progress";
        let waiting = progress_where(&mut h, 3, |p| {
            p.wait_reason == Some(TransferWaitReason::Resources)
        })
        .await;
        eprintln!("scheduler third Resources progress observed={waiting}");
        checkpoint = "check third cannot enter while saturated";
        let third_early = case
            .control
            .wait(
                2,
                if moves[2] {
                    Point::MoveAttempt
                } else {
                    Point::ReadObserved
                },
                QUIET,
            )
            .await;
        let saturated = h.io.available_permits() == 0;
        eprintln!("scheduler third entered early={third_early} I/O saturated={saturated}");
        case.control
            .release(0, if moves[0] { Point::Moved } else { Point::Read });
        checkpoint = "wait third native hold after first release";
        let third = case
            .control
            .wait(
                2,
                if moves[2] { Point::Moved } else { Point::Read },
                OBSERVE,
            )
            .await;
        eprintln!("scheduler third native hold after first release={third}");
        checkpoint = "observations complete";
        (first, second, waiting, third_early, saturated, third)
    })
    .await;
    eprintln!(
        "scheduler overlap submitted={submitted:?} checkpoint={checkpoint} observations={observations:?}"
    );
    report_active(&h, "after observation");
    let done = drain(h, &case, &submitted).await;
    assert_eq!(submitted, [1, 2, 3]);
    assert_eq!(observations.unwrap(), (true, true, true, false, true, true));
    for (index, moving) in moves.iter().enumerate() {
        let p = completed(&done, index as u64 + 1);
        assert_eq!(
            p.verified_bytes,
            if *moving {
                0
            } else {
                case.bytes(index).len() as u64
            }
        );
        if *moving {
            assert_eq!(p.copied_bytes, 0);
        }
        assert_eq!(fs::read(case.output(index)).unwrap(), case.bytes(index));
        assert_eq!(case.sources[index].exists(), !moving);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn exact_two_slots_copy_same_folders_holds_two_native_reads_and_excludes_third() {
    overlap([false; 3], ["one", "two", "three"]).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn exact_two_slots_move_same_folders_holds_two_committed_moves_and_excludes_third() {
    overlap([true; 3], ["one", "two", "three"]).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn exact_two_slots_mixed_same_folders_preserves_proof_and_move_removal() {
    overlap([true, false, true], ["one", "two", "three"]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_slot_reports_resources_until_cancelled_predecessor_actual_stage_cleanup_returns() {
    let case = Case::new(["one", "two", "three"], false, None);
    let _release = Release(case.control.clone());
    case.control.arm(0, Point::Read);
    case.control.arm(0, Point::Cleanup);
    case.control.arm(1, Point::Read);
    let mut h = Harness::with_slots(Some(case.service()), 1);
    let observed = tokio::time::timeout(OBSERVE, async {
        case.enqueue(&h, 0, 1, false, false).await;
        let read = case.control.wait(0, Point::Read, OBSERVE).await;
        case.enqueue(&h, 1, 2, false, false).await;
        let resources = progress_where(&mut h, 2, |p| {
            p.wait_reason == Some(TransferWaitReason::Resources)
        })
        .await;
        h.controller.cancel(OperationId(1));
        case.control.release(0, Point::Read);
        let cleanup = case.control.wait(0, Point::Cleanup, OBSERVE).await;
        let entered_early = case.control.wait(1, Point::ReadObserved, QUIET).await;
        case.control.release(0, Point::Cleanup);
        let entered = case.control.wait(1, Point::Read, OBSERVE).await;
        (read, resources, cleanup, entered_early, entered)
    })
    .await;
    let done = drain(h, &case, &[1, 2]).await;
    assert_eq!(observed.unwrap(), (true, true, true, false, true));
    assert!(
        case.control
            .before((0, Point::Dropped), (1, Point::ReadObserved))
    );
    assert_eq!(
        done.iter().find(|p| p.id == OperationId(1)).unwrap().state,
        CopyOperationState::Cancelled
    );
    completed(&done, 2);
    assert_eq!(fs::read(case.output(1)).unwrap(), case.bytes(1));
    assert!(!case.output(0).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unfilled_earlier_reservation_blocks_opt_in_but_later_immediate_bypasses_it() {
    let case = Case::new(["one", "two", "three"], false, None);
    let _release = Release(case.control.clone());
    for index in 0..3 {
        case.control.arm(index, Point::Read);
    }
    case.control.arm(0, Point::Cleanup);
    let mut h = Harness::with_slots(Some(case.service()), 2);
    let reservation = h.controller.reserve().unwrap();
    let observed = tokio::time::timeout(OBSERVE, async {
        case.enqueue(&h, 1, 1, false, true).await;
        let waiting = progress_where(&mut h, 1, |p| {
            p.wait_reason == Some(TransferWaitReason::EarlierTransfers)
        })
        .await;
        case.enqueue(&h, 2, 2, false, false).await;
        let bypass = case.control.wait(2, Point::Read, OBSERVE).await;
        // Admission order comes from the reserved slot, not this fresh payload ID.
        h.controller
            .enqueue_reserved(case.request(0, 3, false), reservation)
            .await;
        let prior = case.control.wait(0, Point::Read, OBSERVE).await;
        h.controller.cancel(OperationId(3));
        case.control.release(0, Point::Read);
        let cleanup = case.control.wait(0, Point::Cleanup, OBSERVE).await;
        let early = case.control.wait(1, Point::ReadObserved, QUIET).await;
        case.control.release(0, Point::Cleanup);
        let after = case.control.wait(1, Point::Read, OBSERVE).await;
        // C is still held: B cannot depend on a later reservation or on itself.
        (waiting, bypass, prior, cleanup, early, after)
    })
    .await;
    let done = drain(h, &case, &[1, 2, 3]).await;
    assert_eq!(observed.unwrap(), (true, true, true, true, false, true));
    assert!(
        case.control
            .before((0, Point::Dropped), (1, Point::ReadObserved))
    );
    completed(&done, 1);
    completed(&done, 2);
    assert_eq!(
        done.iter().find(|p| p.id == OperationId(3)).unwrap().state,
        CopyOperationState::Cancelled
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn acknowledged_pause_keeps_earlier_dependency_unfinished_until_real_cleanup() {
    let case = Case::new(["one", "two", "three"], false, None);
    let _release = Release(case.control.clone());
    case.control.arm(0, Point::Read);
    case.control.arm(0, Point::Cleanup);
    case.control.arm(1, Point::Read);
    let mut h = Harness::with_slots(Some(case.service()), 2);
    let observed = tokio::time::timeout(OBSERVE, async {
        case.enqueue(&h, 0, 1, false, false).await;
        let read = case.control.wait(0, Point::Read, OBSERVE).await;
        h.controller.set_paused(OperationId(1), true).await;
        case.control.release(0, Point::Read);
        let paused = progress_where(&mut h, 1, |p| p.paused && p.pause_requested).await;
        case.enqueue(&h, 1, 2, false, true).await;
        let waiting = progress_where(&mut h, 2, |p| {
            p.wait_reason == Some(TransferWaitReason::EarlierTransfers)
        })
        .await;
        let early = case.control.wait(1, Point::ReadObserved, QUIET).await;
        h.controller.cancel(OperationId(1));
        let cleanup = case.control.wait(0, Point::Cleanup, OBSERVE).await;
        let during_cleanup = case.control.wait(1, Point::ReadObserved, QUIET).await;
        case.control.release(0, Point::Cleanup);
        let after = case.control.wait(1, Point::Read, OBSERVE).await;
        (read, paused, waiting, early, cleanup, during_cleanup, after)
    })
    .await;
    let done = drain(h, &case, &[1, 2]).await;
    assert_eq!(
        observed.unwrap(),
        (true, true, true, false, true, false, true)
    );
    assert!(
        case.control
            .before((0, Point::Dropped), (1, Point::ReadObserved))
    );
    completed(&done, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_write_decision_and_checked_cleanup_both_precede_finished_dependency_release() {
    let case = Case::new(["one", "two", "three"], false, Some(0));
    let _release = Release(case.control.clone());
    case.control.arm(0, Point::Cleanup);
    case.control.arm(1, Point::Read);
    let mut h = Harness::with_slots(Some(case.service()), 2);
    let observed = tokio::time::timeout(OBSERVE, async {
        case.enqueue(&h, 0, 1, false, false).await;
        let failure = loop {
            match h.events.recv().await {
                Some(Event::Copy(CopyEvent::Failure(f))) => {
                    eprintln!("scheduler observation failure: {f:?}");
                    break f;
                }
                None => return (false, false, false, false, false),
                _ => {}
            }
        };
        case.enqueue(&h, 1, 2, false, true).await;
        let waiting = progress_where(&mut h, 2, |p| {
            p.wait_reason == Some(TransferWaitReason::EarlierTransfers)
        })
        .await;
        let before_decision = case.control.wait(1, Point::ReadObserved, QUIET).await;
        h.controller
            .decide(CopyDecision {
                id: failure.id,
                version: failure.version,
                choice: CopyChoice::Skip,
            })
            .await;
        let cleanup = case.control.wait(0, Point::Cleanup, OBSERVE).await;
        let during_cleanup = case.control.wait(1, Point::ReadObserved, QUIET).await;
        case.control.release(0, Point::Cleanup);
        let after = case.control.wait(1, Point::Read, OBSERVE).await;
        (waiting, before_decision, cleanup, during_cleanup, after)
    })
    .await;
    let done = drain(h, &case, &[1, 2]).await;
    assert_eq!(observed.unwrap(), (true, false, true, false, true));
    assert!(
        case.control
            .before((0, Point::Dropped), (1, Point::ReadObserved))
    );
    let failed = done.iter().find(|p| p.id == OperationId(1)).unwrap();
    assert!(!failed.errors.is_empty() || failed.skipped > 0);
    assert!(case.sources[0].exists());
    completed(&done, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_target_conflict_keeps_opt_in_waiter_behind_decision_and_cancel_cleanup() {
    let case = Case::new(["one", "two", "three"], false, None);
    fs::write(case.output(0), b"existing target must survive").unwrap();
    let _release = Release(case.control.clone());
    case.control.arm(0, Point::Read);
    case.control.arm(0, Point::Cleanup);
    case.control.arm(1, Point::Read);
    let mut h = Harness::with_slots(Some(case.service()), 2);
    let observed = tokio::time::timeout(OBSERVE, async {
        case.enqueue(&h, 0, 1, false, false).await;
        let conflict = loop {
            match h.events.recv().await {
                Some(Event::Copy(CopyEvent::Conflict(c))) => {
                    eprintln!("scheduler observation conflict: {c:?}");
                    break c;
                }
                None => return (false, false, false, false, false, false),
                _ => {}
            }
        };
        case.enqueue(&h, 1, 2, false, true).await;
        let waiting = progress_where(&mut h, 2, |p| {
            p.wait_reason == Some(TransferWaitReason::EarlierTransfers)
        })
        .await;
        let before = case.control.wait(1, Point::ReadObserved, QUIET).await;
        h.controller
            .decide(CopyDecision {
                id: conflict.id,
                version: conflict.version,
                choice: CopyChoice::Rename("renamed-one".into()),
            })
            .await;
        let read = case.control.wait(0, Point::Read, OBSERVE).await;
        h.controller.cancel(OperationId(1));
        case.control.release(0, Point::Read);
        let cleanup = case.control.wait(0, Point::Cleanup, OBSERVE).await;
        let during = case.control.wait(1, Point::ReadObserved, QUIET).await;
        case.control.release(0, Point::Cleanup);
        let after = case.control.wait(1, Point::Read, OBSERVE).await;
        (waiting, before, read, cleanup, during, after)
    })
    .await;
    let done = drain(h, &case, &[1, 2]).await;
    assert_eq!(observed.unwrap(), (true, false, true, true, false, true));
    assert!(
        case.control
            .before((0, Point::Dropped), (1, Point::ReadObserved))
    );
    completed(&done, 2);
    assert_eq!(
        fs::read(case.output(0)).unwrap(),
        b"existing target must survive"
    );
    assert!(!case.destination.join("renamed-one").exists());
}

async fn blocked_delivery(terminal: bool) {
    let case = Case::new(["one", "two", "three"], false, None);
    let _release = Release(case.control.clone());
    case.control.arm(0, Point::Read);
    case.control.arm(1, Point::Read);
    let mut h = Harness::with_slots(Some(case.service()), 2);
    let observed = tokio::time::timeout(OBSERVE, async {
        if terminal {
            case.enqueue(&h, 0, 1, false, false).await;
            if !case.control.wait(0, Point::Read, OBSERVE).await {
                return (false, false, false);
            }
            while h.events.try_recv().is_ok() {}
        }
        let filler = Arc::new(CopyProgress::queued(
            OperationId(999),
            case.destination.clone(),
            0,
            TransferIntent::Copy,
        ));
        while h
            .controller
            .outbox
            .sender
            .try_send(Event::Copy(CopyEvent::Progress(filler.clone())))
            .is_ok()
        {}
        if !terminal {
            case.enqueue(&h, 0, 1, false, false).await;
        }
        let first = case.control.wait(0, Point::Read, OBSERVE).await;
        case.control.release(0, Point::Read);
        if terminal {
            // The provider's actual stage destructor has returned while delivery
            // still has no capacity. Observe actual completion, not Running.
            if !case.control.wait(0, Point::Dropped, OBSERVE).await {
                return (false, false, false);
            }
            while h
                .controller
                .registry
                .lock()
                .unwrap()
                .contains_key(&OperationId(1))
            {
                tokio::task::yield_now().await;
            }
        }
        case.enqueue(&h, 1, 2, false, false).await;
        let other = case.control.wait(1, Point::Read, OBSERVE).await;
        (first, other, h.controller.outbox.sender.capacity() == 0)
    })
    .await;
    let done = drain(h, &case, &[1, 2]).await;
    assert_eq!(observed.unwrap(), (true, true, true));
    completed(&done, 1);
    completed(&done, 2);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blocked_initial_outbox_does_not_stall_other_native_io() {
    blocked_delivery(false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blocked_terminal_outbox_does_not_stall_other_native_io() {
    blocked_delivery(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audited_opaque_provider_completes_and_cancels_with_actual_cleanup() {
    for cancelled in [false, true] {
        let case = Case::new(["one", "two", "three"], true, None);
        let _release = Release(case.control.clone());
        case.control.arm(0, Point::Read);
        case.control.arm(0, Point::Cleanup);
        let h = Harness::with_slots(Some(case.service()), 2);
        let observed = tokio::time::timeout(OBSERVE, async {
            case.enqueue(&h, 0, 1, false, false).await;
            let read = case.control.wait(0, Point::Read, OBSERVE).await;
            if cancelled {
                h.controller.cancel(OperationId(1));
            }
            case.control.release(0, Point::Read);
            let cleanup = !cancelled || case.control.wait(0, Point::Cleanup, OBSERVE).await;
            (read, cleanup)
        })
        .await;
        let done = drain(h, &case, &[1]).await;
        assert_eq!(observed.unwrap(), (true, true));
        assert_eq!(
            done[0].state,
            if cancelled {
                CopyOperationState::Cancelled
            } else {
                CopyOperationState::Completed
            }
        );
        if !cancelled {
            assert_eq!(fs::read(case.output(0)).unwrap(), case.bytes(0));
        }
    }
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn linux_actual_case_only_a_and_a_upper_have_distinct_native_objects_and_overlap() {
    overlap([false; 3], ["a", "A", "third"]).await;
}
