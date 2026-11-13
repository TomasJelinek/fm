use super::*;
use midnight_core::{DirectoryIndex, EntryId};
use std::sync::Arc;

fn presenter() -> Presenter {
    let root = std::env::current_dir().unwrap();
    let mut presenter = Presenter::new(root.clone(), root.clone());
    presenter.preferences.values.thumbnails_network = true;
    presenter.preferences.values.panel_view_modes = [PanelViewMode::Thumbnails; 2];
    let entries: Arc<[Entry]> = (0..2)
        .map(|index| {
            Entry::new(
                EntryId(index),
                root.join(format!("{index}.png")),
                EntryKind::File,
            )
        })
        .collect();
    presenter.panes[0].index = Arc::new(DirectoryIndex::from_entries(&entries));
    presenter.panes[0].entries = entries;
    presenter
}

#[test]
fn size_changes_keep_resident_images_until_a_sharper_frame_replaces_them() {
    let mut p = presenter();
    let offset = p.panes[0].parent_offset();
    let entry = p.panes[0].entries[0].clone();
    let targets = p.thumbnail_targets([(offset, offset + 1), (0, 0)], ["", ""]);
    let identity = &targets[0].identity;
    assert!(p.receive_thumbnail_at_resolution(identity, 0, 96, 48, 96, &vec![128; 96 * 48 * 4]));
    let old_bytes = p.thumbnails.bytes;
    p.change_thumbnail_size(192, &|_| true);
    let enlarged = p.thumbnail_targets([(offset, offset + 1), (0, 0)], ["", ""]);
    assert_eq!(enlarged[0].identity, *identity);
    assert_eq!(enlarged[0].edge, 192);
    assert_eq!(
        p.thumbnail_image(PaneId::Left, &entry)
            .unwrap()
            .size()
            .width,
        96
    );
    assert_eq!(p.thumbnails.bytes, old_bytes);
    assert!(p.receive_thumbnail_at_resolution(identity, 0, 192, 96, 192, &vec![255; 192 * 96 * 4]));
    assert_eq!(p.thumbnails.bytes, 192 * 96 * 4);
    assert_eq!(
        p.thumbnail_image(PaneId::Left, &entry)
            .unwrap()
            .size()
            .width,
        192
    );
    assert!(!p.receive_thumbnail_at_resolution(identity, 0, 96, 48, 96, &vec![128; 96 * 48 * 4]));
    p.change_thumbnail_size(48, &|_| true);
    assert_eq!(
        p.thumbnail_image(PaneId::Left, &entry)
            .unwrap()
            .size()
            .width,
        192
    );
    assert_eq!(p.thumbnails.bytes, 192 * 96 * 4);
}

#[test]
fn small_source_records_larger_request_coverage_without_reuploading_identical_pixels() {
    let mut p = presenter();
    let offset = p.panes[0].parent_offset();
    let targets = p.thumbnail_targets([(offset, offset + 1), (0, 0)], ["", ""]);
    let identity = &targets[0].identity;
    assert!(p.receive_thumbnail_at_resolution(identity, 0, 50, 25, 96, &vec![128; 50 * 25 * 4]));
    assert!(!p.receive_thumbnail_at_resolution(identity, 0, 50, 25, 256, &vec![128; 50 * 25 * 4]));
    assert_eq!(p.thumbnail_coverage(identity, 0), 256);
    assert_eq!(p.thumbnail_coverage(identity, 6), 0);
    assert_eq!(p.thumbnail_coverage("not-resident", 0), 0);
    assert_eq!(p.thumbnails.bytes, 50 * 25 * 4);
    assert!(!p.receive_thumbnail_at_resolution(identity, 0, 96, 48, 192, &vec![255; 96 * 48 * 4]));
    assert_eq!(p.thumbnails.bytes, 50 * 25 * 4);
}

#[test]
fn an_upgrade_that_exceeds_the_pixel_budget_keeps_every_visible_fallback() {
    let mut p = presenter();
    p.thumbnails.visible = (0..55)
        .map(|index| ThumbnailTarget {
            identity: format!("visible-{index}"),
            content: ThumbnailContent::Native(PathBuf::from("preview.png")),
            selected: false,
            hovered: false,
            confirmed_ssd: true,
            edge: 256,
        })
        .collect();
    let large = vec![128; 512 * 510 * 4];
    for index in 0..54 {
        assert!(p.receive_thumbnail_at_resolution(
            &format!("visible-{index}"),
            0,
            512,
            510,
            512,
            &large
        ));
    }
    assert!(p.receive_thumbnail_at_resolution(
        "visible-54",
        0,
        96,
        96,
        96,
        &vec![128; 96 * 96 * 4]
    ));
    let before = p.thumbnails.bytes;
    assert_eq!(p.thumbnail_coverage("visible-54", 0), 96);
    assert!(!p.receive_thumbnail_at_resolution(
        "visible-54",
        0,
        512,
        512,
        512,
        &vec![255; 512 * 512 * 4]
    ));
    assert_eq!(p.thumbnails.bytes, before);
    assert_eq!(p.thumbnail_coverage("visible-54", 0), 96);
    assert_eq!(p.thumbnails.images.len(), 55);
    assert_eq!(
        p.thumbnails.images["visible-54"][0]
            .as_ref()
            .unwrap()
            .size()
            .width,
        96
    );
    assert!(p.thumbnails.bytes <= 54 * 1024 * 1024);

    p.thumbnails
        .visible
        .retain(|target| target.identity != "visible-0" && target.identity != "visible-1");
    p.thumbnails.touch("visible-0");
    assert!(p.receive_thumbnail_at_resolution(
        "visible-54",
        0,
        512,
        512,
        512,
        &vec![255; 512 * 512 * 4]
    ));
    assert!(!p.thumbnails.images.contains_key("visible-1"));
    assert_eq!(p.thumbnail_coverage("visible-1", 0), 0);
    assert_eq!(p.thumbnail_coverage("visible-54", 0), 512);
    assert!(p.thumbnails.images.contains_key("visible-0"));
    assert!(p.thumbnails.images.contains_key("visible-2"));
    assert_eq!(p.thumbnails.images.len(), 54);
    assert_eq!(
        p.thumbnails.images["visible-54"][0]
            .as_ref()
            .unwrap()
            .size()
            .width,
        512
    );
    assert!(p.thumbnails.bytes <= 54 * 1024 * 1024);
}

#[test]
fn loaded_image_survives_scrolling_away_and_is_available_before_the_next_visible_timer() {
    let mut presenter = presenter();
    let offset = presenter.panes[0].parent_offset();
    let first = presenter.panes[0].entries[0].clone();
    let targets = presenter.thumbnail_targets([(offset, offset + 1), (0, 0)], ["", ""]);
    assert_eq!(targets.len(), 1);
    assert!(presenter.receive_thumbnail(&targets[0].identity, 0, 1, 1, &[10, 20, 30, 255]));
    presenter.thumbnail_targets([(offset + 1, offset + 2), (0, 0)], ["", ""]);
    // The panel may request the old row before its 100 ms visibility report runs.
    assert!(presenter.thumbnail_image(PaneId::Left, &first).is_some());
    presenter.thumbnail_targets([(offset, offset + 1), (0, 0)], ["", ""]);
    assert!(presenter.thumbnail_image(PaneId::Left, &first).is_some());
}

#[test]
fn retained_image_cannot_cross_a_replaced_directory_generation() {
    let mut presenter = presenter();
    let offset = presenter.panes[0].parent_offset();
    let first = presenter.panes[0].entries[0].clone();
    let targets = presenter.thumbnail_targets([(offset, offset + 1), (0, 0)], ["", ""]);
    assert!(presenter.receive_thumbnail(&targets[0].identity, 0, 1, 1, &[10, 20, 30, 255]));
    presenter.panes[0].generation += 1;
    assert!(presenter.thumbnail_image(PaneId::Left, &first).is_none());
    presenter.thumbnail_targets([(offset, offset + 1), (0, 0)], ["", ""]);
    assert!(!presenter.receive_thumbnail(&targets[0].identity, 0, 1, 1, &[40, 50, 60, 255]));
}

#[test]
fn a_replayed_cache_frame_does_not_replace_or_upload_a_retained_image() {
    let mut presenter = presenter();
    let offset = presenter.panes[0].parent_offset();
    let targets = presenter.thumbnail_targets([(offset, offset + 1), (0, 0)], ["", ""]);
    let identity = &targets[0].identity;
    assert!(presenter.receive_thumbnail(identity, 0, 1, 1, &[10, 20, 30, 255]));
    assert!(!presenter.receive_thumbnail(identity, 0, 1, 1, &[10, 20, 30, 255]));
    presenter.clear_thumbnail_images();
    assert!(presenter.thumbnails.images.is_empty());
    assert_eq!(presenter.thumbnails.bytes, 0);
    assert!(presenter.receive_thumbnail(identity, 0, 1, 1, &[10, 20, 30, 255]));
}

#[test]
fn pending_refresh_keeps_the_displayed_snapshot_pixels_until_a_replacement_commits() {
    let mut presenter = presenter();
    let root = presenter.panes[0].path.clone();
    let entries = presenter.panes[0].entries.clone();
    assert!(presenter.navigate(PaneId::Left, root.clone(), None, &|_| true));
    presenter.apply(midnight_core::Event::ListingFinished {
        pane: PaneId::Left,
        navigation: midnight_core::NavigationId(presenter.panes[0].generation),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    let offset = presenter.panes[0].parent_offset();
    let first = presenter.panes[0].entries[0].clone();
    let targets = presenter.thumbnail_targets([(offset, offset + 1), (0, 0)], ["", ""]);
    let old = targets[0].identity.clone();
    assert!(presenter.receive_thumbnail(&old, 0, 1, 1, &[10, 20, 30, 255]));

    assert!(presenter.navigate(PaneId::Left, root, None, &|_| true));
    assert!(
        presenter.thumbnail_image(PaneId::Left, &first).is_some(),
        "the displayed snapshot has not changed while refresh is pending"
    );
    let targets = presenter.thumbnail_targets([(offset, offset + 1), (0, 0)], ["", ""]);
    assert_eq!(targets[0].identity, old);

    let entries: Arc<[Entry]> = vec![first.clone()].into();
    presenter.apply(midnight_core::Event::ListingFinished {
        pane: PaneId::Left,
        navigation: midnight_core::NavigationId(presenter.panes[0].generation),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    let targets = presenter.thumbnail_targets([(offset, offset + 1), (0, 0)], ["", ""]);
    assert_ne!(targets[0].identity, old);
    assert!(presenter.receive_thumbnail(&targets[0].identity, 0, 2, 1, &[255; 8]));
    assert!(!presenter.receive_thumbnail(&old, 0, 1, 1, &[0; 4]));
    assert_eq!(
        presenter
            .thumbnail_image(PaneId::Left, &first)
            .unwrap()
            .size()
            .width,
        2
    );
}

#[test]
fn a_large_visited_folder_retains_completed_images_when_pixels_fit_the_budget() {
    let mut presenter = presenter();
    let root = presenter.panes[0].path.clone();
    let entries: Arc<[Entry]> = (0..2219)
        .map(|index| {
            Entry::new(
                EntryId(index),
                root.join(format!("{index}.png")),
                EntryKind::File,
            )
        })
        .collect();
    presenter.panes[0].index = Arc::new(DirectoryIndex::from_entries(&entries));
    presenter.panes[0].entries = entries;
    let offset = presenter.panes[0].parent_offset();
    // A landscape preview workload comfortably below the existing 54 MiB limit.
    let pixels = vec![255; 96 * 48 * 4];
    for row in 0..2219 {
        let targets =
            presenter.thumbnail_targets([(offset + row, offset + row + 1), (0, 0)], ["", ""]);
        assert!(presenter.receive_thumbnail(&targets[0].identity, 0, 96, 48, &pixels));
    }
    assert!(presenter.thumbnails.bytes <= 54 * 1024 * 1024);
    assert!(
        presenter
            .thumbnail_image(PaneId::Left, &presenter.panes[0].entries[0])
            .is_some(),
        "scrolling back should not reload completed previews that fit the pixel budget"
    );
}

#[test]
fn retained_pixels_do_not_override_disabled_or_unclassified_network_preview_policy() {
    let mut presenter = presenter();
    // Opting out of archive members must not suppress native pixel retention.
    presenter.preferences.values.thumbnails_archives = false;
    let offset = presenter.panes[0].parent_offset();
    let first = presenter.panes[0].entries[0].clone();
    let targets = presenter.thumbnail_targets([(offset, offset + 1), (0, 0)], ["", ""]);
    assert!(presenter.receive_thumbnail(&targets[0].identity, 0, 1, 1, &[10, 20, 30, 255]));
    presenter.preferences.values.thumbnails_enabled = false;
    assert!(presenter.thumbnail_image(PaneId::Left, &first).is_none());
    presenter.preferences.values.thumbnails_enabled = true;
    assert_eq!(
        presenter.panes[0].visible_storage_kind(),
        StorageKind::Unknown
    );
    presenter.preferences.values.thumbnails_network = false;
    assert!(presenter.thumbnail_image(PaneId::Left, &first).is_none());
    presenter.preferences.values.thumbnails_network = true;
    assert!(presenter.thumbnail_image(PaneId::Left, &first).is_some());
}

#[test]
fn visited_static_and_animated_images_stay_within_record_and_pixel_budgets() {
    for (count, edge, frames) in [(5000, 1, 1), (400, 96, 6)] {
        let mut presenter = presenter();
        let root = std::env::current_dir().unwrap();
        let entries: Arc<[Entry]> = (0..count)
            .map(|index| {
                Entry::new(
                    EntryId(index),
                    root.join(format!("{index}.png")),
                    EntryKind::File,
                )
            })
            .collect();
        presenter.panes[0].index = Arc::new(DirectoryIndex::from_entries(&entries));
        presenter.panes[0].entries = entries;
        let offset = presenter.panes[0].parent_offset();
        let pixels = vec![255; edge * edge * 4];
        let mut oldest = String::new();
        for row in 0..count as usize {
            let targets =
                presenter.thumbnail_targets([(offset + row, offset + row + 1), (0, 0)], ["", ""]);
            if row == 0 {
                oldest = targets[0].identity.clone();
            }
            for frame in 0..frames {
                assert!(presenter.receive_thumbnail(
                    &targets[0].identity,
                    frame,
                    edge as u32,
                    edge as u32,
                    &pixels
                ));
            }
        }
        assert!(presenter.thumbnails.images.len() <= 4096);
        assert!(presenter.thumbnails.bytes <= 54 * 1024 * 1024);
        assert!(!presenter.thumbnails.images.contains_key(&oldest));
        assert!(
            presenter
                .thumbnail_image(
                    PaneId::Left,
                    &presenter.panes[0].entries[count as usize - 1]
                )
                .is_some()
        );
    }
}
