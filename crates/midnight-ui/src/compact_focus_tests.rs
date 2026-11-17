//! Entering a compact dialog layout can clip the focused field, so focus moves
//! to the dismiss button once. Resizing inside the compact layout must leave
//! focus where the user put it.
use crate::test_harness::Harness;
use i_slint_core::items::AccessibleRole;
use midnight_core::{PaneId, UserPreferences};

#[test]
fn resizing_inside_the_compact_filter_keeps_focus_on_the_edited_field() {
    let mut h = Harness::new(UserPreferences::default());
    h.presenter.borrow_mut().open_panel_filter(PaneId::Left);
    h.refresh();
    let field = "File filter expression";
    assert_eq!(h.focused_label().as_deref(), Some(field));

    h.resize(380, 600);
    assert_eq!(
        h.focused_label().as_deref(),
        Some("Cancel"),
        "entering the compact layout keeps Escape reachable"
    );
    h.click(field, AccessibleRole::TextInput);
    assert_eq!(h.focused_label().as_deref(), Some(field));

    h.resize(360, 560);
    h.resize(370, 300);
    assert_eq!(
        h.focused_label().as_deref(),
        Some(field),
        "resizing inside the compact layout leaves the edited field focused"
    );
}
