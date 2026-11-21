//! One band of document tiles: page slices, gaps, bounds and failures.
use super::*;
use crate::{
    document_view::DocumentView,
    image_scene::{Band, Scene},
    image_tiles::{CANVAS, Tiles},
};
use midnight_core::DocumentLayout;
use midnight_image_viewer_host::{Frame, Metadata, Viewport};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// A 400 x 600 page centred over a 1000 x 400 page, 24 units apart, at actual size.
fn boundary() -> DocumentView {
    let mut view = DocumentView::new(
        &[
            Metadata {
                width: 400,
                height: 600,
            },
            Metadata {
                width: 1000,
                height: 400,
            },
        ],
        [400, 200],
    )
    .unwrap();
    view.geometry.actual();
    view.painted(1);
    view.set_layout(1, DocumentLayout::Continuous);
    view.pan_to([300.0, 550.0]);
    view
}

#[derive(Default)]
struct Renderer {
    cancel: bool,
    reject_source: bool,
    malformed: bool,
    requests: Vec<(usize, Viewport, Duration)>,
    validations: usize,
}

impl PageRenderer for Renderer {
    fn render_page(
        &mut self,
        page: usize,
        viewport: Viewport,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Frame, String> {
        assert!(
            !cancel.load(Ordering::Acquire),
            "a band never renders for a closed session"
        );
        self.requests.push((page, viewport, timeout));
        if self.reject_source {
            return Err("source changed".into());
        }
        let color = if page == 0 {
            [255, 0, 0, 255]
        } else {
            [0, 0, 255, 255]
        };
        Ok(Frame {
            width: viewport.width + u32::from(self.malformed),
            height: viewport.height,
            rgba: color.repeat(viewport.width as usize * viewport.height as usize),
        })
    }

    fn validate(&mut self, _: &AtomicBool) -> Result<(), String> {
        self.validations += 1;
        Ok(())
    }
}

/// Columns 0..4 and row 2 of the canvas at scale 1: rows 512..768 meet the end of the
/// first page (to 600), the gap and the top of the second page (from 624).
fn band(rect: [i32; 4], tiles: &Tiles) -> Band {
    Band {
        key: CANVAS,
        scale: 1.0,
        rect,
        tiles: (rect[1]..rect[3])
            .flat_map(|y| (rect[0]..rect[2]).map(move |x| (x, y)))
            .map(|(x, y)| {
                (
                    Key {
                        page: CANVAS,
                        scale: 1.0f64.to_bits(),
                        x,
                        y,
                    },
                    tiles.reserve().unwrap(),
                )
            })
            .collect(),
    }
}

fn pixel(tiles: &[(Key, Arc<ViewerPixels>)], x: i32, y: i32, at: [usize; 2]) -> [u8; 4] {
    let (_, pixels) = tiles
        .iter()
        .find(|(key, _)| key.x == x && key.y == y)
        .unwrap();
    let index = (at[1] * 256 + at[0]) * 4;
    pixels.rgba()[index..index + 4].try_into().unwrap()
}

#[test]
fn a_band_renders_each_page_it_meets_once_and_leaves_the_gap_transparent() {
    let tiles = Tiles::new(Arc::new(AtomicU64::new(0)));
    let plan = BandPlan::new(&boundary(), band([0, 2, 4, 3], &tiles)).unwrap();
    let cancel = AtomicBool::new(false);
    let mut renderer = Renderer::default();
    let tiles = render_band(&mut renderer, plan, &cancel, Duration::from_secs(15)).unwrap();
    assert_eq!(tiles.len(), 4);
    assert_eq!(
        renderer
            .requests
            .iter()
            .map(|(page, viewport, _)| (*page, *viewport))
            .collect::<Vec<_>>(),
        [
            (
                0,
                Viewport {
                    width: 400,
                    height: 88,
                    scale: 1.0,
                    origin_x: 0.0,
                    origin_y: 512.0
                }
            ),
            (
                1,
                Viewport {
                    width: 1000,
                    height: 144,
                    scale: 1.0,
                    origin_x: 0.0,
                    origin_y: 0.0
                }
            )
        ]
    );
    // Column 1 spans x 256..512: the first page starts at 300.
    assert_eq!(pixel(&tiles, 1, 2, [10, 10]), [0; 4]);
    assert_eq!(pixel(&tiles, 1, 2, [60, 10]), [255, 0, 0, 255]);
    assert_eq!(pixel(&tiles, 1, 2, [60, 87]), [255, 0, 0, 255]);
    assert_eq!(pixel(&tiles, 1, 2, [60, 100]), [0; 4]);
    assert_eq!(pixel(&tiles, 1, 2, [60, 112]), [0, 0, 255, 255]);
    assert_eq!(pixel(&tiles, 3, 2, [200, 255]), [0, 0, 255, 255]);
    // Column 3 spans x 768..1024: the second page ends at 1000.
    assert_eq!(pixel(&tiles, 3, 2, [240, 255]), [0; 4]);
    // The provider validated the source around each slice; no extra round trip.
    assert_eq!(renderer.validations, 0);
    assert!(renderer.requests[1].2 <= renderer.requests[0].2);
}

#[test]
fn a_band_without_page_slices_still_proves_the_source_current() {
    let tiles = Tiles::new(Arc::new(AtomicU64::new(0)));
    // Row 0 beside the first page (x 0..256 at y 0..256) meets no page.
    let plan = BandPlan::new(&boundary(), band([0, 0, 1, 1], &tiles)).unwrap();
    assert!(plan.slices.is_empty());
    let mut renderer = Renderer::default();
    let tiles = render_band(
        &mut renderer,
        plan,
        &AtomicBool::new(false),
        Duration::from_secs(15),
    )
    .unwrap();
    assert_eq!(renderer.validations, 1);
    assert!(tiles[0].1.rgba().iter().all(|byte| *byte == 0));
}

#[test]
fn a_source_change_or_a_malformed_page_reply_fails_the_band() {
    for malformed in [false, true] {
        let tiles = Tiles::new(Arc::new(AtomicU64::new(0)));
        let plan = BandPlan::new(&boundary(), band([0, 2, 4, 3], &tiles)).unwrap();
        let mut renderer = Renderer {
            reject_source: !malformed,
            malformed,
            ..Default::default()
        };
        assert!(
            render_band(
                &mut renderer,
                plan,
                &AtomicBool::new(false),
                Duration::from_secs(15)
            )
            .is_err()
        );
        assert_eq!(renderer.requests.len(), 1, "the band stops at the failure");
        // The failed band's reservations are released with its plan.
        assert_eq!(tiles.reserve().map(drop), Some(()));
    }
    let cancelled = AtomicBool::new(true);
    let tiles = Tiles::new(Arc::new(AtomicU64::new(0)));
    let plan = BandPlan::new(&boundary(), band([0, 2, 4, 3], &tiles)).unwrap();
    let mut renderer = Renderer {
        cancel: true,
        ..Default::default()
    };
    assert!(renderer.cancel);
    assert!(render_band(&mut renderer, plan, &cancelled, Duration::ZERO).is_err());
    assert!(renderer.requests.is_empty(), "expired work renders nothing");
}

#[test]
fn bands_of_large_dpi_views_stay_within_the_provider_viewport_bounds() {
    let mut view = boundary();
    view.resize([7680, 4320]);
    view.geometry.scale = 6.0;
    view.pan_to([0.0, 0.0]);
    let mut scene = Scene::new(Arc::new(AtomicU64::new(0)));
    scene
        .update_document(&view.tile_canvas(), &view.geometry, true)
        .unwrap();
    let plan = scene.plan.as_ref().unwrap();
    assert_eq!(plan.raster_size, [7680, 4320], "exact resolution at 8K");
    assert!(
        plan.keys().count() <= scene.capacity(),
        "the budget holds the view"
    );
    // The first bands of the view (each 16 x 4 tiles at most) fit the provider's bounds.
    for _ in 0..3 {
        let band = scene.next_band().unwrap().unwrap();
        let plan = BandPlan::new(&view, band).unwrap();
        assert!(plan.size[0] <= 4096 && plan.size[1] <= 4096);
        assert!(u64::from(plan.size[0]) * u64::from(plan.size[1]) <= 4_194_304);
        for slice in &plan.slices {
            slice.viewport.validate().unwrap();
        }
        let tiles = plan
            .tiles
            .into_iter()
            .map(|(key, charge)| {
                (
                    key,
                    Arc::new(ViewerPixels::new(vec![255; TILE_BYTES], charge).unwrap()),
                )
            })
            .collect();
        scene.complete_band(tiles).unwrap();
    }
}

#[test]
fn tiny_pages_are_never_silently_dropped() {
    let pages = vec![
        Metadata {
            width: 1,
            height: 1
        };
        4096
    ];
    let mut view = DocumentView::new(&pages, [1000, 1000]).unwrap();
    view.painted(1);
    view.set_layout(1, DocumentLayout::Continuous);
    view.geometry.scale = 1.0 / 128.0;
    view.pan_to([0.0, 0.0]);
    let tiles = Tiles::new(Arc::new(AtomicU64::new(0)));
    // 4096 pages of one unit, 24 units apart, take 4096 * 25 / 128 = 800 pixels: a band
    // of four tile rows meets every one of them, and each gets its own pixel.
    let mut band = band([0, 0, 1, 4], &tiles);
    band.scale = view.geometry.scale;
    for (key, _) in &mut band.tiles {
        key.scale = band.scale.to_bits();
    }
    let plan = BandPlan::new(&view, band).unwrap();
    assert_eq!(plan.slices.len(), 4096);
    assert!(plan.slices.iter().all(|slice| slice.viewport.height >= 1));
}
