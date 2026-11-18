//! A pane shows the watch status of its current view in the window, without a
//! panel error or a dialog, and clears it on `Native` or another view.
use crate::test_harness::Harness;
use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_core::{
    DirectoryIndex, Event, FsError, NavigationId, PaneId, UserPreferences, WatchStatus, WatchedView,
};
use std::{io, path::Path, sync::Arc};

/// Accepts an empty listing of `path` in the left pane.
fn show(h: &mut Harness, path: &Path) -> NavigationId {
    let navigation = {
        let mut presenter = h.presenter.borrow_mut();
        assert!(presenter.navigate(PaneId::Left, path.to_owned(), None, &|_| true));
        let navigation = NavigationId(presenter.panes[0].generation);
        presenter.apply(Event::ListingFinished {
            pane: PaneId::Left,
            navigation,
            entries: Arc::from([]),
            index: Arc::new(DirectoryIndex::from_entries(&[])),
        });
        navigation
    };
    h.refresh();
    navigation
}

fn report(h: &mut Harness, navigation: NavigationId, status: WatchStatus) {
    h.presenter.borrow_mut().apply(Event::WatchStatus {
        view: WatchedView::Flat {
            pane: PaneId::Left,
            navigation,
        },
        status,
    });
    h.refresh();
}

fn polling_only(path: &Path) -> WatchStatus {
    WatchStatus::PollingOnly(FsError::from_io(
        "watch directory",
        path,
        io::Error::other("OS file watch limit reached"),
    ))
}

/// The rendered status: the label and description of the window's watch
/// status element, if one is shown. Never a dialog: the main window is searched
/// only while no dialog is open.
fn shown(h: &Harness) -> Option<(String, String)> {
    assert!(
        h.presenter.borrow().desktop.notice.is_none(),
        "never a dialog: {:?}",
        h.presenter.borrow().desktop.notice
    );
    let property = |item: &i_slint_core::item_tree::ItemRc, which| {
        item.accessible_string_property(which)
            .unwrap_or_default()
            .to_string()
    };
    let item = h.find(|item| {
        item.accessible_role() == AccessibleRole::Text && {
            let label = property(item, AccessibleStringProperty::Label);
            label.starts_with("Change notifications") || label.starts_with("Automatic refresh")
        }
    })?;
    Some((
        property(&item, AccessibleStringProperty::Label),
        property(&item, AccessibleStringProperty::Description),
    ))
}

#[test]
fn the_current_views_watch_status_is_shown_in_its_pane_until_native_or_another_view() {
    let root = std::env::temp_dir().join("watch-status-presentation");
    let mut preferences = UserPreferences::default();
    for tabs in &mut preferences.tabs {
        tabs.paths = vec![root.clone()];
    }
    let mut h = Harness::with_location(preferences, root.clone());
    let first = show(&mut h, &root);
    assert_eq!(shown(&h), None);

    report(&mut h, first, polling_only(&root));
    let (text, details) = shown(&h).expect("a polling-only folder says so in its pane");
    assert!(text.contains("about every 15 seconds"), "{text}");
    assert!(
        details.contains("OS file watch limit reached"),
        "the registration error explains it: {details}"
    );
    assert_eq!(h.window.get_left_watch_status(), text.as_str());
    assert!(h.window.get_right_watch_status().is_empty());
    assert!(h.window.get_left_error().is_empty(), "never a panel error");

    report(&mut h, first, WatchStatus::Native);
    assert_eq!(shown(&h), None, "Native clears it");

    report(&mut h, first, WatchStatus::Restarting);
    let (text, details) = shown(&h).expect("a restarting watch says so");
    assert!(text.contains("at most once a minute"), "{text}");
    assert!(details.is_empty());

    // Another folder is a new view; a late status of the old one is not shown.
    let next = show(&mut h, &root.join("next"));
    assert_eq!(shown(&h), None, "navigation clears it");
    report(&mut h, first, polling_only(&root));
    assert_eq!(shown(&h), None, "a replaced view's status");
    report(&mut h, next, polling_only(&root.join("next")));
    assert!(shown(&h).is_some(), "the new view's own status");
}
