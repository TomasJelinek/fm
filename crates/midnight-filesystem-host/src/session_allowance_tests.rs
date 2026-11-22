use super::*;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    task::Wake,
};

/// Counts its wake-ups, so a test sees exactly when a reservation is woken.
#[derive(Default)]
struct Wakes(AtomicUsize);
impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn waker() -> (Arc<Wakes>, Waker) {
    let wakes = Arc::new(Wakes::default());
    (wakes.clone(), Waker::from(wakes))
}
fn poll(reservation: &mut Reservation, waker: &Waker) -> Poll<Result<NativeSlot, NativeLimit>> {
    Pin::new(reservation).poll(&mut Context::from_waker(waker))
}
fn session(allowance: &Arc<NativeAllowance>, id: u64) -> NativeSlot {
    let slot = allowance.try_reserve().expect("a free slot");
    slot.bind(id, &container(id));
    slot
}
fn container(id: u64) -> PathBuf {
    std::env::temp_dir().join(format!("archive {id} 文档.7z"))
}
/// Both slots are taken by sessions 1 and 2.
fn full() -> (Arc<NativeAllowance>, NativeSlot, NativeSlot) {
    let allowance = NativeAllowance::new(2);
    let first = session(&allowance, 1);
    let second = session(&allowance, 2);
    (allowance, first, second)
}

#[test]
fn a_full_allowance_refuses_at_once_and_lists_its_sessions() {
    let (allowance, first, second) = full();
    let refusal = allowance.try_reserve().unwrap_err();
    assert_eq!(
        refusal.sessions,
        [
            NativeSession {
                id: 1,
                container: container(1)
            },
            NativeSession {
                id: 2,
                container: container(2)
            }
        ]
    );
    assert_eq!(
        refusal.to_string(),
        "Two archives are already open; close one of them first"
    );
    // Without a session to replace, a reservation is refused at once too.
    let (wakes, waker) = waker();
    assert!(matches!(
        poll(&mut allowance.reserve(None), &waker),
        Poll::Ready(Err(ref limit)) if *limit == refusal
    ));
    drop(first);
    assert!(allowance.try_reserve().is_ok(), "a released slot is free");
    drop(second);
    assert_eq!(allowance.counted(), 0);
    assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
}

#[test]
fn a_starting_session_counts_but_is_not_listed() {
    let allowance = NativeAllowance::new(2);
    let _starting = allowance.try_reserve().unwrap();
    let _open = session(&allowance, 7);
    let refusal = allowance.try_reserve().unwrap_err();
    assert_eq!(refusal.sessions.len(), 1);
    assert_eq!(refusal.sessions[0].id, 7);
}

#[test]
fn the_successor_takes_the_replaced_slot_and_a_third_open_never_does() {
    let (allowance, replaced, _other) = full();
    let (wakes, waker) = waker();
    let mut successor = allowance.reserve(Some(1));
    assert!(poll(&mut successor, &waker).is_pending());
    assert!(allowance.try_reserve().is_err());
    drop(replaced);
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1, "the hand-off wakes it");
    assert_eq!(allowance.counted(), 2, "the slot is handed off, not freed");
    assert!(
        allowance.try_reserve().is_err(),
        "a third open cannot take the handed-off slot"
    );
    let Poll::Ready(Ok(slot)) = poll(&mut successor, &waker) else {
        panic!("the successor holds the replaced slot")
    };
    slot.bind(3, &container(3));
    let listed: Vec<_> = allowance.refusal().sessions.iter().map(|s| s.id).collect();
    assert_eq!(listed, [2, 3]);
    drop(successor);
    assert_eq!(
        allowance.counted(),
        2,
        "a finished reservation releases nothing"
    );
    drop(slot);
    assert_eq!(allowance.counted(), 1);
}

#[test]
fn a_session_has_one_successor_and_an_unknown_one_none() {
    let (allowance, _first, _second) = full();
    let (_, waker) = waker();
    let mut successor = allowance.reserve(Some(1));
    assert!(poll(&mut successor, &waker).is_pending());
    assert!(
        matches!(
            poll(&mut allowance.reserve(Some(1)), &waker),
            Poll::Ready(Err(_))
        ),
        "a second successor of the same session is refused at once"
    );
    assert!(
        matches!(
            poll(&mut allowance.reserve(Some(99)), &waker),
            Poll::Ready(Err(_))
        ),
        "a session that does not count is no replacement"
    );
}

#[test]
fn a_cancelled_successor_leaves_the_replaced_slot_to_its_release() {
    let (allowance, replaced, other) = full();
    let (_, waker) = waker();
    let mut successor = allowance.reserve(Some(1));
    assert!(poll(&mut successor, &waker).is_pending());
    // Its navigation was cancelled, superseded or timed out.
    drop(successor);
    assert_eq!(allowance.counted(), 2);
    drop(replaced);
    assert_eq!(
        allowance.counted(),
        1,
        "the release frees the slot as usual"
    );
    let third = allowance.try_reserve().unwrap();
    drop((third, other));
    assert_eq!(allowance.counted(), 0);
}

#[test]
fn a_successor_dropped_after_the_hand_off_frees_the_untaken_slot() {
    let (allowance, replaced, _other) = full();
    let (_, waker) = waker();
    let mut successor = allowance.reserve(Some(1));
    assert!(poll(&mut successor, &waker).is_pending());
    drop(replaced);
    assert_eq!(allowance.counted(), 2);
    // The navigation ended before its task took the granted slot.
    drop(successor);
    assert_eq!(allowance.counted(), 1);
    assert!(allowance.try_reserve().is_ok());
}

#[test]
fn a_failed_open_returns_the_handed_off_slot() {
    let (allowance, replaced, _other) = full();
    let (_, waker) = waker();
    let mut successor = allowance.reserve(Some(1));
    assert!(poll(&mut successor, &waker).is_pending());
    drop(replaced);
    let Poll::Ready(Ok(slot)) = poll(&mut successor, &waker) else {
        panic!("handed-off slot")
    };
    // The archive could not be opened in it.
    drop(slot);
    assert_eq!(allowance.counted(), 1);
    assert!(allowance.try_reserve().is_ok());
}

#[test]
fn an_unwinding_successor_is_withdrawn() {
    let (allowance, replaced, _other) = full();
    let waiting = allowance.clone();
    let unwound = std::panic::catch_unwind(move || {
        let (_, waker) = waker();
        let mut successor = waiting.reserve(Some(1));
        assert!(poll(&mut successor, &waker).is_pending());
        panic!("navigation worker unwound while waiting");
    });
    assert!(unwound.is_err());
    drop(replaced);
    assert_eq!(allowance.counted(), 1, "no successor kept the slot");
}

#[test]
fn a_session_that_stays_held_keeps_its_slot_from_its_successor() {
    // A viewer or job still holds the replaced session: it is never released, and its
    // successor gives up (its navigation's deadline), which changes nothing.
    let (allowance, still_held, _other) = full();
    let (wakes, waker) = waker();
    let mut successor = allowance.reserve(Some(1));
    assert!(poll(&mut successor, &waker).is_pending());
    assert!(poll(&mut successor, &waker).is_pending());
    drop(successor);
    assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
    assert_eq!(allowance.counted(), 2);
    let listed: Vec<_> = allowance.refusal().sessions.iter().map(|s| s.id).collect();
    assert_eq!(listed, [1, 2], "the shared session still counts");
    drop(still_held);
}

#[test]
fn a_successor_takes_a_slot_that_frees_first_and_lets_the_replaced_one_go() {
    let (allowance, replaced, other) = full();
    let (wakes, waker) = waker();
    let mut successor = allowance.reserve(Some(1));
    assert!(poll(&mut successor, &waker).is_pending());
    drop(other);
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1, "a free slot wakes it");
    let Poll::Ready(Ok(slot)) = poll(&mut successor, &waker) else {
        panic!("the free slot")
    };
    assert_eq!(allowance.counted(), 2);
    drop(replaced);
    assert_eq!(
        allowance.counted(),
        1,
        "the replaced session no longer has a successor"
    );
    drop(slot);
}
