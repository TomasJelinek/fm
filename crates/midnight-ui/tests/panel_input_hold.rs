//! A panel that waits for a file job's change holds only its own input, keeps
//! the keys typed into it and replays them in order, with their modifiers, once
//! it shows the change, its re-read's wait expires or Escape ends the wait.
use crate::support::ui::{self, TestUi};
use slint::platform::Key;
use std::{cell::RefCell, rc::Rc, time::Duration};

/// What the file-list keys of each panel did: `(pane, action)`.
#[derive(Default)]
struct Delivered {
    keys: Vec<(i32, i32)>,
    bulk: Vec<(i32, i32)>,
    hot_paths: Vec<(i32, i32)>,
}

fn observe(ui: &TestUi) -> Rc<RefCell<Delivered>> {
    let delivered = Rc::new(RefCell::new(Delivered::default()));
    let sink = delivered.clone();
    ui.window
        .on_key(move |pane, key| sink.borrow_mut().keys.push((pane, key)));
    let sink = delivered.clone();
    ui.window
        .on_bulk_selection(move |pane, operation| sink.borrow_mut().bulk.push((pane, operation)));
    let sink = delivered.clone();
    ui.window
        .on_bookmark_jump(move |pane, slot| sink.borrow_mut().hot_paths.push((pane, slot)));
    delivered
}

fn focus_files(ui: &mut TestUi, pane: i32) {
    ui.window.set_active_pane(pane);
    ui.window.invoke_focus_files();
    ui.draw();
    assert_eq!(ui.window.get_focused_pane(), pane);
}

/// No wait.
const FREE: i32 = 0;
/// A confirmed Rename or creation in the panel's folder runs.
const JOB: i32 = 1;
/// The completed job's re-read is in flight.
const REREAD: i32 = 2;

fn hold_left(ui: &mut TestUi, hold: i32) {
    ui.window.set_left_mutation_hold(hold);
    ui.draw();
}

const DOWN: i32 = 3;
/// "selection.all", Ctrl+A in the file list.
const SELECT_ALL: i32 = 0;

#[test]
fn a_held_panel_replays_its_keys_in_order_with_their_modifiers_once_shown() {
    let mut ui = TestUi::new();
    let delivered = observe(&ui);
    focus_files(&mut ui, 0);
    hold_left(&mut ui, REREAD);
    assert!(ui.window.get_left_input_held());
    assert!(!ui.window.get_right_input_held());
    ui::key(&ui.window, Key::DownArrow);
    // Ctrl is released before the hold ends; the key keeps it.
    ui::chord(&ui.window, &[Key::Control], "a");
    ui::key(&ui.window, Key::DownArrow);
    assert!(
        delivered.borrow().keys.is_empty(),
        "no key acts on stale rows"
    );
    assert!(delivered.borrow().bulk.is_empty());
    assert!(ui.window.get_mutation_input_blocked());

    hold_left(&mut ui, FREE);
    assert_eq!(
        delivered.borrow().keys,
        [(0, DOWN), (0, DOWN)],
        "replayed in order"
    );
    assert_eq!(
        delivered.borrow().bulk,
        [(0, SELECT_ALL)],
        "Ctrl+A keeps its Ctrl rather than typing an a"
    );
    assert!(!ui.window.get_mutation_input_blocked());
    // Slint's modifier state again matches the keyboard, where no key is held.
    assert_eq!(ui.window.get_function_modifiers(), 0);
    ui::key(&ui.window, Key::DownArrow);
    assert_eq!(
        delivered.borrow().keys.len(),
        3,
        "a plain Down acts at once"
    );
}

#[test]
fn the_other_panel_and_the_command_line_stay_usable_during_a_hold() {
    let mut ui = TestUi::new();
    let delivered = observe(&ui);
    hold_left(&mut ui, JOB);
    focus_files(&mut ui, 1);
    ui::key(&ui.window, Key::DownArrow);
    assert_eq!(
        delivered.borrow().keys,
        [(1, DOWN)],
        "the right panel moves"
    );
    ui.window.invoke_focus_command();
    ui.draw();
    ui::key(&ui.window, "x");
    assert_eq!(ui.window.get_command_text().as_str(), "x");
    assert!(
        !ui.window.get_held_keys_pending(),
        "nothing waits for the held panel"
    );
    hold_left(&mut ui, FREE);
    assert_eq!(delivered.borrow().keys, [(1, DOWN)]);
}

#[test]
fn keys_typed_into_a_panel_the_user_left_are_not_replayed_elsewhere() {
    let mut ui = TestUi::new();
    let delivered = observe(&ui);
    focus_files(&mut ui, 0);
    hold_left(&mut ui, REREAD);
    ui::key(&ui.window, Key::DownArrow);
    // A click on the command line moves the focus away from the held panel.
    ui.window.invoke_focus_command();
    ui.draw();
    hold_left(&mut ui, FREE);
    assert!(delivered.borrow().keys.is_empty());
    assert!(!ui.window.get_mutation_input_blocked());
}

#[test]
fn a_reread_that_never_shows_the_change_releases_after_three_seconds_and_says_so() {
    let (mut ui, clock) = TestUi::with_virtual_clock();
    let delivered = observe(&ui);
    focus_files(&mut ui, 0);
    hold_left(&mut ui, REREAD);
    ui::key(&ui.window, Key::DownArrow);
    ui::advance(&clock, Duration::from_millis(2_900));
    assert!(ui.window.get_left_input_held());
    assert!(ui.window.get_left_hold_status().is_empty());
    assert!(delivered.borrow().keys.is_empty());
    ui::advance(&clock, Duration::from_millis(200));
    assert!(!ui.window.get_left_input_held(), "the hold expired");
    assert_eq!(delivered.borrow().keys, [(0, DOWN)]);
    assert!(!ui.window.get_mutation_input_blocked());
    // The panel says it is still updating until the change appears.
    assert_eq!(
        ui.window.get_left_hold_status().as_str(),
        "Updating the file list…"
    );
    ui::key(&ui.window, Key::DownArrow);
    assert_eq!(delivered.borrow().keys.len(), 2, "later keys act at once");
    hold_left(&mut ui, FREE);
    assert!(ui.window.get_left_hold_status().is_empty());
}

#[test]
fn a_slow_job_holds_its_panel_until_it_completes_and_says_what_it_does() {
    let (mut ui, clock) = TestUi::with_virtual_clock();
    let delivered = observe(&ui);
    focus_files(&mut ui, 0);
    let job = "Renaming “a.txt” to “b.txt”…";
    ui.window.set_mutation_job_text(job.into());
    hold_left(&mut ui, JOB);
    ui::key(&ui.window, Key::DownArrow);
    // A quick job needs no explanation; a slower one explains the wait cursor.
    ui::advance(&clock, Duration::from_millis(400));
    assert!(ui.window.get_left_hold_status().is_empty());
    ui::advance(&clock, Duration::from_millis(200));
    assert_eq!(ui.window.get_left_hold_status().as_str(), job);
    assert!(ui.window.get_right_hold_status().is_empty());
    // A job that runs 5 s keeps the panel waiting all along.
    ui::advance(&clock, Duration::from_millis(4_400));
    assert!(ui.window.get_left_input_held());
    assert!(delivered.borrow().keys.is_empty());
    // Its re-read's wait is bounded from the completion, not the confirmation.
    hold_left(&mut ui, REREAD);
    assert!(ui.window.get_left_hold_status().is_empty());
    ui::advance(&clock, Duration::from_millis(2_900));
    assert!(ui.window.get_left_input_held());
    assert!(delivered.borrow().keys.is_empty());
    ui::advance(&clock, Duration::from_millis(200));
    assert!(!ui.window.get_left_input_held());
    assert_eq!(delivered.borrow().keys, [(0, DOWN)]);
    // The change appears.
    hold_left(&mut ui, FREE);
    assert!(ui.window.get_left_hold_status().is_empty());
    assert!(!ui.window.get_mutation_input_blocked());
}

#[test]
fn escape_ends_a_wait_and_replays_the_keys_typed_before_it() {
    let (mut ui, clock) = TestUi::with_virtual_clock();
    let delivered = observe(&ui);
    focus_files(&mut ui, 0);
    ui.window
        .set_mutation_job_text("Creating “new.txt”…".into());
    hold_left(&mut ui, JOB);
    ui::key(&ui.window, Key::DownArrow);
    ui::advance(&clock, Duration::from_secs(1));
    assert!(ui.window.get_left_input_held());
    ui::key(&ui.window, Key::Escape);
    assert!(!ui.window.get_left_input_held(), "Escape ended the wait");
    ui.draw();
    assert_eq!(
        delivered.borrow().keys,
        [(0, DOWN)],
        "Escape itself is not replayed"
    );
    assert!(!ui.window.get_mutation_input_blocked());
    ui::key(&ui.window, Key::DownArrow);
    assert_eq!(delivered.borrow().keys.len(), 2, "keys act at once");
    // The completed job's re-read does not hold the panel again.
    hold_left(&mut ui, REREAD);
    assert!(!ui.window.get_left_input_held());
    // A new job's wait holds again.
    hold_left(&mut ui, FREE);
    hold_left(&mut ui, JOB);
    assert!(ui.window.get_left_input_held());
}

/// A failed or cancelled job has no change to show: the presenter ends its
/// panel's wait with the job, and the panel takes its keys at once.
#[test]
fn a_failed_job_releases_its_panel_at_once_without_a_status() {
    let (mut ui, clock) = TestUi::with_virtual_clock();
    let delivered = observe(&ui);
    focus_files(&mut ui, 0);
    ui.window
        .set_mutation_job_text("Creating folder “new”…".into());
    hold_left(&mut ui, JOB);
    ui::key(&ui.window, Key::DownArrow);
    ui::advance(&clock, Duration::from_secs(1));
    assert!(!ui.window.get_left_hold_status().is_empty());
    hold_left(&mut ui, FREE);
    assert!(!ui.window.get_left_input_held());
    assert_eq!(delivered.borrow().keys, [(0, DOWN)]);
    assert!(ui.window.get_left_hold_status().is_empty());
    assert!(!ui.window.get_mutation_input_blocked());
}

#[test]
fn a_native_hook_shortcut_waits_with_the_held_panels_keys() {
    let mut ui = TestUi::new();
    let delivered = observe(&ui);
    focus_files(&mut ui, 0);
    hold_left(&mut ui, JOB);
    ui::key(&ui.window, Key::DownArrow);
    // Ctrl+1 (Hot Path 1) arrives from the native window hook, not from Slint.
    assert!(
        ui.window
            .invoke_physical_shortcut("Digit1".into(), true, false, false, false),
        "taken, not left to the logical key"
    );
    assert!(delivered.borrow().hot_paths.is_empty());
    hold_left(&mut ui, FREE);
    let delivered = delivered.borrow();
    assert_eq!(delivered.keys, [(0, DOWN)]);
    assert_eq!(delivered.hot_paths, [(0, 1)], "after the earlier key");
}

/// The native hook's number-row key, as it reaches the window: the shortcut with
/// the text the key types, then the hook's mark of a number-row key.
fn native_digit(ui: &TestUi, digit: u8) -> bool {
    ui.window.set_native_key_text(digit.to_string().into());
    let taken = ui.window.invoke_physical_shortcut(
        format!("Digit{digit}").into(),
        false,
        false,
        false,
        false,
    );
    ui.window.set_native_digit_event(true);
    taken
}

/// "re2" typed right after F2: the native hook takes the 2 (Hot Path 2 in the
/// file list) while "r" and "e" wait. Replayed, they start a quick search, where
/// a bare digit is no shortcut: the 2 types instead of being lost.
#[test]
fn a_held_number_row_key_types_its_text_once_earlier_keys_start_a_search() {
    let mut ui = TestUi::new();
    let delivered = observe(&ui);
    focus_files(&mut ui, 0);
    hold_left(&mut ui, JOB);
    ui::key(&ui.window, "r");
    ui::key(&ui.window, "e");
    assert!(native_digit(&ui, 2), "taken while the panel waits");
    assert_eq!(ui.window.get_left_search_query().as_str(), "");
    hold_left(&mut ui, FREE);
    assert_eq!(ui.window.get_left_search_query().as_str(), "re2");
    assert!(delivered.borrow().hot_paths.is_empty(), "no Hot Path opens");
}

/// The hook's mark describes its latest native key: keys replayed after it are
/// not number-row keys and keep their shortcuts.
#[test]
fn replayed_keys_do_not_take_the_hooks_latest_number_row_mark() {
    let mut ui = TestUi::new();
    let delivered = observe(&ui);
    focus_files(&mut ui, 0);
    hold_left(&mut ui, JOB);
    ui::key(&ui.window, Key::DownArrow);
    // The last native key, typed after Down, came from the number row.
    assert!(native_digit(&ui, 5));
    hold_left(&mut ui, FREE);
    let delivered = delivered.borrow();
    assert_eq!(delivered.keys, [(0, DOWN)], "Down keeps its shortcut");
    assert_eq!(delivered.hot_paths, [(0, 5)], "the bound digit still opens");
    assert!(ui.window.get_native_digit_event(), "the live mark is kept");
}
