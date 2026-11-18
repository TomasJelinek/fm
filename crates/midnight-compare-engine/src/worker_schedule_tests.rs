use super::*;
use std::sync::mpsc;

#[test]
fn progressing_identity_uses_nonblocking_input_poll_between_bounded_turns() {
    assert_eq!(
        InputPoll::after_turn(IdentityTurn::Advanced, true),
        InputPoll::NonBlocking
    );
    let (sender, receiver) = mpsc::sync_channel(1);
    sender.send("input before the next identity turn").unwrap();
    let poll = InputPoll::after_turn(IdentityTurn::Advanced, true);
    assert_eq!(
        poll.receive(&receiver).unwrap(),
        "input before the next identity turn"
    );
    assert!(matches!(
        poll.receive(&receiver),
        Err(RecvTimeoutError::Timeout)
    ));
    drop(sender);
    assert!(matches!(
        poll.receive(&receiver),
        Err(RecvTimeoutError::Disconnected)
    ));
}

#[test]
fn blocked_identity_selects_a_nonzero_capped_wait_instead_of_hot_spinning() {
    assert!(matches!(
        InputPoll::after_turn(IdentityTurn::Blocked, true),
        InputPoll::Wait(delay) if !delay.is_zero() && delay <= Duration::from_millis(10)
    ));
}

#[test]
fn unconsumed_frames_and_completed_identity_wait_for_a_signal() {
    for turn in [
        IdentityTurn::Advanced,
        IdentityTurn::Blocked,
        IdentityTurn::Complete(IdentityVerdict::IdenticalFiles),
    ] {
        assert_eq!(InputPoll::after_turn(turn, false), InputPoll::Block);
    }
    assert_eq!(
        InputPoll::after_turn(IdentityTurn::Complete(IdentityVerdict::Different), true),
        InputPoll::Block
    );
}
