//! A Thumbnails view says in its pane's status row why it shows no previews, and what the
//! preview service reports, without a dialog and without taking the keyboard
//! (THUMB-UNKNOWN-STORAGE, WIN-11).
use crate::test_harness::Harness;
use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_core::{
    DirectoryIndex, Entry, EntryId, EntryKind, Event, NavigationId, PaneId, PanelViewMode,
    StorageKind, UserPreferences,
};
use std::{path::Path, sync::Arc};

/// Accepts a listing of one image of `path` in the left pane.
fn show(h: &mut Harness, path: &Path) -> NavigationId {
    let navigation = {
        let mut presenter = h.presenter.borrow_mut();
        assert!(presenter.navigate(PaneId::Left, path.to_owned(), None, &|_| true));
        let navigation = NavigationId(presenter.panes[0].generation);
        let entries: Arc<[Entry]> = Arc::from([Entry::new(
            EntryId(1),
            path.join("tile.png"),
            EntryKind::File,
        )]);
        presenter.apply(Event::ListingFinished {
            pane: PaneId::Left,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
        navigation
    };
    h.refresh();
    navigation
}

fn classify(h: &mut Harness, navigation: NavigationId, kind: StorageKind) {
    h.presenter.borrow_mut().apply(Event::StorageKindUpdated {
        pane: PaneId::Left,
        navigation,
        kind,
    });
    h.refresh();
}

/// The left pane's rendered status row, if one is shown. Never a dialog: the main
/// window is searched only while no notice is open.
fn shown(h: &Harness) -> Option<String> {
    assert!(
        h.presenter.borrow().desktop.notice.is_none(),
        "never a dialog: {:?}",
        h.presenter.borrow().desktop.notice
    );
    let expected = h.window.get_left_watch_status();
    if expected.is_empty() {
        return None;
    }
    let item = h.find(|item| {
        item.accessible_role() == AccessibleRole::Text
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|label| label == expected)
    })?;
    item.accessible_string_property(AccessibleStringProperty::Label)
        .map(|label| label.to_string())
}

#[test]
fn storage_reasons_and_preview_failures_are_status_rows_that_leave_the_keyboard_alone() {
    let root = std::env::temp_dir().join("thumbnail-status-presentation");
    let mut preferences = UserPreferences::default();
    for tabs in &mut preferences.tabs {
        tabs.paths = vec![root.clone()];
    }
    preferences.panel_view_modes[0] = PanelViewMode::Thumbnails;
    let mut h = Harness::with_location(preferences, root.clone());
    let navigation = show(&mut h, &root);
    let focused = h.focused_label();
    assert_eq!(shown(&h), None, "a pending classification says nothing");

    classify(&mut h, navigation, StorageKind::Network);
    let text = shown(&h).expect("the withheld view says why");
    assert!(text.contains("network locations"), "{text}");
    assert!(
        text.contains("Options › Previews › Allow previews on network locations"),
        "{text}"
    );
    assert!(h.window.get_right_watch_status().is_empty());
    assert!(h.window.get_left_error().is_empty(), "never a panel error");
    assert_eq!(h.focused_label(), focused, "the keyboard stays in place");

    // A local disk of unknown media type shows previews and says nothing.
    classify(&mut h, navigation, StorageKind::Local);
    assert_eq!(shown(&h), None);

    let failure = "Could not store or read preview cache: Captured native content changed.";
    h.presenter
        .borrow_mut()
        .report_thumbnail_error(failure.into());
    h.refresh();
    assert_eq!(shown(&h).as_deref(), Some(failure));
    assert_eq!(h.window.get_message_generation(), 0, "no notice window");
    assert_eq!(h.focused_label(), focused, "the keyboard stays in place");
}
