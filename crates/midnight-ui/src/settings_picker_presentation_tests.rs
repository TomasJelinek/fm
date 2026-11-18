//! An open picker is checked once a second; a pending cancellation is retried every
//! 25 ms, even when a slow check was already armed, until the queue accepts it.
use super::*;
use crate::{EditorOptionsView, presenter::settings_picker::Target, test_harness::Harness};
use midnight_core::{PathPickerOwner, UserPreferences};
use std::cell::Cell;

/// The harness queue, counting every cancellation attempt including refused ones.
fn sender(harness: &Harness, attempts: &Rc<Cell<usize>>) -> Rc<dyn Fn(Command) -> bool> {
    let (sent, refuse, attempts) = (
        harness.sent.clone(),
        harness.refuse.clone(),
        attempts.clone(),
    );
    Rc::new(move |command| {
        if matches!(command, Command::CancelPathPicker { .. }) {
            attempts.set(attempts.get() + 1);
        }
        if refuse.get() {
            return false;
        }
        sent.borrow_mut().push(command);
        true
    })
}

fn open_editor_picker(harness: &Harness, send: &Rc<dyn Fn(Command) -> bool>) {
    harness.presenter.borrow_mut().request_settings_picker(
        Target::Editor,
        PathPickerOwner::None,
        send.as_ref(),
    );
    assert!(harness.presenter.borrow().settings_picker.busy());
    wake(&harness.window, &harness.presenter, send.clone());
}

#[test]
fn an_open_picker_is_checked_once_a_second_without_cancelling() {
    let mut harness = Harness::options(UserPreferences::default(), 4);
    harness.freeze_time();
    // The production Browse callback arms the slow check.
    harness.window.global::<EditorOptionsView>().invoke_browse();
    assert!(harness.presenter.borrow().settings_picker.busy());
    let timer = check_timer(&harness.window);
    assert!(timer.running());
    assert_eq!(timer.interval(), OPEN_CHECK);
    for _ in 0..10 {
        harness.advance(OPEN_CHECK);
        assert!(timer.running());
        assert_eq!(timer.interval(), OPEN_CHECK, "no 25 ms polling while open");
    }
    assert!(
        !harness
            .sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::CancelPathPicker { .. }))
    );
}

#[test]
fn an_invalidated_picker_preempts_the_slow_check_and_retries_every_25_ms() {
    let mut harness = Harness::options(UserPreferences::default(), 4);
    harness.freeze_time();
    let attempts = Rc::new(Cell::new(0));
    let send = sender(&harness, &attempts);
    open_editor_picker(&harness, &send);
    let timer = check_timer(&harness.window);
    assert_eq!(timer.interval(), OPEN_CHECK);
    harness.advance(Duration::from_millis(100));

    // Editing the draft invalidates the picker while its 1 s check is armed; the
    // invalidating callback wakes the check, and the full queue refuses the cancellation.
    harness.refuse.set(true);
    harness.presenter.borrow_mut().discard_editor_draft();
    assert!(
        harness
            .presenter
            .borrow()
            .settings_picker
            .cancellation_pending()
    );
    wake(&harness.window, &harness.presenter, send.clone());
    assert_eq!(timer.interval(), CANCEL_RETRY);
    for attempt in 1..=3 {
        harness.advance(CANCEL_RETRY);
        assert_eq!(attempts.get(), attempt, "one retry per 25 ms");
    }
    harness.refuse.set(false);
    harness.advance(CANCEL_RETRY);
    assert_eq!(attempts.get(), 4);
    assert!(
        harness
            .sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::CancelPathPicker { .. }))
    );
    assert!(!harness.presenter.borrow().settings_picker.busy());
    assert!(!timer.running(), "a closed picker needs no check");
    harness.advance(OPEN_CHECK * 5);
    assert_eq!(attempts.get(), 4);
}

#[test]
fn a_borrowed_presenter_retries_the_check_at_the_cancellation_cadence() {
    let mut harness = Harness::options(UserPreferences::default(), 4);
    harness.freeze_time();
    let attempts = Rc::new(Cell::new(0));
    let send = sender(&harness, &attempts);
    open_editor_picker(&harness, &send);
    harness.presenter.borrow_mut().discard_editor_draft();
    let borrowed = harness.presenter.clone();
    {
        let _held = borrowed.borrow_mut();
        wake(&harness.window, &harness.presenter, send.clone());
        let timer = check_timer(&harness.window);
        assert_eq!(timer.interval(), CANCEL_RETRY);
    }
    harness.advance(CANCEL_RETRY);
    assert_eq!(attempts.get(), 1);
    assert!(!harness.presenter.borrow().settings_picker.busy());
}
