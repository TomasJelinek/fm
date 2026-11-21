//! Retained document tiles over scripted camera motion, with a counting page renderer in
//! place of the plugin: scrolling never renders a tile twice, finished bands survive newer
//! cameras, and continuous input keeps publishing frames.
use super::*;
use crate::{
    document_render::{PageRenderer, render_band},
    document_view::DocumentView,
    image_tiles::Key,
    image_view::{ImageView, Paint},
};
use midnight_core::DocumentLayout;
use midnight_image_viewer_host::{Frame, Metadata, Viewport};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicU64},
    time::Duration,
};

#[derive(Default)]
struct Counting {
    pages: HashMap<usize, usize>,
}

impl PageRenderer for Counting {
    fn render_page(
        &mut self,
        page: usize,
        viewport: Viewport,
        _: &AtomicBool,
        _: Duration,
    ) -> Result<Frame, String> {
        *self.pages.entry(page).or_default() += 1;
        Ok(Frame {
            width: viewport.width,
            height: viewport.height,
            rgba: vec![255; viewport.width as usize * viewport.height as usize * 4],
        })
    }

    fn validate(&mut self, _: &AtomicBool) -> Result<(), String> {
        Ok(())
    }
}

struct Harness {
    view: ImageView,
    requests: Receiver<Render>,
    renderer: Counting,
    /// How often each tile was rendered.
    tiles: HashMap<Key, usize>,
    published: usize,
    sequence: u64,
    /// The camera the UI's tiled camera wants, independent of the engine's progress.
    wanted: [f64; 2],
}

impl Harness {
    /// `pages` Letter pages in the Continuous layout at `zoom` physical pixels per point.
    fn new(pages: usize, viewport: [u32; 2], zoom: f64) -> Self {
        let (worker, requests) = test_worker();
        let mut view = ImageView::new(worker, Arc::new(AtomicU64::new(0)));
        let mut document = DocumentView::new(
            &vec![
                Metadata {
                    width: 612,
                    height: 792,
                };
                pages
            ],
            viewport,
        )
        .unwrap();
        document.painted(1);
        assert!(document.set_layout(1, DocumentLayout::Continuous));
        document.geometry.scale = zoom;
        document.geometry.fitted = false;
        document.pan_to([0.0, 0.0]);
        let wanted = document.geometry.origin;
        view.document = true;
        view.document_view = Some(document);
        view.changed();
        Self {
            view,
            requests,
            renderer: Counting::default(),
            tiles: HashMap::new(),
            published: 0,
            sequence: 0,
            wanted,
        }
    }

    /// One request, as the viewer worker's paint does: a published frame then lets a
    /// camera that waited for it move on (`ImageView::published`).
    fn request(&mut self) {
        if let Some(Paint::Tiles(_)) = self.view.request().unwrap() {
            self.published += 1;
            self.view.published();
        }
    }

    /// Answer the actor requests until the view is idle; returns the bands rendered.
    fn settle(&mut self) -> usize {
        let mut bands = 0;
        loop {
            self.request();
            match self.requests.try_recv() {
                Ok(Render::Band { revision, plan }) => {
                    self.answer(revision, plan);
                    bands += 1;
                }
                Ok(Render::Validate(revision)) => {
                    self.view.pending = None;
                    self.view.validated = Some(revision);
                }
                Ok(Render::Tile { .. }) => panic!("documents render bands, not single tiles"),
                Err(_) => return bands,
            }
        }
    }

    /// Render a band as the actor does and deliver it as `Event::Tiles` does.
    fn answer(&mut self, revision: u64, plan: crate::document_render::BandPlan) {
        assert!(
            plan.size[0] <= 4096 && plan.size[1] <= 4096,
            "a band within the provider's edge: {:?}",
            plan.size
        );
        for (key, _) in &plan.tiles {
            *self.tiles.entry(*key).or_default() += 1;
        }
        let tiles = render_band(
            &mut self.renderer,
            plan,
            &AtomicBool::new(false),
            Duration::from_secs(15),
        )
        .unwrap();
        self.view.pending = None;
        self.view.validated = Some(revision);
        self.view.scene.complete_band(tiles).unwrap();
    }

    /// The UI's tiled camera moving its retained tiles: `ViewerImageInput::PanTo`, as the
    /// viewer worker applies it. A far target may wait for the camera in progress.
    fn pan(&mut self, dy: f64) {
        self.wanted[1] += dy;
        self.sequence += 1;
        let surface = self.view.scene.surface;
        if self.view.pan_document(surface, self.sequence, self.wanted) {
            self.view.changed_by(true);
        }
        // An applied camera is clamped at either end; the UI's camera clamps the same way.
        if self.view.deferred_pan.is_none() {
            self.wanted = self.origin();
        }
        assert_eq!(self.view.scene.surface, surface, "a pan keeps the surface");
    }

    fn origin(&self) -> [f64; 2] {
        self.view.document_view.as_ref().unwrap().geometry.origin
    }
}

/// The regression the user saw: every scroll step rendered every visible page again.
#[test]
fn scrolling_down_and_back_renders_each_tile_once_and_each_page_in_few_bands() {
    let mut harness = Harness::new(3, [1000, 700], 1.5);
    assert!(harness.settle() > 0);
    assert!(harness.published > 0, "the first view is published");
    let opened = harness.published;
    // 20 points per wheel step down to the end of the document, then back up.
    let mut steps = 0;
    loop {
        let before = harness.origin()[1];
        harness.pan(20.0);
        harness.settle();
        steps += 1;
        if harness.origin()[1] == before {
            break;
        }
    }
    assert!(steps > 50, "the scroll crossed every page");
    let rendered = harness.tiles.clone();
    let pages = harness.renderer.pages.clone();
    for _ in 0..steps {
        harness.pan(-20.0);
        harness.settle();
    }
    assert_eq!(harness.origin()[1], 0.0);
    assert!(
        harness.tiles.values().all(|count| *count == 1),
        "a tile was rendered twice: {:?}",
        harness
            .tiles
            .iter()
            .filter(|(_, count)| **count > 1)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        harness.tiles, rendered,
        "scrolling back rendered retained tiles"
    );
    assert_eq!(harness.renderer.pages, pages);
    // A Letter page at 150% is 918 x 1188 pixels: four tile columns, five rows. Bands of
    // up to 16 tiles across and 64 tiles meet each page a few times, not once per step.
    for (page, renders) in &pages {
        assert!(*renders <= 4, "page {page} was rendered {renders} times");
    }
    assert!(
        harness.published - opened >= 2 * steps,
        "every step published a frame: {} for {} steps each way",
        harness.published - opened,
        steps
    );
}

/// A pan over retained tiles of a surface whose source was validated is published at
/// once, while the guard band it lacks renders: neither that band nor the camera's own
/// source check (slow on a network share) holds publication back.
#[test]
fn a_ready_camera_of_a_validated_surface_publishes_at_once() {
    let mut harness = Harness::new(300, [1000, 700], 2.0);
    harness.settle();
    // One tile row down (128 points at 200%): the rows coming into view were guard rows
    // and exist; the new guard row does not.
    harness.pan(128.0);
    assert!(
        harness.view.scene.visible_ready(),
        "the camera moved over retained tiles"
    );
    assert!(harness.view.scene.needs_tiles(), "a guard row is missing");
    let before = harness.published;
    harness.request();
    assert_eq!(harness.published, before + 1, "published at once");
    let Ok(Render::Band { revision, plan }) = harness.requests.try_recv() else {
        panic!("the guard row is prefetched")
    };
    harness.answer(revision, plan);
    harness.settle();
    // A pan that needs no tile: published at once, and the camera's own source check
    // follows while the actor is idle.
    harness.pan(10.0);
    let before = harness.published;
    harness.request();
    assert_eq!(
        harness.published,
        before + 1,
        "published before its source check"
    );
    assert!(matches!(
        harness.requests.try_recv(),
        Ok(Render::Validate(validated)) if validated == harness.view.revision
    ));
}

/// A page only a few tiles wide makes narrow bands; the bands prefetched ahead still stay
/// within 16 tiles down, the provider's 4096-pixel edge, instead of failing the view.
#[test]
fn bands_of_narrow_pages_stay_within_the_provider_edge() {
    // Letter at 100%: 612 x 792 pixels, three tile columns.
    let mut harness = Harness::new(300, [1000, 700], 1.0);
    assert!(harness.settle() > 0);
    for _ in 0..80 {
        harness.pan(64.0);
        harness.settle();
    }
    assert!(
        harness.origin()[1] > 4.0 * 816.0,
        "the scroll passed several pages"
    );
    assert!(harness.view.scene.tiles().is_some(), "the view is complete");
}

/// A band rendered for an older camera fills the newer camera's plan (same canvas and
/// scale, overlapping region) instead of being discarded and rendered again.
#[test]
fn a_band_finished_for_an_older_camera_is_kept_for_the_newer_one() {
    let mut harness = Harness::new(3, [1000, 700], 1.5);
    harness.settle();
    // One tile row down (256 pixels at 150%): the view is ready at once and the guard
    // band of this camera is requested.
    harness.pan(256.0 / 1.5);
    harness.request();
    let Ok(Render::Band { revision, plan }) = harness.requests.try_recv() else {
        panic!("the guard band of the camera is requested")
    };
    let band: Vec<Key> = plan.tiles.iter().map(|(key, _)| *key).collect();
    assert!(!band.is_empty());
    // The camera moves on by a third of a tile while the band renders; its view is
    // covered, so it applies at once.
    harness.pan(30.0);
    assert!(harness.view.revision > revision);
    let before = harness.published;
    harness.answer(revision, plan);
    harness.settle();
    assert!(
        band.iter().all(|key| harness.tiles[key] == 1),
        "a finished band was rendered again for the newer camera"
    );
    assert!(harness.published > before, "the newer camera was published");
}

/// Wheel input every 16 ms keeps arriving while bands render: each completed band still
/// leads to a published frame of the latest camera instead of being superseded.
#[test]
fn continuous_camera_input_never_starves_publication() {
    let mut harness = Harness::new(300, [1000, 700], 2.0);
    harness.settle();
    let mut published_during_input = 0;
    for _ in 0..240 {
        // One pan per display frame, and at most one actor reply in between.
        harness.pan(12.0);
        let before = harness.published;
        harness.request();
        match harness.requests.try_recv() {
            Ok(Render::Band { revision, plan }) => harness.answer(revision, plan),
            Ok(Render::Validate(revision)) => {
                harness.view.pending = None;
                harness.view.validated = Some(revision);
            }
            Ok(Render::Tile { .. }) => panic!("documents render bands"),
            Err(_) => {}
        }
        harness.request();
        published_during_input += usize::from(harness.published > before);
    }
    assert!(
        published_during_input > 120,
        "only {published_during_input} of 240 input frames published a camera"
    );
    // The scroll passed several pages; tiles behind it were evicted within the budget.
    let scene = &harness.view.scene;
    assert!(harness.origin()[1] > 3.0 * 816.0);
    assert!(scene.capacity() >= 128);
}

/// The budget keys tiles by canvas page and scale (a page's rotation is part of the page)
/// and evicts the least recently used ones; the current view is always complete.
#[test]
fn a_long_scroll_stays_within_the_tile_budget_and_reuses_nothing_stale() {
    let mut harness = Harness::new(300, [1000, 700], 2.0);
    harness.settle();
    for _ in 0..400 {
        harness.pan(48.0);
        harness.settle();
        assert!(
            harness.view.scene.tiles().is_some(),
            "the visible tiles exist"
        );
    }
    let rendered_once = harness.tiles.values().all(|count| *count == 1);
    assert!(rendered_once, "a forward scroll never renders a tile twice");
    // Back at the start, the oldest tiles were evicted and are rendered again; nothing
    // from another scale stands in for them.
    harness.pan(-harness.origin()[1]);
    let bands = harness.settle();
    assert!(bands > 0, "evicted tiles render again");
    let scale = harness.view.scene.plan.as_ref().unwrap().scale.to_bits();
    assert!(harness.tiles.keys().all(|key| key.scale == scale));
}

/// A fast scrollbar drag asks for cameras far apart, faster than their tiles render. The
/// engine finishes the camera in progress and publishes it before moving on, so the view
/// shows the pages the drag passes instead of nothing until the drag stops.
#[test]
fn a_fast_drag_publishes_the_cameras_it_passes() {
    // A 2400 x 1600 view at 700% (a page is 4284 pixels wide): its visible tiles take two
    // bands, more than one display frame's worth of rendering.
    let mut harness = Harness::new(300, [2400, 1600], 7.0);
    harness.settle();
    let mut shown = Vec::new();
    for _ in 0..80 {
        // Two pages per display frame: no two targets share a tile.
        harness.pan(1632.0);
        let before = harness.published;
        harness.request();
        match harness.requests.try_recv() {
            Ok(Render::Band { revision, plan }) => harness.answer(revision, plan),
            Ok(Render::Validate(revision)) => {
                harness.view.pending = None;
                harness.view.validated = Some(revision);
            }
            Ok(Render::Tile { .. }) => panic!("documents render bands"),
            Err(_) => {}
        }
        harness.request();
        if harness.published > before {
            shown.push(harness.view.scene.plan.as_ref().unwrap().raster_origin[1]);
        }
    }
    shown.dedup();
    assert!(
        shown.len() >= 8,
        "the drag showed only {} cameras on the way: {shown:?}",
        shown.len()
    );
    assert!(
        shown.windows(2).all(|pair| pair[1] > pair[0]),
        "the shown cameras follow the drag: {shown:?}"
    );
    // Released: the last target is shown once its tiles are ready.
    harness.settle();
    assert_eq!(harness.origin()[1], harness.wanted[1]);
}
