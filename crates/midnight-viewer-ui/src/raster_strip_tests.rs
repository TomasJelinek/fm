//! Prepared text strips in the real window: the camera moves over them frame by frame
//! without waiting for the worker, without new images and without new textures.
use super::image_presentation_tests::{advance_time, present as present_outputs};
use super::*;
use slint::Model as _;
use std::sync::Arc;

const STRIP: u32 = 256;

/// A frame of `count` strips from world y `top`, each a single colour, shown at `origin`.
fn strips(
    open: &OpenWindow,
    revision: u64,
    pixel: u64,
    top: f64,
    count: u32,
    origin: f64,
) -> ViewerOutput {
    let viewport = [
        open.window.get_content_width().round() as u32,
        open.window.get_content_height().round() as u32,
    ];
    let ViewerOutput::Frame(mut frame) = text_presentation_tests::frame(revision, 0, 0.5) else {
        unreachable!()
    };
    frame.mode = 2;
    frame.modes = vec![1, 2];
    frame.width = viewport[0];
    frame.height = count * STRIP;
    frame.pixels = Arc::from([]);
    let list: Arc<[ViewerRasterBand]> = (0..count)
        .map(|index| ViewerRasterBand {
            id: 1000 + (top as u64) / u64::from(STRIP) + u64::from(index),
            top: top + f64::from(index * STRIP),
            height: STRIP,
            pixels: vec![200; viewport[0] as usize * STRIP as usize * 4].into(),
            gutter: None,
        })
        .collect();
    let bottom = top + f64::from(count * STRIP);
    frame.raster = Some(ViewerRasterInfo {
        surface: ViewerSurfaceId(40),
        pixel_id: pixel,
        origin: [0.0, origin],
        raster_origin: [0.0, top],
        viewport,
        coverage_min: [0.0, top],
        coverage_max: [0.0, bottom - f64::from(viewport[1])],
        limit_min: [Some(0.0), Some(0.0)],
        limit_max: [Some(0.0), None],
        pan_sequence: 0,
        scroll_per_pixel: [0.0, 1e-6],
        scroll_map: None,
        settled: true,
        gutter: None,
        bands: Some(ViewerRasterBands {
            width: viewport[0],
            left: 0.0,
            gutter_width: 0,
            list,
        }),
    });
    ViewerOutput::Frame(frame)
}

fn redraw(open: &OpenWindow) {
    advance_time(Duration::from_millis(8));
    image_motion::observe(
        open,
        &slint::winit_030::winit::event::WindowEvent::RedrawRequested,
    );
}

/// Ten wheel notches glide over strips prepared for six screens: every frame moves the
/// camera until the whole distance is shown, on the window's own clock, and no strip
/// becomes a new image. The worker answers nothing meanwhile.
#[test]
fn the_camera_glides_over_prepared_strips_without_new_pixels() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let sent = Rc::new(RefCell::new(Vec::new()));
    let sink = sent.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        sink.borrow_mut().push(input);
        true
    });
    let screen = open.window.get_content_height();
    let count = (6.0 * screen / STRIP as f32).ceil() as u32;
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![strips(&open, 10, 1, 0.0, count, 0.0)],
    );
    assert!(open.window.get_retained_raster());
    assert_eq!(open.window.get_raster_bands().row_count(), count as usize);
    let uploads = open.window.get_raster_upload_count();
    assert_eq!(uploads, count as i32, "each strip became one image");
    let scale = f64::from(open.window.get_content_height().round())
        / f64::from(open.window.get_raster_viewport_height());
    let notch = image_motion::notch_pixels(&open, false);
    for _ in 0..10 {
        assert!(raster_motion::push(&open, 0.0, notch));
    }
    let mut previous = open.retained_raster.borrow().camera.origin[1];
    let mut frames = 0;
    while open.retained_raster.borrow().camera.active() && frames < 200 {
        redraw(&open);
        frames += 1;
        let origin = open.retained_raster.borrow().camera.origin[1];
        assert!(
            origin > previous || !open.retained_raster.borrow().camera.active(),
            "frame {frames} waited at {origin}"
        );
        previous = origin;
    }
    let shown = open.retained_raster.borrow().camera.origin[1];
    assert_eq!(
        shown,
        (f64::from(10.0 * notch) * scale).round(),
        "the whole glide is shown"
    );
    assert_eq!(shown.fract(), 0.0, "the camera rests on a whole pixel");
    assert_eq!(
        open.window.get_raster_upload_count(),
        uploads,
        "no new image"
    );
    // The camera asked the worker for its positions; none of them was needed to move.
    assert!(
        sent.borrow()
            .iter()
            .any(|input| matches!(input, ViewerInput::Raster { .. }))
    );
    manager.close_all();
}

/// A frame that adds strips ahead keeps the images of the strips it still shows: only the
/// new strips become images.
#[test]
fn new_strips_ahead_become_images_and_shown_ones_keep_theirs() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    *open.input.borrow_mut() = Rc::new(|_| true);
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![strips(&open, 10, 1, 0.0, 6, 0.0)],
    );
    let first = open.window.get_raster_upload_count();
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![strips(&open, 11, 2, 0.0, 9, 0.0)],
    );
    assert_eq!(open.window.get_raster_upload_count(), first + 3);
    assert_eq!(open.window.get_raster_bands().row_count(), 9);
    // Below the first strip, a frame that starts a strip further down drops that strip
    // and keeps the images of the eight it still shows.
    assert!(raster_presentation::push(&open, 0.0, 300.0, true));
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![strips(&open, 12, 3, 256.0, 8, 0.0)],
    );
    assert_eq!(open.window.get_raster_upload_count(), first + 3);
    assert_eq!(open.window.get_raster_bands().row_count(), 8);
    manager.close_all();
}

/// Strips that stay shown keep their Slint rows while strips leave and arrive and the page
/// origin moves: the renderer uploads a row's texture again whenever that row changes, and
/// every row's when the model is replaced.
#[test]
fn shown_strips_keep_their_rows_while_strips_come_and_go() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    *open.input.borrow_mut() = Rc::new(|_| true);
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![strips(&open, 10, 1, 0.0, 6, 0.0)],
    );
    let model = open.window.get_raster_bands();
    let rows: Vec<_> = model.iter().collect();
    // Two strips further down: the first two leave, two arrive, and the page origin moves
    // to the new first strip.
    assert!(raster_presentation::push(&open, 0.0, 600.0, true));
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![strips(&open, 11, 2, 512.0, 6, 0.0)],
    );
    let shown = open.window.get_raster_bands();
    assert!(shown == model, "the window keeps its model");
    assert_eq!(shown.row_count(), 6);
    assert_eq!(
        shown.iter().take(4).collect::<Vec<_>>(),
        rows[2..],
        "the four strips still shown keep their rows"
    );
    assert_eq!(open.window.get_raster_upload_count(), 8, "two new images");
    // Each row lies at its strip's height below the page origin.
    let viewport = open.window.get_content_height().round();
    let origin = open.window.get_raster_bands_origin();
    let mut tops: Vec<_> = shown
        .iter()
        .map(|row| (origin + row.y) * viewport)
        .collect();
    tops.sort_by(f32::total_cmp);
    for (index, top) in tops.into_iter().enumerate() {
        assert!(
            (top - index as f32 * STRIP as f32).abs() < 1e-3,
            "row {index} at {top}"
        );
    }
    manager.close_all();
}
