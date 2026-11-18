//! An open picker is checked once a second; a pending cancellation is retried every
//! 25 ms, even when a slow check was already armed, until the queue accepts it. Every
//! Options callback that invalidates an open picker wakes its check.
use super::*;
use crate::{
    EditorOptionsDraft, EditorOptionsView, UserCommandSettingsView,
    presenter::settings_picker::{OPTIONS_TOPIC_CUSTOM_COMMANDS, Target},
    test_harness::Harness,
};
use midnight_core::{PathPickerOwner, UserPreferences};
use std::cell::Cell;

fn cancellations(harness: &Harness) -> usize {
    harness
        .sent
        .borrow()
        .iter()
        .filter(|command| matches!(command, Command::CancelPathPicker { .. }))
        .count()
}

/// Invalidate the picker the production Browse callback opened, with the queue full
/// during the callback: a callback that delivers the cancellation itself is refused.
/// The woken check then delivers it at the cancellation cadence, long before the
/// pending one-second check.
fn expect_prompt_cancellation(
    harness: &mut Harness,
    name: &str,
    browse: impl FnOnce(&Harness),
    invalidate: impl FnOnce(&Harness),
) {
    browse(&*harness);
    harness.settle();
    assert!(harness.presenter.borrow().settings_picker.busy(), "{name}");
    harness.advance(Duration::from_millis(100));
    let before = cancellations(harness);
    harness.refuse.set(true);
    invalidate(&*harness);
    harness.refuse.set(false);
    assert!(
        harness
            .presenter
            .borrow()
            .settings_picker
            .cancellation_pending(),
        "{name} invalidates the picker"
    );
    assert_eq!(cancellations(harness), before, "{name}");
    harness.advance(CANCEL_RETRY);
    assert_eq!(cancellations(harness), before + 1, "{name} wakes the check");
    assert!(!harness.presenter.borrow().settings_picker.busy(), "{name}");
}

fn browse_editor(harness: &Harness) {
    harness.window.global::<EditorOptionsView>().invoke_browse();
}

fn browse_command(harness: &Harness) {
    harness
        .window
        .global::<UserCommandSettingsView>()
        .invoke_browse(0);
}

#[test]
fn editor_callbacks_that_invalidate_a_picker_deliver_its_cancellation_within_25_ms() {
    let mut harness = Harness::options(UserPreferences::default(), 4);
    harness.freeze_time();
    expect_prompt_cancellation(
        &mut harness,
        "an edited form",
        browse_editor,
        |harness: &Harness| {
            let form = EditorOptionsDraft {
                executable: "editor 編集.exe".into(),
                ..Default::default()
            };
            harness
                .window
                .global::<EditorOptionsView>()
                .invoke_edit(form);
        },
    );
    expect_prompt_cancellation(
        &mut harness,
        "a selected rule",
        browse_editor,
        |harness: &Harness| {
            harness
                .window
                .global::<EditorOptionsView>()
                .invoke_select(0)
        },
    );
    expect_prompt_cancellation(
        &mut harness,
        "an added rule",
        browse_editor,
        |harness: &Harness| {
            harness
                .window
                .global::<EditorOptionsView>()
                .invoke_action(0)
        },
    );
    expect_prompt_cancellation(
        &mut harness,
        "an added argument",
        browse_editor,
        |harness: &Harness| {
            let view = harness.window.global::<EditorOptionsView>();
            view.invoke_argument(1, 0, "--wait".into());
        },
    );
    expect_prompt_cancellation(
        &mut harness,
        "a discarded draft",
        browse_editor,
        |harness: &Harness| {
            harness
                .window
                .global::<EditorOptionsView>()
                .invoke_discard()
        },
    );
    expect_prompt_cancellation(
        &mut harness,
        "another Options topic",
        browse_editor,
        |harness: &Harness| {
            let view = harness.window.global::<UserCommandSettingsView>();
            view.invoke_picker_context(9);
        },
    );
    // Last: Cancel closes Options.
    expect_prompt_cancellation(
        &mut harness,
        "Options Cancel",
        browse_editor,
        |harness: &Harness| harness.window.invoke_options_cancelled(),
    );
}

#[test]
fn command_callbacks_that_invalidate_a_picker_deliver_its_cancellation_within_25_ms() {
    let mut harness = Harness::options(UserPreferences::default(), 9);
    harness.freeze_time();
    harness
        .window
        .global::<UserCommandSettingsView>()
        .invoke_new_command();
    harness.settle();
    expect_prompt_cancellation(
        &mut harness,
        "an edited draft",
        browse_command,
        |harness: &Harness| {
            let view = harness.window.global::<UserCommandSettingsView>();
            let mut form = view.get_draft();
            form.name = "Renamed 名前".into();
            view.invoke_draft_edited(view.get_token(), form);
        },
    );
    expect_prompt_cancellation(
        &mut harness,
        "an added argument",
        browse_command,
        |harness: &Harness| {
            let view = harness.window.global::<UserCommandSettingsView>();
            view.invoke_argument(1, 0, "--wait".into());
        },
    );
    expect_prompt_cancellation(
        &mut harness,
        "an inserted variable",
        browse_command,
        |harness: &Harness| {
            let view = harness.window.global::<UserCommandSettingsView>();
            assert!(view.invoke_insert_variable(view.get_token(), view.get_draft(), 0, 1));
        },
    );
}

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

    // Discarding the draft invalidates the picker while its 1 s check is armed. Waking
    // the check then re-arms it at the cancellation cadence, and the full queue refuses
    // the cancellation.
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

/// Open an editor picker while Custom Commands is shown, then report that page while the
/// presenter is borrowed, as a topic change can reach it. The callback leaves the page to
/// the event loop, which applies it once the borrow has ended, with the queue refusing
/// commands while `refuse` is set.
fn leave_the_picker_page_during_a_borrow(harness: &mut Harness, refuse: bool) {
    let send = sender(harness, &Rc::default());
    open_editor_picker(harness, &send);
    harness.refresh();
    assert!(harness.window.global::<EditorOptionsView>().get_busy());
    assert_eq!(check_timer(&harness.window).interval(), OPEN_CHECK);
    {
        let _held = harness.presenter.borrow();
        harness
            .window
            .global::<UserCommandSettingsView>()
            .invoke_picker_context(OPTIONS_TOPIC_CUSTOM_COMMANDS);
    }
    assert!(
        !harness
            .presenter
            .borrow()
            .settings_picker
            .cancellation_pending(),
        "a borrowed presenter is left unchanged"
    );
    harness.refuse.set(refuse);
    harness.settle();
    harness.refuse.set(false);
}

#[test]
fn a_page_change_during_a_presenter_borrow_publishes_the_closed_picker_at_once() {
    let mut harness = Harness::options(UserPreferences::default(), OPTIONS_TOPIC_CUSTOM_COMMANDS);
    harness.freeze_time();
    leave_the_picker_page_during_a_borrow(&mut harness, false);
    assert_eq!(cancellations(&harness), 1);
    assert!(!harness.presenter.borrow().settings_picker.busy());
    assert!(
        !harness.window.global::<EditorOptionsView>().get_busy(),
        "the closed picker is published with its cancellation, not at the slow check"
    );
}

#[test]
fn a_page_change_during_a_presenter_borrow_retries_a_refused_cancellation_every_25_ms() {
    let mut harness = Harness::options(UserPreferences::default(), OPTIONS_TOPIC_CUSTOM_COMMANDS);
    harness.freeze_time();
    leave_the_picker_page_during_a_borrow(&mut harness, true);
    assert!(
        harness
            .presenter
            .borrow()
            .settings_picker
            .cancellation_pending()
    );
    assert_eq!(cancellations(&harness), 0);
    harness.advance(CANCEL_RETRY);
    assert_eq!(
        cancellations(&harness),
        1,
        "the deferred change wakes the check"
    );
    assert!(!harness.presenter.borrow().settings_picker.busy());
    assert!(!harness.window.global::<EditorOptionsView>().get_busy());
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
