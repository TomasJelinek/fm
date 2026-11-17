//! A refresh that changes nothing must not republish presentation state:
//! dirty properties repaint every bound control, and a full refresh walks all
//! models.
use crate::test_harness::Harness;
use midnight_core::{Command, Event, UserPreferences};
use slint::platform::Key;
use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};

#[test]
fn a_path_paste_waiting_for_the_clipboard_refreshes_only_when_it_inserts() {
    let mut h = Harness::new(UserPreferences::default());
    let refreshes = Rc::new(Cell::new(0));
    let count = refreshes.clone();
    // Only the application host installs this hook; every full refresh calls it.
    h.window
        .on_refresh_warning_viewer(move || count.set(count.get() + 1));
    h.window.invoke_focus_files();
    h.chord(&[Key::Control], "g");
    assert_eq!(h.focused_label().as_deref(), Some("Folder path"));
    h.freeze_time();
    h.sent.borrow_mut().clear();
    h.chord(&[Key::Control], "v");
    let request = h
        .sent
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::ReadPathClipboardText { request } => Some(*request),
            _ => None,
        })
        .expect("Paste asks the backend for clipboard text");
    let waiting = refreshes.get();
    for _ in 0..8 {
        h.advance(Duration::from_millis(25));
    }
    assert_eq!(
        refreshes.get(),
        waiting,
        "ticks that are still waiting for the reply change nothing"
    );
    h.presenter
        .borrow_mut()
        .apply(Event::PathClipboardTextRead {
            request,
            result: Ok(Arc::from("pasted")),
        });
    h.advance(Duration::from_millis(25));
    assert_eq!(
        refreshes.get(),
        waiting + 1,
        "the tick that inserts the reply refreshes once"
    );
    assert!(
        h.window.get_left_path_draft().contains("pasted"),
        "{}",
        h.window.get_left_path_draft()
    );
}
