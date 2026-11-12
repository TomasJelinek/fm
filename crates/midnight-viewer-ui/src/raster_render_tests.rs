//! Render the real Slint body/gutter arrangement; no font or foreground thresholds.
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
