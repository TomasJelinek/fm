use super::*;
use midnight_core::StorageKind;

#[test]
fn delayed_network_classification_never_admits_a_preview_without_opt_in() {
    let mut p = Presenter::new(test_root(), test_root());
    p.preferences.values.panel_view_modes[0] = PanelViewMode::Thumbnails;
    let navigation = begin(&mut p, PaneId::Left, test_root().to_str().unwrap());
    let entry = Entry::new(EntryId(1), test_root().join("clip.avi"), EntryKind::File);
    finish(&mut p, PaneId::Left, navigation, vec![entry]);
    assert!(p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]).is_empty());
    p.apply(Event::StorageKindUpdated {
        pane: PaneId::Left,
        navigation,
        kind: StorageKind::Network,
    });
    assert!(p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]).is_empty());
    p.preferences.values.thumbnails_network = true;
    assert_eq!(p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]).len(), 1);
}

#[test]
fn archive_previews_snapshot_readable_members_without_doing_io_on_ui_thread() {
    // The fixture mount panics on listing, validation and reads: presentation
    // may capture its authority but must never call those methods.
    let mut p = super::archives::archive_pane();
    p.preferences.values.panel_view_modes[0] = PanelViewMode::Thumbnails;
    let rows = p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]);
    assert_eq!(rows.len(), 2, "both distinct same-caption archive files");
    assert_ne!(rows[0].identity, rows[1].identity);
}

#[test]
fn disabling_archive_previews_stops_member_admission_without_blocking_native_previews() {
    let mut p = super::archives::archive_pane();
    p.preferences.values.panel_view_modes = [PanelViewMode::Thumbnails; 2];
    let native_path = test_root().join("native-preview.png");
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(70),
        native_path.clone(),
        EntryKind::File,
    )]
    .into();
    p.panes[1].index = Arc::new(DirectoryIndex::from_entries(&entries));
    p.panes[1].entries = entries;
    p.panes[1].storage_kind = StorageKind::LocalSsd;
    assert_eq!(p.thumbnail_targets([(0, 10); 2], ["", ""]).len(), 3);

    p.preferences.values.thumbnails_archives = false;
    let targets = p.thumbnail_targets([(0, 10); 2], ["", ""]);
    assert_eq!(
        targets.len(),
        1,
        "archive opt-out must leave native previews available"
    );
    assert!(
        matches!(&targets[0].content, crate::ThumbnailContent::Native(path) if path == &native_path)
    );
}

#[test]
fn disabling_archive_previews_hides_resident_pixels_and_rejects_frames_before_timer() {
    let mut p = super::archives::archive_pane();
    p.preferences.values.panel_view_modes[0] = PanelViewMode::Thumbnails;
    let entry = p.panes[0].entries[1].clone();
    let targets = p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]);
    let identity = targets[0].identity.clone();
    assert!(p.receive_thumbnail(&identity, 0, 1, 1, &[10, 20, 30, 255]));
    assert!(p.thumbnail_image(PaneId::Left, &entry).is_some());

    p.preferences.values.thumbnails_archives = false;
    assert!(
        p.thumbnail_image(PaneId::Left, &entry).is_none(),
        "hide cached pixels before the next target pulse"
    );
    assert!(
        !p.receive_thumbnail(&identity, 1, 1, 1, &[255; 4]),
        "an already-visible archive target must not accept a late frame after opt-out"
    );
    assert!(p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]).is_empty());
    assert!(!p.receive_thumbnail(&identity, 2, 1, 1, &[255; 4]));

    // Like the master preview switch, re-enabling can reuse completed pixels
    // without inventing new listing identities or inspecting the archive here.
    p.preferences.values.thumbnails_archives = true;
    let resumed = p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]);
    assert_eq!(resumed[0].identity, identity);
    assert_eq!(p.thumbnail_coverage(&identity, 0), 96);
    assert!(p.thumbnail_image(PaneId::Left, &entry).is_some());
}

#[test]
fn archive_previews_keep_resident_pixels_when_scrolled_back_and_reject_stale_listing() {
    let mut p = super::archives::archive_pane();
    p.preferences.values.panel_view_modes[0] = PanelViewMode::Thumbnails;
    let rows = p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]);
    assert_eq!(rows.len(), 2);
    let identity = rows[0].identity.clone();
    assert!(p.receive_thumbnail_at_resolution(&identity, 0, 1, 1, 96, &[1, 2, 3, 255]));
    assert!(p.thumbnail_targets([(0, 0), (0, 0)], ["", ""]).is_empty());
    assert_eq!(p.thumbnail_coverage(&identity, 0), 96);
    assert_eq!(
        p.thumbnail_targets([(0, 10), (0, 0)], ["", ""])[0].identity,
        identity
    );
    assert_eq!(p.thumbnail_coverage(&identity, 0), 96);
    p.panes[0].successful_generation = Some(NavigationId(999));
    p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]);
    assert!(!p.receive_thumbnail_at_resolution(&identity, 0, 2, 1, 192, &[255; 8]));
}

#[test]
fn archive_preview_admission_rejects_links_unknown_sizes_and_foreign_member_authority() {
    let mut p = super::archives::archive_pane();
    p.preferences.values.panel_view_modes[0] = PanelViewMode::Thumbnails;
    let file = p.panes[0].entries[1].clone();
    let mut invalid = Vec::new();
    for variant in 0..7 {
        let mut entry = file.clone();
        let node = entry.virtual_node.as_mut().unwrap();
        match variant {
            0 => node.readable = false,
            1 => node.size = None,
            2 => node.id.mount = 99,
            3 => node.id.entry = 0,
            4 => node.parent = 99,
            5 => node.kind = EntryKind::Symlink,
            _ => {
                entry.virtual_node = None;
                entry.path = test_root().join("must-not-open.png");
            }
        }
        invalid.push(entry);
    }
    p.panes[0].index = Arc::new(DirectoryIndex::from_entries(&invalid));
    p.panes[0].entries = invalid.into();
    assert!(p.thumbnail_targets([(0, 20), (0, 0)], ["", ""]).is_empty());
}

#[test]
fn archive_preview_identity_carries_policy_and_never_inherits_native_ssd_permission() {
    let mut p = super::archives::archive_pane();
    p.preferences.values.panel_view_modes[0] = PanelViewMode::Thumbnails;
    p.panes[0].storage_kind = StorageKind::LocalSsd;
    let old = p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]);
    assert_eq!(old.len(), 2);
    assert!(!old[0].confirmed_ssd);
    assert!(matches!(
        old[0].content,
        crate::ThumbnailContent::Archive {
            network_allowed: false,
            ..
        }
    ));
    p.preferences.values.thumbnails_network = true;
    let new = p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]);
    assert_eq!(old[0].identity, new[0].identity);
    assert_ne!(
        old[0].content, new[0].content,
        "policy change must replace pinned capture"
    );
    p.panes[0].archive.as_mut().unwrap().location.mount = None;
    assert!(p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]).is_empty());
}

#[test]
fn disabling_network_previews_hides_old_archive_pixels_until_backend_readmission() {
    let mut p = super::archives::archive_pane();
    p.preferences.values.panel_view_modes[0] = PanelViewMode::Thumbnails;
    p.preferences.values.thumbnails_network = true;
    let entry = p.panes[0].entries[1].clone();
    let rows = p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]);
    assert!(p.receive_thumbnail(&rows[0].identity, 0, 1, 1, &[255; 4]));
    assert!(p.thumbnail_image(PaneId::Left, &entry).is_some());
    p.preferences.values.thumbnails_network = false;
    assert!(
        p.thumbnail_image(PaneId::Left, &entry).is_none(),
        "hide before next timer pulse"
    );
    assert!(
        !p.receive_thumbnail(&rows[0].identity, 1, 1, 1, &[255; 4]),
        "reject old-policy frame"
    );
    let current = p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]);
    assert_eq!(p.thumbnail_coverage(&current[0].identity, 0), 0);
    assert!(p.thumbnail_image(PaneId::Left, &entry).is_none());
    // A fresh successful backend admission may restore a local archive image from disk cache.
    assert!(p.receive_thumbnail(&current[0].identity, 0, 1, 1, &[255; 4]));
    assert!(p.thumbnail_image(PaneId::Left, &entry).is_some());
}
