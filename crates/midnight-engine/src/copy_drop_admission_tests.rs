use super::*;

fn unpublished(admission: &Admission) {
    let mut state = admission.0.state.lock().unwrap();
    assert!(state.slots.is_empty());
    assert!(state.controls.is_empty());
    assert_eq!(state.finished.prepare_sequence(), Ok(1));
}

#[test]
fn staged_native_capacity_is_real_but_not_visible_to_finished_snapshots() {
    let admission = Admission::default();
    let stages: Vec<_> = (0..MAX_JOBS)
        .map(|_| admission.stage_drop().unwrap())
        .collect();
    assert_eq!(admission.capacity().available_permits(), 0);
    assert!(admission.stage_drop().is_err());
    unpublished(&admission);
    drop(stages);
    assert_eq!(admission.capacity().available_permits(), MAX_JOBS);
    unpublished(&admission);
    let returned: Vec<_> = (0..MAX_JOBS)
        .map(|_| admission.0.retirement.try_reserve().unwrap())
        .collect();
    assert!(admission.0.retirement.try_reserve().is_err());
    drop(returned);
}

#[test]
fn admission_contention_returns_empty_stage_capacity_without_publication() {
    let admission = Admission::default();
    let stage = admission.stage_drop().unwrap();
    let state = admission.0.state.lock().unwrap();
    let result = stage.publish();
    let returned = admission.capacity().available_permits();
    drop(state);
    assert!(matches!(result, Err(error) if error.kind == io::ErrorKind::WouldBlock));
    assert_eq!(returned, MAX_JOBS);
    unpublished(&admission);
}

#[test]
fn success_fence_contention_rolls_back_before_registering_finished() {
    let admission = Admission::default();
    let stage = admission.stage_drop().unwrap();
    let fence = admission.0.fence.try_registration().unwrap();
    let result = stage.publish();
    drop(fence);
    assert!(matches!(result, Err(error) if error.kind == io::ErrorKind::WouldBlock));
    assert_eq!(admission.capacity().available_permits(), MAX_JOBS);
    unpublished(&admission);
}

#[test]
fn shutdown_between_stage_and_publish_refuses_without_a_visible_owner() {
    let admission = Admission::default();
    let mut stage = admission.stage_drop().unwrap();
    let cancel = stage
        .owned
        .as_mut()
        .unwrap()
        .resources_mut()
        .pending_cancel()
        .unwrap();
    admission.shutdown();
    let result = stage.publish();
    assert!(cancel.is_cancelled());
    assert!(matches!(result, Err(error) if error.kind == io::ErrorKind::Interrupted));
    assert_eq!(admission.capacity().available_permits(), MAX_JOBS);
    unpublished(&admission);
}

#[test]
fn published_native_reservation_keeps_its_original_owner_and_earlier_snapshot() {
    let admission = Admission::default();
    let mut earlier = admission.stage_drop().unwrap().publish().unwrap();
    let cancel = earlier.owned.resources_mut().pending_cancel().unwrap();
    let observer = earlier.owned.resources_mut().finished_observer();
    assert_eq!(earlier.owned.resources_mut().admission_sequence(), 1);
    let mut later = admission.stage_drop().unwrap().publish().unwrap();
    let predecessors = later.owned.resources_mut().take_predecessors();
    assert_eq!(later.owned.resources_mut().admission_sequence(), 2);
    assert!(!predecessors.is_finished());
    assert!(!observer.is_finished());
    assert_eq!(admission.capacity().available_permits(), MAX_JOBS - 2);
    assert!(
        admission.0.state.lock().unwrap().controls.is_empty(),
        "pre-ID native drops do not create operation controls"
    );
    admission.shutdown();
    assert!(cancel.is_cancelled());
    let Reservation {
        owned,
        payload,
        completions,
    } = earlier;
    drop((payload, completions));
    assert!(ownership::retire_confirmed(owned).is_ok());
    assert!(observer.is_finished());
    assert!(predecessors.is_finished());
    let Reservation {
        owned,
        payload,
        completions,
    } = later;
    drop((payload, completions));
    assert!(ownership::retire_confirmed(owned).is_ok());
    assert_eq!(admission.capacity().available_permits(), MAX_JOBS);
}

#[test]
fn empty_stage_rollback_does_not_queue_behind_held_native_destruction() {
    use std::{sync::mpsc, thread};
    struct Held {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }
    impl Drop for Held {
        fn drop(&mut self) {
            let _ = self.entered.send(());
            let _ = self.release.recv_timeout(Duration::from_secs(5));
        }
    }
    let domain = crate::retirement::RetirementDomain::try_start(2, 0).unwrap();
    let admission = Admission::new(domain.clone());
    let (entered, entering) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let retired = domain.try_reserve().unwrap().retire(Held {
        entered,
        release: released,
    });
    let started = entering.recv_timeout(Duration::from_secs(2));
    let stage = admission.stage_drop().unwrap();
    let state = admission.0.state.lock().unwrap();
    let (sent, received) = mpsc::channel();
    let worker = thread::spawn(move || {
        let _ = sent.send(stage.publish().map(|_| ()));
    });
    let refused = received.recv_timeout(Duration::from_secs(2));
    let returned = admission.capacity().available_permits();
    let slot_returned = domain.try_reserve();
    drop(state);
    let _ = release.send(());
    worker.join().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let acknowledged =
        runtime.block_on(async { tokio::time::timeout(Duration::from_secs(2), retired).await });
    assert!(started.is_ok());
    assert!(matches!(refused, Ok(Err(error)) if error.kind == io::ErrorKind::WouldBlock));
    assert_eq!(returned, MAX_JOBS);
    assert!(
        slot_returned.is_ok(),
        "unused staging metadata must bypass the blocked native worker"
    );
    assert!(matches!(acknowledged, Ok(Ok(()))));
    drop(slot_returned);
    unpublished(&admission);
}
