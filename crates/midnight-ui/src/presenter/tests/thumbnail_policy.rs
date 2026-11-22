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

/// A Thumbnails view of one image in a folder whose classification has not arrived.
fn thumbnail_folder() -> (Presenter, NavigationId) {
    let mut p = Presenter::new(test_root(), test_root());
    p.preferences.values.panel_view_modes[0] = PanelViewMode::Thumbnails;
    let navigation = begin(&mut p, PaneId::Left, test_root().to_str().unwrap());
    let entry = Entry::new(EntryId(1), test_root().join("tile.png"), EntryKind::File);
    finish(&mut p, PaneId::Left, navigation, vec![entry]);
    (p, navigation)
}

fn classify(p: &mut Presenter, navigation: NavigationId, kind: StorageKind) {
    p.apply(Event::StorageKindUpdated {
        pane: PaneId::Left,
        navigation,
        kind,
    });
}

/// THUMB-UNKNOWN-STORAGE: the disk of a virtual or cloud machine (WSL 2's Hyper-V disk)
/// is final local storage of unknown media type, not a pending classification: its
/// previews need no network permission, and their reads stay exclusive.
#[test]
fn a_local_disk_of_unknown_media_admits_previews_without_network_permission() {
    let (mut p, navigation) = thumbnail_folder();
    assert!(
        p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]).is_empty(),
        "nothing is read before the classification arrives"
    );
    assert_eq!(
        p.thumbnail_status(PaneId::Left),
        None,
        "pending says nothing"
    );
    classify(&mut p, navigation, StorageKind::Local);
    let targets = p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]);
    assert_eq!(targets.len(), 1);
    assert!(
        !targets[0].confirmed_ssd,
        "unknown media keeps exclusive I/O"
    );
    assert_eq!(p.thumbnail_status(PaneId::Left), None);
}

/// Network storage and storage not known to be local keep today's rule, and the view
/// names the reason and the option that changes it, without a dialog.
#[test]
fn network_and_unclassified_storage_withhold_previews_and_say_which_option_allows_them() {
    for (kind, reason) in [
        (StorageKind::Network, "network locations"),
        (StorageKind::Unknown, "not known to be local"),
    ] {
        let (mut p, navigation) = thumbnail_folder();
        classify(&mut p, navigation, kind);
        assert!(p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]).is_empty());
        let status = p
            .thumbnail_status(PaneId::Left)
            .expect("the view says why it shows no previews");
        assert!(status.text.contains(reason), "{}", status.text);
        assert!(
            status
                .text
                .contains("Options › Previews › Allow previews on network locations"),
            "{}",
            status.text
        );
        assert!(p.desktop.notice.is_none(), "never a dialog");
        assert_eq!(
            p.thumbnail_status(PaneId::Right),
            None,
            "only the withheld view"
        );
        p.preferences.values.thumbnails_network = true;
        assert_eq!(p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]).len(), 1);
        assert_eq!(p.thumbnail_status(PaneId::Left), None);
    }
}

#[test]
fn switched_off_archive_previews_name_their_option() {
    let mut p = super::archives::archive_pane();
    p.preferences.values.panel_view_modes[0] = PanelViewMode::Thumbnails;
    p.preferences.values.thumbnails_archives = false;
    assert!(p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]).is_empty());
    let status = p.thumbnail_status(PaneId::Left).expect("the view says why");
    assert!(
        status
            .text
            .contains("Options › Previews › Allow previews inside archives"),
        "{}",
        status.text
    );
    p.preferences.values.thumbnails_enabled = false;
    assert_eq!(
        p.thumbnail_status(PaneId::Left),
        None,
        "previews the user switched off say nothing"
    );
}

/// WIN-11: a preview-service failure is a status-row text of the panes that show
/// thumbnails, never a notice; clearing the previews clears it.
#[test]
fn a_preview_service_failure_is_a_status_row_never_a_notice() {
    let (mut p, navigation) = thumbnail_folder();
    classify(&mut p, navigation, StorageKind::LocalSsd);
    p.report_thumbnail_error("Could not store or read preview cache: disk full".into());
    assert!(
        p.desktop.notice.is_none(),
        "never a dialog that takes the keyboard"
    );
    assert_eq!(p.desktop.notice_generation, 0);
    let status = p.thumbnail_status(PaneId::Left).expect("shown in the view");
    assert_eq!(
        status.text,
        "Could not store or read preview cache: disk full"
    );
    assert_eq!(
        p.thumbnail_status(PaneId::Right),
        None,
        "a pane without thumbnails"
    );
    p.clear_thumbnail_images();
    assert_eq!(p.thumbnail_status(PaneId::Left), None);
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
    p.panes[1].storage_kind = Some(StorageKind::LocalSsd);
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
    p.panes[0].storage_kind = Some(StorageKind::LocalSsd);
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

/// WIN-11's siblings: startup's own background requests report a failure on the status
/// line, never as a notice that takes the keyboard from the first panel.
#[test]
fn a_failed_startup_locale_load_is_a_status_line_never_a_notice() {
    let mut p = Presenter::new(test_root(), test_root());
    p.apply(Event::LocaleLoaded(Err(FsError::from_io(
        "load locale",
        Path::new("locale"),
        io::Error::other("no regional settings"),
    ))));
    assert!(p.desktop.notice.is_none(), "never a dialog");
    let background = p.desktop.background.as_deref().unwrap_or_default();
    assert!(
        background.starts_with("Regional format unavailable: "),
        "{background}"
    );
}
