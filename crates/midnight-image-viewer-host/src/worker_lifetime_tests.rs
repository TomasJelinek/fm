use super::*;
use std::{
    mem::ManuallyDrop,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

type Events = Arc<Mutex<Vec<&'static str>>>;

fn is_unconfirmed(error: &io::Error) -> bool {
    matches!(
        cleanup_status(error),
        Some(CleanupStatus::ProcessUnconfirmed | CleanupStatus::PinUnconfirmed)
    )
}

#[derive(Clone, Copy)]
enum Failure {
    None,
    Error,
    Panic,
}

struct Payload(Events);
impl Drop for Payload {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("payload-drop");
    }
}

struct Transport {
    events: Events,
    failure: Failure,
    gate: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
}
impl Shutdown for Transport {
    fn shutdown(&mut self) -> io::Result<()> {
        self.events.lock().unwrap().push("wait");
        if let Some((entered, resume)) = &self.gate {
            entered.send(()).unwrap();
            resume.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        match self.failure {
            Failure::None => Ok(()),
            Failure::Error => Err(io::Error::other("termination is unconfirmed")),
            Failure::Panic => std::panic::panic_any(Payload(self.events.clone())),
        }
    }
}
impl Drop for Transport {
    fn drop(&mut self) {
        self.events.lock().unwrap().push("transport-drop");
    }
}

struct Identity {
    events: Events,
    failure: Failure,
}
impl Release for Identity {
    fn release_on_worker(self) -> io::Result<()> {
        // Match the pin contract: a failed consuming release retains its owner.
        let owner = ManuallyDrop::new(self);
        owner.events.lock().unwrap().push("pin-release");
        match owner.failure {
            Failure::None => {
                drop(ManuallyDrop::into_inner(owner));
                Ok(())
            }
            Failure::Error => Err(io::Error::other("pin close is unconfirmed")),
            Failure::Panic => std::panic::panic_any(Payload(owner.events.clone())),
        }
    }
}
impl Drop for Identity {
    fn drop(&mut self) {
        self.events.lock().unwrap().push("pin-drop");
    }
}

struct Source(Events);
impl Drop for Source {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("source-drop");
    }
}

fn fixture(
    wait_failure: Failure,
    close_failure: Failure,
) -> (Owner<Transport, Identity, Source>, Events) {
    let events = Events::default();
    let mut owner = Owner::new(Identity {
        events: events.clone(),
        failure: close_failure,
    })
    .with_source(Source(events.clone()));
    owner
        .start(|| {
            Ok(Transport {
                events: events.clone(),
                failure: wait_failure,
                gate: None,
            })
        })
        .unwrap();
    (owner, events)
}

#[test]
fn confirmed_wait_and_transport_drop_precede_pin_release_and_source_drop() {
    let (mut owner, events) = fixture(Failure::None, Failure::None);
    assert!(events.lock().unwrap().is_empty());
    owner.finish().unwrap();
    owner.finish().unwrap();
    drop(owner);
    assert_eq!(
        *events.lock().unwrap(),
        [
            "wait",
            "transport-drop",
            "source-drop",
            "pin-release",
            "pin-drop"
        ]
    );
}

#[test]
fn prepared_to_session_transfer_keeps_the_same_pin_until_session_retirement() {
    let events = Events::default();
    let mut prepared = Owner::new(Identity {
        events: events.clone(),
        failure: Failure::None,
    });
    prepared
        .start(|| {
            Ok(Transport {
                events: events.clone(),
                failure: Failure::None,
                gate: None,
            })
        })
        .unwrap();
    let mut session = prepared.with_source(Source(events.clone()));
    assert!(events.lock().unwrap().is_empty());
    session.finish().unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        [
            "wait",
            "transport-drop",
            "source-drop",
            "pin-release",
            "pin-drop"
        ]
    );
}

#[test]
fn blocked_native_wait_retains_pin_and_source_until_actual_confirmation() {
    let (mut owner, events) = fixture(Failure::None, Failure::None);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    owner.transport_mut().gate = Some((entered_tx, resume_rx));
    let worker = std::thread::spawn(move || owner.finish());
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(*events.lock().unwrap(), ["wait"]);
    resume_tx.send(()).unwrap();
    worker.join().unwrap().unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        [
            "wait",
            "transport-drop",
            "source-drop",
            "pin-release",
            "pin-drop"
        ]
    );
}

#[test]
fn unknown_termination_is_sticky_and_retains_every_native_and_source_owner() {
    let (mut owner, events) = fixture(Failure::Error, Failure::None);
    assert!(is_unconfirmed(&owner.finish().unwrap_err()));
    assert!(is_unconfirmed(&owner.finish().unwrap_err()));
    drop(owner);
    assert_eq!(*events.lock().unwrap(), ["wait"]);
}

#[test]
fn panicking_wait_retains_owners_and_does_not_drop_the_opaque_payload() {
    let (mut owner, events) = fixture(Failure::Panic, Failure::None);
    assert!(is_unconfirmed(&owner.finish().unwrap_err()));
    drop(owner);
    assert_eq!(*events.lock().unwrap(), ["wait"]);
}

#[test]
fn failed_consuming_pin_close_drops_source_without_a_second_close_attempt() {
    let (mut owner, events) = fixture(Failure::None, Failure::Error);
    assert!(is_unconfirmed(&owner.finish().unwrap_err()));
    assert!(is_unconfirmed(&owner.finish().unwrap_err()));
    drop(owner);
    assert_eq!(
        *events.lock().unwrap(),
        ["wait", "transport-drop", "source-drop", "pin-release"]
    );
}

#[test]
fn panicking_pin_close_drops_source_and_retains_opaque_payload_without_retry() {
    let (mut owner, events) = fixture(Failure::None, Failure::Panic);
    assert!(is_unconfirmed(&owner.finish().unwrap_err()));
    drop(owner);
    assert_eq!(
        *events.lock().unwrap(),
        ["wait", "transport-drop", "source-drop", "pin-release"]
    );
}

#[test]
fn opaque_spawn_failure_retains_pin_and_source_without_assuming_no_child_exists() {
    let events = Events::default();
    let mut owner = Owner::<Transport, _, _>::new(Identity {
        events: events.clone(),
        failure: Failure::None,
    })
    .with_source(Source(events.clone()));
    assert!(is_unconfirmed(
        &owner
            .start(|| Err(io::Error::other("late spawn failure")))
            .unwrap_err()
    ));
    drop(owner);
    assert!(events.lock().unwrap().is_empty());
}

#[test]
fn constructor_failure_before_spawn_can_release_pin_and_source_in_order() {
    let events = Events::default();
    let mut owner = Owner::<Transport, _, _>::new(Identity {
        events: events.clone(),
        failure: Failure::None,
    })
    .with_source(Source(events.clone()));
    owner.finish().unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        ["source-drop", "pin-release", "pin-drop"]
    );
}

struct Reservation(Events);
impl Drop for Reservation {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("capacity-release");
    }
}

#[test]
fn shared_capacity_owner_survives_the_caller_after_unknown_termination() {
    let (mut owner, events) = fixture(Failure::Error, Failure::None);
    let caller = Arc::new(Reservation(events.clone()));
    owner.retain(Box::new(caller.clone()));
    drop(caller);
    assert_eq!(
        cleanup_status(&owner.finish().unwrap_err()),
        Some(CleanupStatus::ProcessUnconfirmed)
    );
    drop(owner);
    assert_eq!(*events.lock().unwrap(), ["wait"]);
}

#[test]
fn pin_only_failure_frees_child_capacity_after_actual_termination() {
    let (mut owner, events) = fixture(Failure::None, Failure::Error);
    let caller = Arc::new(Reservation(events.clone()));
    owner.retain(Box::new(caller.clone()));
    drop(caller);
    assert_eq!(
        cleanup_status(&owner.finish().unwrap_err()),
        Some(CleanupStatus::PinUnconfirmed)
    );
    drop(owner);
    assert_eq!(
        *events.lock().unwrap(),
        [
            "wait",
            "transport-drop",
            "source-drop",
            "capacity-release",
            "pin-release"
        ]
    );
}

#[test]
fn pre_spawn_error_preserves_kind_and_marks_confirmed_cleanup() {
    let events = Events::default();
    let mut owner = Owner::<Transport, _, _>::new(Identity {
        events: events.clone(),
        failure: Failure::None,
    });
    let error = owner.fail(io::ErrorKind::Interrupted.into());
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(cleanup_status(&error), Some(CleanupStatus::Confirmed));
    assert_eq!(*events.lock().unwrap(), ["pin-release", "pin-drop"]);
}

#[test]
fn failed_pin_constructor_marker_survives_confirmed_no_process_cleanup() {
    let events = Events::default();
    let caller = Arc::new(Reservation(events.clone()));
    let mut owner = Owner::<Transport, Identity, ()>::empty(Some(Box::new(caller.clone())));
    drop(caller);
    let error = owner.fail(classified(
        CleanupStatus::PinUnconfirmed,
        io::ErrorKind::PermissionDenied.into(),
    ));
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(cleanup_status(&error), Some(CleanupStatus::PinUnconfirmed));
    assert_eq!(*events.lock().unwrap(), ["capacity-release"]);
}
