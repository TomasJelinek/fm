use super::*;
use midnight_core::{Entry, EntryId, EntryKind, PanelViewMode};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    sync::Arc,
};

fn tile(identity: &str, logical: u32) -> ThumbnailTarget {
    ThumbnailTarget {
        identity: identity.into(),
        content: ThumbnailContent::FileIcon(PathBuf::from(format!("C:/programs/{identity}.exe"))),
        selected: false,
        hovered: false,
        confirmed_ssd: false,
        edge: logical,
    }
}

fn preview(identity: &str) -> ThumbnailTarget {
    ThumbnailTarget {
        content: ThumbnailContent::Native(PathBuf::from(format!("C:/pictures/{identity}.png"))),
        ..tile(identity, 96)
    }
}

/// A presenter whose visible tiles are `rows`, as the last thumbnail pass left them.
fn showing(rows: &[ThumbnailTarget]) -> Presenter {
    let mut presenter = Presenter::new(PathBuf::new(), PathBuf::new());
    presenter.thumbnails.visible = rows.to_vec();
    presenter
}

/// An accepted view: its generation and the identities and edges it asked for.
type View = (u64, Vec<(String, u16)>);

/// Records the file icon views the backend accepted, refusing while `refuse` is on.
#[derive(Default)]
struct Backend {
    views: RefCell<Vec<View>>,
    refuse: Cell<bool>,
    offered: Cell<usize>,
}

impl Backend {
    fn send(&self) -> impl Fn(Command) -> bool + '_ {
        move |command| {
            let Command::SetFileIconView {
                generation,
                targets,
            } = command
            else {
                panic!("unexpected command: {command:?}");
            };
            self.offered.set(self.offered.get() + 1);
            if self.refuse.get() {
                return false;
            }
            let targets = targets
                .into_iter()
                .map(|target| (target.identity, target.edge))
                .collect();
            self.views.borrow_mut().push((generation, targets));
            true
        }
    }

    fn last(&self) -> View {
        self.views
            .borrow()
            .last()
            .cloned()
            .expect("a view was sent")
    }
}

fn targets(entries: &[(&str, u16)]) -> Vec<(String, u16)> {
    entries
        .iter()
        .map(|(identity, edge)| ((*identity).into(), *edge))
        .collect()
}

fn artwork(edge: u16) -> IconPixels {
    let edge = u32::from(edge);
    IconPixels::new(edge, edge, vec![180; (edge * edge * 4) as usize]).unwrap()
}

#[test]
fn tile_edges_follow_the_display_scale() {
    assert_eq!(device_edge(96, 1.0), 96);
    assert_eq!(device_edge(96, 1.25), 120);
    assert_eq!(device_edge(96, 2.25), 216);
    assert_eq!(device_edge(64, 1.5), 96);
    assert_eq!(
        device_edge(192, 1.5),
        MAX_ICON_EDGE,
        "288 px tiles get the largest"
    );
    assert_eq!(device_edge(256, 1.0), MAX_ICON_EDGE);
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(device_edge(72, scale), 72, "{scale}");
    }
}

#[test]
fn visible_program_tiles_without_artwork_are_asked_for_once_at_their_device_edge() {
    let rows = [tile("a", 96), tile("b", 96), preview("c")];
    let mut presenter = showing(&rows);
    let backend = Backend::default();
    presenter.refresh_file_icons(&rows, 2.25, &backend.send());
    assert_eq!(backend.last(), (1, targets(&[("a", 216), ("b", 216)])));
    // Nothing changed: nothing is offered again.
    presenter.refresh_file_icons(&rows, 2.25, &backend.send());
    assert_eq!(backend.offered.get(), 1);

    // Artwork of the current view is admitted and does not start another view, so
    // the load in progress for "b" is not cancelled.
    assert!(presenter.receive_file_icon(1, "a", 216, &artwork(216)));
    assert_eq!(presenter.thumbnail_coverage("a", 0), 216);
    presenter.refresh_file_icons(&rows, 2.25, &backend.send());
    assert_eq!(backend.offered.get(), 1);
    // Another generation, another edge or a tile that was not asked for is refused.
    assert!(!presenter.receive_file_icon(0, "b", 216, &artwork(216)));
    assert!(!presenter.receive_file_icon(1, "b", 96, &artwork(96)));
    assert!(!presenter.receive_file_icon(1, "c", 216, &artwork(216)));
    assert_eq!(presenter.thumbnail_coverage("b", 0), 0);

    // Scrolled: "a" left, "d" arrived. Only tiles still without artwork are asked.
    let scrolled = [tile("b", 96), tile("d", 96), tile("a", 96)];
    presenter.thumbnails.visible = scrolled.to_vec();
    presenter.refresh_file_icons(&scrolled, 2.25, &backend.send());
    assert_eq!(backend.last(), (2, targets(&[("b", 216), ("d", 216)])));
    assert!(
        !presenter.receive_file_icon(1, "b", 216, &artwork(216)),
        "stale view"
    );
    // A display of another density asks again at the new edge; artwork at least as
    // large as a tile covers it, as a larger preview does.
    presenter.refresh_file_icons(&scrolled, 1.0, &backend.send());
    assert_eq!(backend.last(), (3, targets(&[("b", 96), ("d", 96)])));
    presenter.refresh_file_icons(&scrolled, 2.5, &backend.send());
    assert_eq!(
        backend.last(),
        (4, targets(&[("b", 240), ("d", 240), ("a", 240)])),
        "216 px artwork does not cover a 240 px tile"
    );
}

#[test]
fn a_refused_view_is_offered_again_and_a_covered_view_cancels_what_still_loads() {
    let rows = [tile("a", 96), tile("b", 96)];
    let mut presenter = showing(&rows);
    let backend = Backend::default();
    backend.refuse.set(true);
    presenter.refresh_file_icons(&rows, 1.0, &backend.send());
    presenter.refresh_file_icons(&rows, 1.0, &backend.send());
    assert_eq!(
        backend.offered.get(),
        2,
        "a refused view is offered on each pass"
    );
    backend.refuse.set(false);
    presenter.refresh_file_icons(&rows, 1.0, &backend.send());
    assert_eq!(backend.last(), (1, targets(&[("a", 96), ("b", 96)])));

    // "a" arrived; scrolling to it alone needs nothing, and the empty view stops the
    // load of "b", which left the view.
    assert!(presenter.receive_file_icon(1, "a", 96, &artwork(96)));
    let only_a = [tile("a", 96)];
    presenter.thumbnails.visible = only_a.to_vec();
    presenter.refresh_file_icons(&only_a, 1.0, &backend.send());
    assert_eq!(backend.last(), (2, Vec::new()));
    // With nothing loading, a view whose tiles all have artwork sends nothing.
    let offered = backend.offered.get();
    let reordered = [preview("p"), tile("a", 96)];
    presenter.refresh_file_icons(&reordered, 1.0, &backend.send());
    assert_eq!(backend.offered.get(), offered);
}

#[test]
fn a_cleared_cache_asks_again_for_the_visible_tiles() {
    let rows = [tile("a", 128)];
    let mut presenter = showing(&rows);
    let backend = Backend::default();
    presenter.refresh_file_icons(&rows, 1.0, &backend.send());
    assert!(presenter.receive_file_icon(1, "a", 128, &artwork(128)));
    presenter.clear_thumbnail_images();
    presenter.thumbnails.visible = rows.to_vec();
    presenter.refresh_file_icons(&rows, 1.0, &backend.send());
    assert_eq!(backend.last(), (2, targets(&[("a", 128)])));
}

#[test]
fn program_artwork_is_retained_within_the_preview_byte_budget_least_recent_first() {
    // The thumbnail image cache's pixel budget (`thumbnail_image_cache.rs`).
    const BUDGET: usize = 54 * 1024 * 1024;
    let tile_bytes = 216 * 216 * 4;
    let mut presenter = showing(&[]);
    let backend = Backend::default();
    // Scrolling through 400 program tiles, one screen of three at a time.
    for screen in 0..400 / 3 {
        let rows: Vec<_> = (0..3)
            .map(|index| tile(&format!("program {}", screen * 3 + index), 96))
            .collect();
        presenter.thumbnails.visible = rows.clone();
        presenter.refresh_file_icons(&rows, 2.25, &backend.send());
        let (generation, targets) = backend.last();
        for (identity, edge) in targets {
            assert!(presenter.receive_file_icon(generation, &identity, edge, &artwork(edge)));
        }
        assert!(presenter.thumbnails.bytes <= BUDGET, "screen {screen}");
    }
    let kept = presenter.thumbnails.bytes / tile_bytes;
    assert_eq!(
        kept,
        BUDGET / tile_bytes,
        "the budget is used, not exceeded"
    );
    // The visible screen and the most recent ones stay; the oldest went first.
    for recent in ["program 396", "program 395", "program 300"] {
        assert_eq!(presenter.thumbnail_coverage(recent, 0), 216, "{recent}");
    }
    for old in ["program 0", "program 50"] {
        assert_eq!(presenter.thumbnail_coverage(old, 0), 0, "{old}");
    }
}

#[test]
fn programs_in_a_tile_pane_show_their_own_artwork_where_the_platform_reads_it() {
    let root = std::env::current_dir().unwrap();
    let mut presenter = Presenter::new(root.clone(), root.clone());
    presenter.preferences.values.thumbnails_network = true;
    presenter.preferences.values.panel_view_modes =
        [PanelViewMode::Thumbnails, PanelViewMode::Detailed];
    let entries: Arc<[Entry]> = ["tool.EXE", "picture.png", "notes.txt"]
        .iter()
        .zip(0..)
        .map(|(name, id)| Entry::new(EntryId(id), root.join(name), EntryKind::File))
        .collect();
    presenter.panes[0].index = Arc::new(midnight_core::DirectoryIndex::from_entries(&entries));
    presenter.panes[0].entries = entries;
    let offset = presenter.panes[0].parent_offset();
    let rows = presenter.thumbnail_targets([(offset, offset + 3), (0, 0)], ["", ""]);
    let program = rows
        .iter()
        .find(|row| row.identity.contains(":entry:0:"))
        .expect("the program is a visible tile");
    assert_eq!(
        matches!(program.content, ThumbnailContent::FileIcon(_)),
        cfg!(windows),
        "{:?}",
        program.content
    );
    for row in rows
        .iter()
        .filter(|row| !row.identity.contains(":entry:0:"))
    {
        assert!(
            matches!(row.content, ThumbnailContent::Native(_)),
            "{row:?}"
        );
    }
    assert!(!carries_own_artwork(None));
}
