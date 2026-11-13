//! Small shared projections for navigation menus, breadcrumbs and focused-entry details.
use crate::*;

fn bookmark_tooltip(presenter: &Presenter, bookmark: &midnight_core::Bookmark) -> String {
    let mut text = format!("Hot Path\n{}", bookmark.display_text());
    if let Some(slot) = bookmark.slot {
        let keys = presenter.shortcut_hint(&format!("bookmark.open.{slot}"));
        if !keys.is_empty() {
            text.push('\n');
            text.push_str(keys);
        }
    }
    text
}

pub(super) fn bookmark_choices(presenter: &Presenter) -> Vec<NavigationChoiceRow> {
    let mut bookmarks = presenter.bookmarks().iter().collect::<Vec<_>>();
    bookmarks.sort_by_key(|bookmark| bookmark.slot.unwrap_or(11));
    bookmarks
        .into_iter()
        .map(|bookmark| NavigationChoiceRow {
            identity: format!(
                "bookmark:{}:{}",
                presenter.preferences.revision.0, bookmark.id.0
            )
            .into(),
            label: bookmark
                .slot
                .map_or_else(
                    || bookmark.display_text(),
                    |slot| format!("{}  {}", slot % 10, bookmark.display_text()),
                )
                .into(),
            compact_label: bookmark
                .slot
                .map(|slot| (slot % 10).to_string())
                .unwrap_or_default()
                .into(),
            tooltip: bookmark_tooltip(presenter, bookmark).into(),
            icon: presenter
                .desktop
                .image(&IconKey::action(ActionIcon::Bookmarks)),
            kind: LocationRole::HotPath,
        })
        .collect()
}

pub(super) fn refresh(window: &MainWindow, presenter: &Presenter) {
    crate::metadata_column_presentation::refresh(window, presenter);
    window.set_left_can_target_cursor(presenter.can_target_cursor(PaneId::Left));
    window.set_right_can_target_cursor(presenter.can_target_cursor(PaneId::Right));
    window.set_left_can_target_selection(presenter.can_target_selection(PaneId::Left));
    window.set_right_can_target_selection(presenter.can_target_selection(PaneId::Right));
    window.set_left_free_space(volume_presentation::panel_space(presenter, PaneId::Left).into());
    window.set_right_free_space(volume_presentation::panel_space(presenter, PaneId::Right).into());
    window.set_sizes_in_bytes(presenter.preferences.values.size_display == SizeDisplay::Bytes);
    window.set_remember_command_history(presenter.preferences.values.remember_command_history);
    window.set_copy_checksum_verification(presenter.preferences.values.copy_checksum_verification);
    window.set_disk_io_concurrency_index(match presenter.preferences.values.disk_io_concurrency {
        DiskIoConcurrency::Automatic => 0,
        DiskIoConcurrency::Manual(maximum @ 1..=32) => i32::from(maximum),
        DiskIoConcurrency::Manual(_) => -1,
    });
    let mut hot_paths = bookmark_choices(presenter);
    hot_paths.push(NavigationChoiceRow {
        identity: "add-hot-path".into(),
        label: "Add path".into(),
        compact_label: "Add path".into(),
        tooltip: "Save the active panel's current folder".into(),
        icon: presenter
            .desktop
            .image(&IconKey::action(ActionIcon::AddBookmark)),
        kind: LocationRole::HotPath,
    });
    hot_paths.push(NavigationChoiceRow {
        identity: "manage-hot-paths".into(),
        label: "Manage Hot Paths…".into(),
        compact_label: "Hot Paths".into(),
        tooltip: "Add or remove saved paths and assign numbered shortcuts".into(),
        icon: presenter
            .desktop
            .image(&IconKey::action(ActionIcon::Bookmarks)),
        kind: LocationRole::HotPath,
    });
    navigation_models::update(
        window.get_hot_path_choices(),
        hot_paths,
        |row| row.identity.clone(),
        |model| window.set_hot_path_choices(model),
    );
    history_models::refresh(window, presenter);
    window.set_command_history_revision(presenter.preferences.revision.0 as i32);
    window.set_history_revision(
        presenter.panes[0]
            .generation
            .wrapping_add(presenter.panes[1].generation) as i32,
    );
    for pane in [PaneId::Left, PaneId::Right] {
        let paths = presenter
            .path_parts(pane)
            .into_iter()
            .map(|part| PathPartRow {
                identity: part.identity.into(),
                label: part.label.into(),
                prefix: part.prefix.into(),
            })
            .collect();
        refresh_details(window, details_rows(presenter, pane), pane);
        if pane == PaneId::Left {
            navigation_models::update(
                window.get_left_path_parts(),
                paths,
                |row| row.identity.clone(),
                |model| window.set_left_path_parts(model),
            );
        } else {
            navigation_models::update(
                window.get_right_path_parts(),
                paths,
                |row| row.identity.clone(),
                |model| window.set_right_path_parts(model),
            );
        }
    }
}

pub(super) fn bookmark_id(presenter: &Presenter, token: &str) -> Option<BookmarkId> {
    let (revision, id) = token.strip_prefix("bookmark:")?.split_once(':')?;
    (revision.parse::<u64>().ok()? == presenter.preferences.revision.0)
        .then_some(BookmarkId(id.parse::<u64>().ok()?))
}

/// Capture the bounded status projection before Slint can notify model observers.
pub(crate) fn details_rows(presenter: &Presenter, pane: PaneId) -> Vec<DetailRow> {
    presenter
        .panel_details(pane)
        .into_iter()
        .map(|field| DetailRow {
            identity: field.identity.into(),
            label: field.label.into(),
            value: field.value.into(),
            explanation: field.explanation.into(),
        })
        .collect()
}

/// Publish the status snapshot without retaining a presenter borrow.
pub(crate) fn refresh_details(window: &MainWindow, details: Vec<DetailRow>, pane: PaneId) {
    if pane == PaneId::Left {
        navigation_models::update(
            window.get_left_details(),
            details,
            |row| row.identity.clone(),
            |model| window.set_left_details(model),
        );
    } else {
        navigation_models::update(
            window.get_right_details(),
            details,
            |row| row.identity.clone(),
            |model| window.set_right_details(model),
        );
    }
}
