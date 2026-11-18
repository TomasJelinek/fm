//! Render the real Slint body/gutter arrangement; no font or foreground thresholds.
use super::image_presentation_tests::present as present_outputs;
use super::*;
use slint::{Rgb8Pixel, platform::WindowEvent};

const VIEW: [u32; 2] = [700, 420];
const GUTTER: u32 = 35;
const BODY: [u32; 2] = [805, 560];
const BODY_BG: [u8; 3] = [15, 80, 30];
const GUTTER_BG: [u8; 3] = [40, 10, 90];
const BODY_MARK: [u8; 3] = [250, 30, 20];
const GUTTER_MARK: [u8; 3] = [20, 230, 245];
const BAND: [u8; 3] = [45, 70, 240];
const COLORS: [[u8; 3]; 5] = [BODY_BG, GUTTER_BG, BODY_MARK, GUTTER_MARK, BAND];

fn pixels(width: u32, height: u32, gutter: bool) -> std::sync::Arc<[u8]> {
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            let rgb = if (210..238).contains(&y)
                && if gutter {
                    (7..21).contains(&x)
                } else {
                    (210..238).contains(&x)
                } {
                if gutter { GUTTER_MARK } else { BODY_MARK }
            } else if (350..378).contains(&y) {
                BAND
            } else if gutter {
                GUTTER_BG
            } else {
                BODY_BG
            };
            pixels.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
    pixels.into()
}

fn frame(revision: u64) -> Box<ViewerFrame> {
    let ViewerOutput::Frame(mut frame) = text_presentation_tests::frame(revision, 0, 0.2) else {
        unreachable!()
    };
    frame.mode = 2;
    frame.modes = vec![1, 2];
    frame.width = BODY[0];
    frame.height = BODY[1];
    frame.pixels = pixels(BODY[0], BODY[1], false);
    frame.raster = Some(ViewerRasterInfo {
        surface: ViewerSurfaceId(910 + revision),
        pixel_id: 1,
        origin: [70.25, 70.375],
        raster_origin: [0.25, 0.375],
        viewport: VIEW,
        coverage_min: [0.25, 0.375],
        coverage_max: [140.25, 140.375],
        limit_min: [Some(0.0); 2],
        limit_max: [None; 2],
        pan_sequence: 0,
        scroll_per_pixel: [0.001; 2],
        scroll_map: None,
        settled: true,
        gutter: Some(ViewerRasterGutter {
            width: GUTTER,
            height: BODY[1],
            pixels: pixels(GUTTER, BODY[1], true),
        }),
    });
    frame
}

fn prepare(open: &OpenWindow, service: &Service, scale: f32) {
    open.timer.stop();
    open.image_redraw.stop();
    *open.input.borrow_mut() = Rc::new(|_| true);
    let logical = open
        .window
        .window()
        .size()
        .to_logical(open.window.window().scale_factor());
    open.window
        .window()
        .dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: scale,
        });
    open.window.window().set_size(logical);
    present_outputs(
        open,
        service,
        &|_| {},
        vec![ViewerOutput::Frame(frame(open.frame.get() + 1))],
    );
    // Mirror the runtime resize timer, which is stopped to exclude backend delivery.
    open.window.set_compact(true);
    open.window.set_narrow_typography(true);
    // Draw before measuring: fonts and conditional toolbar rows participate in layout.
    Bitmap::capture();
    // Keep the effective raster scale at 1.75 even when native DPI differs.
    for _ in 0..3 {
        let size = open.window.window().size().to_logical(scale);
        open.window.window().set_size(slint::LogicalSize::new(
            size.width + 400.0 - open.window.get_content_width(),
            size.height + 240.0 - open.window.get_content_height(),
        ));
        Bitmap::capture();
    }
    assert!((open.window.get_content_width() - 400.0).abs() < 0.01);
    assert!((open.window.get_content_height() - 240.0).abs() < 0.01);
    // Refresh the effective viewport ratio after the deliberately synthetic resize.
    present_outputs(
        open,
        service,
        &|_| {},
        vec![ViewerOutput::Frame(frame(open.frame.get() + 1))],
    );
}

struct Bitmap {
    width: usize,
    height: usize,
    pixels: Vec<Rgb8Pixel>,
}
impl Bitmap {
    fn capture() -> Self {
        let (size, pixels) = image_presentation_tests::bitmap();
        let result = Self {
            width: size.width as usize,
            height: size.height as usize,
            pixels,
        };
        if let Some(directory) = std::env::var_os("MIDNIGHT_RETAINED_RENDER_DIR") {
            use std::io::Write;
            static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let index = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            let mut output =
                std::fs::File::create(directory.join(format!("raster-{index}.ppm"))).unwrap();
            write!(output, "P6\n{} {}\n255\n", result.width, result.height).unwrap();
            for pixel in &result.pixels {
                output.write_all(&[pixel.r, pixel.g, pixel.b]).unwrap();
            }
        }
        result
    }
    fn rgb(&self, x: usize, y: usize) -> [u8; 3] {
        let p = self.pixels[y * self.width + x];
        [p.r, p.g, p.b]
    }
    fn bounds(&self, color: [u8; 3]) -> [i32; 4] {
        let mut bounds = [i32::MAX, i32::MAX, -1, -1];
        let mut count = 0;
        for y in 0..self.height {
            for x in 0..self.width {
                if self.rgb(x, y) == color {
                    bounds[0] = bounds[0].min(x as i32);
                    bounds[1] = bounds[1].min(y as i32);
                    bounds[2] = bounds[2].max(x as i32);
                    bounds[3] = bounds[3].max(y as i32);
                    count += 1;
                }
            }
        }
        assert!(
            count >= 20,
            "missing rendered marker {color:?}: {count} pixels"
        );
        bounds
    }
    /// The physical `[x0, y0, x1, y1)` region shows a one-pixel checkerboard of
    /// `colors`: a resampled texel blends colors, a stretched one repeats. Full
    /// rows and columns are scanned at a stride; every repeated column crosses
    /// each scanned row and every repeated row each scanned column.
    fn assert_checker(&self, region: [usize; 4], colors: [[u8; 3]; 2], what: &str) {
        const STRIDE: usize = 23;
        assert!(
            region[0] + 16 < region[2] && region[1] + 16 < region[3],
            "{what}: region {region:?} is too small to check"
        );
        let check = |x: usize, y: usize, next: [usize; 2], direction: &str| {
            let rgb = self.rgb(x, y);
            assert!(
                colors.contains(&rgb),
                "{what}: resampled texel {rgb:?} at {x},{y}"
            );
            assert!(
                next[0] == region[2] || next[1] == region[3] || rgb != self.rgb(next[0], next[1]),
                "{what}: repeated {direction} at {x},{y}"
            );
        };
        for y in (region[1]..region[3]).step_by(STRIDE) {
            for x in region[0]..region[2] {
                check(x, y, [x + 1, y], "column");
            }
        }
        for x in (region[0]..region[2]).step_by(STRIDE) {
            for y in region[1]..region[3] {
                check(x, y, [x, y + 1], "row");
            }
        }
    }
    fn assert_clipped(&self, open: &OpenWindow, scale: f32) {
        let left = (open.window.get_content_left() * scale).floor() as usize;
        let top = (open.window.get_content_top() * scale).floor() as usize;
        let right = ((open.window.get_content_left() + open.window.get_content_width()) * scale)
            .ceil() as usize;
        let bottom = ((open.window.get_content_top() + open.window.get_content_height()) * scale)
            .ceil() as usize;
        for y in 0..self.height {
            for x in 0..self.width {
                if COLORS.contains(&self.rgb(x, y)) {
                    assert!(
                        (left..right).contains(&x) && (top..bottom).contains(&y),
                        "retained overscan escaped canvas at {x},{y}"
                    );
                }
            }
        }
    }
}

#[test]
fn retained_rendered_body_moves_beside_fixed_gutter_and_both_share_vertical_motion() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    for scale in [1.0, 1.5] {
        prepare(&open, &service, scale);
        let initial = Bitmap::capture();
        let body = initial.bounds(BODY_MARK);
        let gutter = initial.bounds(GUTTER_MARK);
        let left = open.window.get_content_left() * scale;
        let gutter_right = left + GUTTER as f32 / 1.75 * scale;
        assert!(
            gutter[0] >= left.floor() as i32 && gutter[2] < gutter_right.ceil() as i32,
            "gutter marker must be in the left strip, not implicitly centered: {gutter:?}"
        );
        assert!(body[0] >= gutter_right.floor() as i32);
        assert_eq!([body[1], body[3]], [gutter[1], gutter[3]]);
        let uploads = open.window.get_raster_upload_count();
        open.window.invoke_scroll(8.0, 0.0);
        let horizontal = Bitmap::capture();
        assert_eq!(horizontal.bounds(GUTTER_MARK), gutter);
        let moved = horizontal.bounds(BODY_MARK);
        let distance = (8.0 * scale) as i32;
        assert_eq!(
            moved,
            [body[0] - distance, body[1], body[2] - distance, body[3]]
        );
        open.window.invoke_scroll(0.0, 8.0);
        let vertical = Bitmap::capture();
        assert_eq!(
            vertical.bounds(BODY_MARK),
            [moved[0], moved[1] - distance, moved[2], moved[3] - distance]
        );
        assert_eq!(
            vertical.bounds(GUTTER_MARK),
            [
                gutter[0],
                gutter[1] - distance,
                gutter[2],
                gutter[3] - distance
            ]
        );
        for image in [&initial, &horizontal, &vertical] {
            image.assert_clipped(&open, scale);
        }
        assert_eq!(
            open.window.get_raster_upload_count(),
            uploads,
            "camera movement must reuse the same uploaded pixels"
        );
    }
    manager.close_all();
}

/// The retained fixture with its guards around a `viewport` of physical pixels.
fn sized(revision: u64, viewport: [u32; 2]) -> Box<ViewerFrame> {
    let mut frame = frame(revision);
    let body = viewport.map(|extent| extent + 105);
    frame.width = body[0];
    frame.height = body[1];
    frame.pixels = pixels(body[0], body[1], false);
    let info = frame.raster.as_mut().unwrap();
    info.viewport = viewport;
    let gutter = info.gutter.as_mut().unwrap();
    gutter.height = body[1];
    gutter.pixels = pixels(GUTTER, body[1], true);
    frame
}

#[test]
fn native_rasters_map_each_texel_to_one_physical_pixel_on_fractional_canvases() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    *open.input.borrow_mut() = Rc::new(|_| true);
    let content = || {
        [
            open.window.get_content_width(),
            open.window.get_content_height(),
        ]
    };
    let fractional = |extent: f32| ((extent - extent.round()).abs()) > 0.01;
    // Chrome makes the canvas a fraction of a physical pixel at these scales.
    let logical = open
        .window
        .window()
        .size()
        .to_logical(open.window.window().scale_factor());
    let scale = [1.1_f32, 1.3, 1.15, 1.7]
        .into_iter()
        .find(|&scale| {
            open.window
                .window()
                .dispatch_event(WindowEvent::ScaleFactorChanged {
                    scale_factor: scale,
                });
            open.window.window().set_size(logical);
            Bitmap::capture();
            content().iter().all(|extent| fractional(extent * scale))
        })
        .expect("a scale with a fractional physical canvas");
    let canvas = content();
    let geometry = window_geometry(&open.window);
    let native = [geometry.width, geometry.height];
    assert_eq!(native, canvas.map(|extent| (extent * scale).round() as u32));
    let mapped = || {
        [
            open.window.get_raster_viewport_width() * scale,
            open.window.get_raster_viewport_height() * scale,
        ]
    };
    let one_to_one =
        |mapped: [f32; 2]| (0..2).all(|axis| (mapped[axis] - native[axis] as f32).abs() < 1e-3);

    // Retained text at native resolution: the canvas clips the sub-pixel rest.
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(sized(open.frame.get() + 1, native))],
    );
    assert!(open.window.get_retained_raster());
    assert!(
        one_to_one(mapped()),
        "{:?} stretched over {native:?}",
        mapped()
    );
    // A budget-reduced raster still covers the whole canvas.
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(sized(
            open.frame.get() + 1,
            native.map(|extent| extent / 2),
        ))],
    );
    assert_eq!(
        [
            open.window.get_raster_viewport_width(),
            open.window.get_raster_viewport_height()
        ],
        canvas
    );
    // Viewport-only (legacy and HTML) frames use the same native mapping.
    let ViewerOutput::Frame(mut legacy) =
        text_presentation_tests::frame(open.frame.get() + 1, 0, 0.2)
    else {
        unreachable!()
    };
    legacy.generation = 8;
    legacy.width = native[0];
    legacy.height = native[1];
    legacy.pixels = vec![255; native[0] as usize * native[1] as usize * 4].into();
    present_outputs(&open, &service, &|_| {}, vec![ViewerOutput::Frame(legacy)]);
    assert!(!open.window.get_retained_raster());
    assert!(
        one_to_one(mapped()),
        "{:?} stretched over {native:?}",
        mapped()
    );
    manager.close_all();
}

const CHECKER_BODY: [[u8; 3]; 2] = [[200, 40, 40], [40, 200, 220]];
const CHECKER_GUTTER: [[u8; 3]; 2] = [[250, 250, 60], [60, 60, 250]];

fn checker(width: u32, height: u32, colors: [[u8; 3]; 2]) -> std::sync::Arc<[u8]> {
    let row = |phase: usize| -> Vec<u8> {
        (0..width as usize)
            .flat_map(|x| {
                let [r, g, b] = colors[(x + phase) % 2];
                [r, g, b, 255]
            })
            .collect()
    };
    let rows = [row(0), row(1)];
    let mut pixels = Vec::with_capacity(rows[0].len() * height as usize);
    for y in 0..height as usize {
        pixels.extend_from_slice(&rows[y % 2]);
    }
    pixels.into()
}

/// Rendered pixels, not only the mapping: no texel is resampled or repeated
/// although the canvas origin falls between physical pixels (the 6 px padding
/// is 6.6 px at 110% and exactly half a pixel at 125% and 175%) and the canvas
/// size can be fractional. The software renderer cannot show a stretch below
/// half a pixel (the mapping test above covers it); `examples/texel_alignment.rs`
/// repeats this check natively, also with FemtoVG.
#[test]
fn native_rasters_draw_one_pixel_checkerboards_exactly_at_fractional_canvas_origins() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    *open.input.borrow_mut() = Rc::new(|_| true);
    // Enter the retained text layout first: its controls decide the canvas.
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(frame(open.frame.get() + 1))],
    );
    let logical = open
        .window
        .window()
        .size()
        .to_logical(open.window.window().scale_factor());
    for (index, scale) in [1.1_f32, 1.25, 1.75].into_iter().enumerate() {
        open.window
            .window()
            .dispatch_event(WindowEvent::ScaleFactorChanged {
                scale_factor: scale,
            });
        open.window.window().set_size(logical);
        Bitmap::capture();
        let [left, top, width, height] = [
            open.window.get_content_left(),
            open.window.get_content_top(),
            open.window.get_content_width(),
            open.window.get_content_height(),
        ]
        .map(|value| value * scale);
        assert!(
            (left - left.round()).abs() > 0.01,
            "{scale}: the canvas origin must fall between physical pixels"
        );
        let geometry = window_geometry(&open.window);
        let native = [geometry.width, geometry.height];
        // Inner physical pixels of the canvas; partially covered edges are clipped.
        let inner = [
            left.ceil() as usize + 1,
            top.ceil() as usize + 1,
            (left + width).floor() as usize - 1,
            (top + height).floor() as usize - 1,
        ];
        let mut generation = 10 + 100 * index as u64;

        // Guard bands start the retained images far above and left of the canvas,
        // at offsets whose logical value is inexact. A half-pixel canvas origin
        // must not let rounding grow such an image by a row or column.
        for guard in [64_u32, 100, 128, 200, 256, 300] {
            generation += 1;
            let body = native.map(|extent| extent + 2 * guard);
            let mut retained = frame(open.frame.get() + 1);
            retained.generation = generation;
            retained.width = body[0];
            retained.height = body[1];
            retained.pixels = checker(body[0], body[1], CHECKER_BODY);
            let info = retained.raster.as_mut().unwrap();
            info.viewport = native;
            info.origin = info.raster_origin.map(|edge| edge + f64::from(guard));
            info.coverage_max = info.raster_origin.map(|edge| edge + f64::from(2 * guard));
            let gutter = info.gutter.as_mut().unwrap();
            gutter.height = body[1];
            gutter.pixels = checker(GUTTER, body[1], CHECKER_GUTTER);
            present_outputs(
                &open,
                &service,
                &|_| {},
                vec![ViewerOutput::Frame(retained)],
            );
            assert!(open.window.get_retained_raster());
            let image = Bitmap::capture();
            let split = left + GUTTER as f32;
            image.assert_checker(
                [inner[0], inner[1], split.floor() as usize - 1, inner[3]],
                CHECKER_GUTTER,
                &format!("{scale} guard {guard}: retained gutter"),
            );
            image.assert_checker(
                [split.ceil() as usize + 1, inner[1], inner[2], inner[3]],
                CHECKER_BODY,
                &format!("{scale} guard {guard}: retained body"),
            );
        }

        let ViewerOutput::Frame(mut legacy) =
            text_presentation_tests::frame(open.frame.get() + 1, 0, 0.2)
        else {
            unreachable!()
        };
        legacy.generation = generation + 1;
        // The same mode keeps the same controls, and therefore the same canvas.
        legacy.mode = 2;
        legacy.modes = vec![1, 2];
        legacy.width = native[0];
        legacy.height = native[1];
        legacy.pixels = checker(native[0], native[1], CHECKER_BODY);
        present_outputs(&open, &service, &|_| {}, vec![ViewerOutput::Frame(legacy)]);
        assert!(!open.window.get_retained_raster());
        Bitmap::capture().assert_checker(
            inner,
            CHECKER_BODY,
            &format!("{scale}: viewport-only frame"),
        );
    }
    manager.close_all();
}

#[test]
fn retained_rendered_settled_crop_equals_the_viewport_only_reference() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    prepare(&open, &service, 1.0);
    open.window.invoke_scroll(8.0, 8.0);
    let retained = Bitmap::capture();
    let mut reference = frame(open.frame.get() + 1);
    let info = reference.raster.take().unwrap();
    let gutter = info.gutter.unwrap();
    let origin = open.retained_raster.borrow().camera.origin;
    let offset =
        std::array::from_fn::<_, 2, _>(|axis| (origin[axis] - info.raster_origin[axis]) as usize);
    assert_eq!(offset, [84, 84]);
    let mut crop = Vec::with_capacity(VIEW[0] as usize * VIEW[1] as usize * 4);
    for y in 0..VIEW[1] as usize {
        let row = y + offset[1];
        let gutter_start = row * GUTTER as usize * 4;
        crop.extend_from_slice(&gutter.pixels[gutter_start..gutter_start + GUTTER as usize * 4]);
        let start = (row * BODY[0] as usize + offset[0]) * 4;
        crop.extend_from_slice(&reference.pixels[start..start + (VIEW[0] - GUTTER) as usize * 4]);
    }
    reference.width = VIEW[0];
    reference.height = VIEW[1];
    reference.pixels = crop.into();
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(reference)],
    );
    let viewport = Bitmap::capture();
    let left = open.window.get_content_left() as usize;
    let top = open.window.get_content_top() as usize;
    for y in top..top + 240 {
        for x in left..left + 400 {
            assert_eq!(
                retained.rgb(x, y),
                viewport.rgb(x, y),
                "settled crop differs at canvas pixel {},{}",
                x - left,
                y - top
            );
        }
    }
    manager.close_all();
}
