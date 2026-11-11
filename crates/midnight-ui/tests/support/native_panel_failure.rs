//! Failed native creation reports through a visible owned Notice without side effects.
use super::*;
use i_slint_core::{item_tree::ItemRc, window::WindowInner};

fn visible_error(item: ItemRc) -> bool {
    if item.is_visible()
        && item.accessible_role() == AccessibleRole::Text
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|text| text.contains("injected native creation failure"))
    {
        return true;
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if visible_error(item) {
            return true;
        }
    }
    false
}

fn creation_notice(h: &Harness) -> Rc<MinimalSoftwareWindow> {
    // Normal refresh replaces Main.desktop_status, and compact Main has no status row.
    let owner = h.windows.borrow()[0].clone();
    modeless::draw(&owner, 320, 200, "panel-creation-owner-compact");
    refresh(&h.main, &h.state.borrow());
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
    }
    let notice = h.child();
    modeless::draw(&notice, 320, 200, "panel-creation-notice-compact");
    assert!(
        visible_error(ItemRc::new_root(
            WindowInner::from_pub(notice.window()).component()
        )),
        "native creation failure must remain visible in the compact owned Notice"
    );
    notice
}

fn close_notice_and_check_no_retry(h: &Harness, notice: &MinimalSoftwareWindow) {
    let attempts = h.attempts.get();
    key(notice, Key::Escape);
    assert!(!notice.window().is_visible());
    assert!(!h.main.get_dialog_input_blocked());
    for _ in 0..3 {
        refresh(&h.main, &h.state.borrow());
        slint::platform::update_timers_and_animations();
    }
    assert_eq!(
        h.attempts.get(),
        attempts,
        "refresh cannot reopen a failed host"
    );
    assert!(
        h.commands.borrow().is_empty(),
        "reporting cannot mutate backend state"
    );
    assert!(
        h.windows
            .borrow()
            .iter()
            .skip(1)
            .all(|w| !w.window().is_visible())
    );
}

#[test]
fn native_hotpaths_failed_creation_reports_visibly_and_explicit_open_retries() {
    let h = Harness::new();
    h.refuse_window.set(true);
    h.main.invoke_show_bookmarks(0);
    let notice = creation_notice(&h);
    assert!(h.state.borrow().bookmarks().is_empty());
    close_notice_and_check_no_retry(&h, &notice);
    h.main.invoke_show_bookmarks(0);
    let manager = h.child();
    assert!(modeless::accessible_item(&manager, AccessibleRole::Button, "Add current").is_some());
    assert!(h.commands.borrow().is_empty());
    assert!(h.state.borrow().bookmarks().is_empty());
    key(&manager, Key::Escape);
    assert!(!h.main.get_dialog_input_blocked());
}

#[test]
fn native_filter_failed_creation_cleans_draft_and_keeps_applied_filter() {
    let h = Harness::new();
    h.refuse_window.set(true);
    h.main.invoke_open_panel_filter(0);
    let notice = creation_notice(&h);
    assert!(
        !h.main.get_panel_filter_view().open,
        "failed draft must be retired"
    );
    assert!(h.main.get_panel_filter_view().enabled);
    assert_eq!(h.main.get_panel_filter_view().expression, "*.txt");
    close_notice_and_check_no_retry(&h, &notice);
    h.main.invoke_open_panel_filter(0);
    let filter = h.child();
    assert_eq!(value(&filter), "*.txt");
    assert!(h.commands.borrow().is_empty());
    key(&filter, Key::Escape);
    assert!(!h.main.get_dialog_input_blocked());
}
