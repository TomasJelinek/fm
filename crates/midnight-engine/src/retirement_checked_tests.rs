//! These tests catch actual-slot reuse after uncertain cleanup, not just guard counts.
use super::*;
use midnight_core::FsError;
use std::sync::atomic::AtomicUsize;
use tokio::sync::Semaphore;

#[derive(Clone, Copy)]
enum Release {
    Confirm,
    Fail,
    Panic,
    Unknown,
}
struct PayloadProbe {
    mode: Release,
    dropped: Arc<AtomicUsize>,
    callbacks: Arc<AtomicUsize>,
}
impl CheckedPayload for PayloadProbe {
    fn retirement_is_checked(&self) -> bool {
        !matches!(self.mode, Release::Unknown)
    }
    fn discard_on_worker(&mut self) -> CheckedRelease<()> {
        self.callbacks.fetch_add(1, Ordering::SeqCst);
        match self.mode {
            Release::Confirm => CheckedRelease::Confirmed(()),
            Release::Fail => CheckedRelease::Unconfirmed(failure()),
            Release::Panic => panic!("injected checked release panic"),
            Release::Unknown => panic!("unknown implementation must not be called"),
        }
    }
}
impl Drop for PayloadProbe {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
fn failure() -> FsError {
    FsError::from_io(
        "checked test",
        std::path::Path::new(""),
        io::Error::other("unconfirmed"),
    )
}

#[tokio::test]
async fn confirmed_checked_cleanup_returns_the_original_slot_once() {
    let domain = RetirementDomain::try_start(1, 0).unwrap();
    let lease = domain.try_reserve().unwrap();
    let slot = lease.slot.unwrap();
    let dropped = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    lease
        .own_checked(
            PayloadProbe {
                mode: Release::Confirm,
                dropped: dropped.clone(),
                callbacks: calls.clone(),
            },
            (),
        )
        .retire()
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(domain.control.shared.available[slot].load(Ordering::Acquire));
    assert_eq!(domain.try_reserve().unwrap().slot, Some(slot));
}

#[tokio::test]
async fn failed_panicking_and_unknown_cleanup_acknowledge_error_but_retain_actual_slot() {
    for mode in [Release::Fail, Release::Panic, Release::Unknown] {
        let domain = RetirementDomain::try_start(1, 0).unwrap();
        let lease = domain.try_reserve().unwrap();
        let slot = lease.slot.unwrap();
        let dropped = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let budget = Arc::new(Semaphore::new(1));
        let credit = budget.clone().try_acquire_owned().unwrap();
        let shared = Arc::downgrade(&domain.control.shared);
        let ack = lease
            .own_checked(
                PayloadProbe {
                    mode,
                    dropped: dropped.clone(),
                    callbacks: calls.clone(),
                },
                credit,
            )
            .retire();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(2), ack)
                .await
                .unwrap()
                .is_err()
        );
        assert!(!domain.control.shared.available[slot].load(Ordering::Acquire));
        assert!(domain.try_reserve().is_err());
        assert_eq!(budget.available_permits(), 0);
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            usize::from(!matches!(mode, Release::Unknown))
        );
        drop(domain);
        assert!(
            shared.upgrade().is_some(),
            "quarantine survives domain shutdown"
        );
    }
}

#[test]
fn callback_unwind_cannot_be_reclassified_as_confirmed_by_a_later_finalizer() {
    let domain = RetirementDomain::try_start(1, 0).unwrap();
    let lease = domain.try_reserve().unwrap();
    let slot = lease.slot.unwrap();
    let dropped = Arc::new(AtomicUsize::new(0));
    let mut owner = lease.own_checked(
        PayloadProbe {
            mode: Release::Confirm,
            dropped: dropped.clone(),
            callbacks: Arc::default(),
        },
        (),
    );
    assert!(owner.with_worker(|_| panic!("capture panic")).is_err());
    let result = owner.finish_on_worker(|_| CheckedRelease::Confirmed(()));
    assert!(matches!(result, CheckedFinish::Unconfirmed(_)));
    assert!(!domain.control.shared.available[slot].load(Ordering::Acquire));
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn ordinary_panicking_destructor_still_returns_its_slot() {
    struct Panics;
    impl Drop for Panics {
        fn drop(&mut self) {
            panic!("ordinary destructor panic");
        }
    }
    let domain = RetirementDomain::try_start(1, 0).unwrap();
    assert!(domain.try_reserve().unwrap().retire(Panics).await.is_err());
    assert!(domain.try_reserve().is_ok());
}

#[test]
fn dropping_unpolled_acknowledgement_never_cancels_checked_cleanup() {
    let domain = RetirementDomain::try_start(1, 0).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let ack = domain
        .try_reserve()
        .unwrap()
        .own_checked(
            PayloadProbe {
                mode: Release::Fail,
                dropped: dropped.clone(),
                callbacks: calls.clone(),
            },
            (),
        )
        .retire();
    drop(ack);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while calls.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(domain.try_reserve().is_err());
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
}
