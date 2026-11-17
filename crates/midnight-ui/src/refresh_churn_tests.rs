//! A refresh that changes nothing must not republish presentation state:
//! dirty properties repaint every bound control, and a full refresh walks all
//! models. Software test platforms have no native icons, so every action icon
//! is absent here, the case where Slint image equality never holds.
use crate::{HotPathsPresentation, PanelFilterPresentation, test_harness::Harness};
use i_slint_core::properties::PropertyTracker;
use midnight_core::{Command, Event, PaneId, UserPreferences};
use slint::{ComponentHandle, platform::Key};
use std::{cell::Cell, pin::Pin, rc::Rc, sync::Arc, time::Duration};

/// Track the properties `read` reads; `is_dirty` then reports any later set
/// that Slint considered a change.
fn track(read: impl FnOnce()) -> Pin<Box<PropertyTracker>> {
    let tracker = Box::pin(PropertyTracker::default());
    tracker.as_ref().evaluate(read);
    tracker
}

#[test]
fn unchanged_refreshes_leave_open_tool_dialog_icons_clean() {
    let root = std::env::temp_dir().join("refresh-churn");
    let mut preferences = UserPreferences::default();
    for tabs in &mut preferences.tabs {
        tabs.paths = vec![root.clone()];
    }
    let mut h = Harness::with_location(preferences, root);
    let main = h.window.clone_strong();
    assert_eq!(
        main.get_action_icons().close.size().width,
        0,
        "absent native icons are the case these guards cover"
    );

    main.invoke_open_hot_paths_window(0);
    h.settle();
    let hot_paths = main.global::<HotPathsPresentation>();
    assert!(!hot_paths.get_token().is_empty(), "the manager is open");
    let palette = track(|| {
        hot_paths.get_action_icons();
    });
    h.refresh();
    h.refresh();
    assert!(
        !palette.is_dirty(),
        "an unchanged palette is not republished to the open Hot Paths manager"
    );
    hot_paths.invoke_dismiss(hot_paths.get_token());
    h.settle();

    h.presenter.borrow_mut().open_panel_filter(PaneId::Left);
    h.refresh();
    let filter = main.global::<PanelFilterPresentation>();
    assert!(!filter.get_token().is_empty(), "the filter is open");
    let close = track(|| {
        filter.get_close_icon();
    });
    h.refresh();
    h.refresh();
    assert!(
        !close.is_dirty(),
        "an unchanged close icon is not republished to the open Panel Filter"
    );
}

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
