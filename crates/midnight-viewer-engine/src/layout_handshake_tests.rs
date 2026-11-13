use super::*;

#[test]
fn fixed_viewport_sessions_never_wait_for_layout() {
    let mut handshake = Handshake::new(false);
    assert!(handshake.select(6, 3).unwrap().is_none());
    assert!(!handshake.waiting());
    assert!(handshake.accept(1).is_none());
}

#[test]
fn selection_carries_the_frame_generation_and_requires_the_exact_token() {
    let mut handshake = Handshake::new(true);
    let selected = handshake.select(6, 41).unwrap().unwrap();
    assert_ne!(selected.token, 0);
    assert_eq!(selected.mode, 6);
    assert_eq!(selected.content_generation, 41);
    assert!(handshake.accept(0).is_none());
    assert!(handshake.accept(selected.token + 1).is_none());
    assert_eq!(handshake.selection(), Some(selected));
    assert_eq!(handshake.accept(selected.token), Some(selected));
    assert!(!handshake.waiting());
    assert!(handshake.accept(selected.token).is_none());
}

#[test]
fn same_mode_and_file_replacement_still_receive_a_fresh_token() {
    let mut handshake = Handshake::new(true);
    let old = handshake.select(6, 10).unwrap().unwrap();
    handshake.invalidate();
    let current = handshake.select(6, 11).unwrap().unwrap();
    assert!(current.token > old.token);
    assert!(handshake.accept(old.token).is_none());
    assert_eq!(handshake.selection(), Some(current));
    assert_eq!(handshake.accept(current.token), Some(current));
}

#[test]
fn fallback_replaces_the_pending_mode_and_rejects_its_predecessor() {
    let mut handshake = Handshake::new(true);
    let html = handshake.select(6, 10).unwrap().unwrap();
    let text = handshake.select(1, 11).unwrap().unwrap();
    assert!(handshake.accept(html.token).is_none());
    assert_eq!(handshake.accept(text.token).unwrap().mode, 1);
}

#[test]
fn closing_is_terminal_even_for_a_later_same_batch_acknowledgement() {
    let mut handshake = Handshake::new(true);
    let selected = handshake.select(6, 9).unwrap().unwrap();
    handshake.close();
    assert!(handshake.closed());
    assert!(!handshake.waiting());
    assert!(handshake.accept(selected.token).is_none());
    assert!(handshake.select(1, 10).is_err());
}

#[test]
fn token_exhaustion_fails_without_reusing_old_authority() {
    let mut handshake = Handshake::new(true);
    let old = handshake.select(6, 9).unwrap().unwrap();
    handshake.next_token = u64::MAX;
    assert!(handshake.select(1, 10).is_err());
    assert!(!handshake.waiting());
    assert!(handshake.accept(old.token).is_none());
}
