//! Embedded production forms do not depend on a second native-window allocation.
use super::*;

#[test]
fn embedded_filter_and_nested_help_ignore_native_child_creation_failure() {
    let h = Harness::new();
    h.refuse_window.set(true);
    h.main.invoke_open_panel_filter(0);
    let main = h.form();
    assert_eq!(value(&main), "*.txt");
    key(&main, Key::F1);
    modeless::draw(&main, 320, 200, "embedded-filter-help-compact");
    assert!(
        !h.main
            .global::<PanelFilterPresentation>()
            .get_help_token()
            .is_empty()
    );
    assert!(modeless::accessible_popup_item(&main, AccessibleRole::Button, "Close").is_some());
    assert!(h.refuse_window.get(), "no native allocation was attempted");
    assert_eq!(h.attempts.get(), 1);
    key(&main, Key::Escape);
    assert!(
        h.main
            .global::<PanelFilterPresentation>()
            .get_help_token()
            .is_empty()
    );
    key(&main, Key::Escape);
    h.filter_closed();
    assert!(h.commands.borrow().is_empty());
    assert!(h.main.get_panel_filter_view().enabled);
}

#[test]
fn embedded_hotpaths_ignore_native_child_creation_failure_and_refresh_does_not_reopen() {
    let h = Harness::new();
    h.refuse_window.set(true);
    h.main.invoke_show_bookmarks(0);
    let main = h.form();
    assert!(
        modeless::accessible_popup_item(&main, AccessibleRole::Button, "Add current").is_some()
    );
    key(&main, Key::Escape);
    for _ in 0..3 {
        refresh(&h.main, &h.state.borrow());
        slint::platform::update_timers_and_animations();
    }
    assert!(
        h.main
            .global::<HotPathsPresentation>()
            .get_token()
            .is_empty()
    );
    assert!(h.refuse_window.get());
    assert_eq!(h.attempts.get(), 1);
    assert!(h.commands.borrow().is_empty());
    assert!(h.state.borrow().bookmarks().is_empty());
}
