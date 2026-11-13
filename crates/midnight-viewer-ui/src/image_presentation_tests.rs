use super::*;
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::sync::Arc;

thread_local! {
    static TEST_TIME: Cell<Duration> = const { Cell::new(Duration::ZERO) };
    static TEST_WINDOW: RefCell<std::rc::Weak<MinimalSoftwareWindow>> = const { RefCell::new(std::rc::Weak::new()) };
}
pub(super) fn advance_time(time: Duration) {
    TEST_TIME.with(|clock| clock.set(clock.get() + time));
}
struct TestPlatform;
impl Platform for TestPlatform {
    fn duration_since_start(&self) -> Duration {
        TEST_TIME.with(Cell::get)
    }
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        TEST_WINDOW.with(|slot| *slot.borrow_mut() = Rc::downgrade(&window));
        Ok(window)
    }
}

/// Full software framebuffer from the same adapter that presents the viewer.
pub(super) fn bitmap() -> (slint::PhysicalSize, Vec<slint::Rgb8Pixel>) {
    let window = TEST_WINDOW.with(|slot| slot.borrow().upgrade().expect("viewer adapter"));
    let size = window.window().size();
    assert!(size.width > 0 && size.height > 0);
    let mut pixels = vec![slint::Rgb8Pixel::default(); size.width as usize * size.height as usize];
    window.window().request_redraw();
    assert!(window.draw_if_needed(|renderer| {
        renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
        renderer.render(&mut pixels, size.width as usize);
        renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
    }));
    (size, pixels)
}

#[derive(Debug)]
struct Pixels;
impl ViewerPixelOwner for Pixels {}

fn tile(id: u64, x: f32, y: f32) -> ViewerTile {
    ViewerTile {
        id: ViewerTileId(id),
        position: [x, y],
        width: 256,
        height: 256,
        pixels: Arc::new(ViewerPixels::new(vec![255; 256 * 256 * 4], Arc::new(Pixels)).unwrap()),
    }
}
fn info() -> ViewerImageInfo {
    ViewerImageInfo {
        surface: ViewerSurfaceId(20),
        origin: [512.0; 2],
        visible_source: [400.0, 240.0],
        raster_scale: 1.0,
        pan_sequence: 0,
        width: 4000,
        height: 3000,
        zoom: 1.0,
        fitted: false,
        page: 0,
        page_count: 1,
    }
}
fn frame() -> ViewerFrame {
    ViewerFrame {
        generation: 1,
        raster: None,
        raster_rebase: None,
        position_sequence: [0; 2],
        revision: 1,
        mode: VIEWER_MODE_IMAGE,
        modes: vec![VIEWER_MODE_IMAGE],
        wrap_enabled: false,
        wrap_available: false,
        image: Some(info()),
        width: 400,
        height: 240,
        pixels: Arc::from([]),
        tiles: Some(
            vec![
                tile(1, -256.0, -256.0),
                tile(2, 0.0, -256.0),
                tile(3, 256.0, -256.0),
                tile(4, -256.0, 0.0),
                tile(5, 0.0, 0.0),
                tile(6, 256.0, 0.0),
                tile(7, -256.0, 256.0),
                tile(8, 0.0, 256.0),
                tile(9, 256.0, 256.0),
            ]
            .into(),
        ),
        accessible_text: "Synthetic image".into(),
        position: 0.2,
        vertical_extent: 0.1,
        horizontal_position: 0.2,
        horizontal_extent: 0.1,
        caption: "Test image".into(),
        status: "Test".into(),
    }
}

pub(super) fn open() -> (
    tokio::runtime::Runtime,
    ViewerWindows,
    Rc<OpenWindow>,
    Service,
) {
    slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let service = Service::with_native_capture(
        runtime.handle().clone(),
        Arc::new(|_, _| Err("synthetic fixture".into())),
    );
    let manager = ViewerWindows::new(service.clone());
    let path = std::env::temp_dir().join("synthetic-image.bmp");
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
    manager
        .open(
            ViewerLaunch {
                find: None,
                associations: Default::default(),
                tree: None,
                generated: None,
                preferences: Arc::new(Default::default()),
                archive: None,
                view: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
                focused: path,
                basic: false,
                disabled_plugins: Arc::from([]),
            },
            ViewerAppearance {
                family: "monospace".into(),
                font_size: 14.0,
                foreground: [255; 4],
                background: [0, 0, 0, 255],
                selection: [55; 4],
                accent: [150; 4],
            },
            slint::LogicalSize::new(900.0, 600.0),
            Rc::new(|_, _| None),
            Rc::new(|_| {}),
        )
        .unwrap();
    let open = manager.windows.borrow()[0].clone();
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(Box::new(frame()))],
    );
    (runtime, manager, open, service)
}

#[test]
fn image_drag_and_cached_scroll_reposition_before_backend_and_do_not_upload_again() {
    let (_runtime, manager, open, service) = open();
    assert!(open.window.get_tiled());
    let uploaded = open.window.get_tile_upload_count();
    assert_eq!(uploaded, 9);
    open.window.invoke_image_drag(12.5, 9.25);
    assert_eq!(open.image_camera.borrow().origin, [499.5, 502.75]);
    assert!(open.window.get_tile_offset_x() < 0.0);
    assert!(open.window.get_tile_offset_y() < 0.0);
    assert_eq!(open.window.get_tile_upload_count(), uploaded);
    open.window.invoke_scroll(3.5, 7.25);
    assert_eq!(open.image_camera.borrow().origin, [503.0, 510.0]);
    let mut acknowledgement = frame();
    acknowledgement.revision = 2;
    acknowledgement.image.as_mut().unwrap().pan_sequence = 1;
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(Box::new(acknowledgement))],
    );
    assert_eq!(open.image_camera.borrow().origin, [503.0, 510.0]);
    assert_eq!(open.window.get_tile_upload_count(), uploaded);
    manager.close_all();
}

#[test]
fn scrollbar_latest_target_survives_stale_reply_and_only_new_frontier_uploads() {
    let (_runtime, manager, open, service) = open();
    open.window.invoke_seek(0.75);
    open.window.invoke_seek(0.9);
    assert!((open.window.get_position() - 0.9).abs() < 0.0001);
    let viewport = [
        open.window.get_image_viewport_width(),
        open.window.get_image_viewport_height(),
    ];
    let mut stale = frame();
    stale.revision = 2;
    stale.position = 0.1;
    stale.image.as_mut().unwrap().visible_source = [800.0, 480.0];
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(Box::new(stale))],
    );
    assert!((open.window.get_position() - 0.9).abs() < 0.0001);
    assert_eq!(
        [
            open.window.get_image_viewport_width(),
            open.window.get_image_viewport_height(),
        ],
        viewport,
        "a rejected frame cannot change retained image geometry",
    );
    let mut frontier = frame();
    frontier.revision = 3;
    let mut tiles = frontier.tiles.as_ref().unwrap().to_vec();
    tiles.push(tile(10, 512.0, 0.0));
    frontier.tiles = Some(tiles.into());
    // A fresh surface (e.g. resize) abandons the pending old scrollbar camera.
    frontier.image.as_mut().unwrap().surface = ViewerSurfaceId(21);
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(Box::new(frontier))],
    );
    assert_eq!(open.window.get_tile_upload_count(), 10);
    assert_eq!(open.image_camera.borrow().origin, [512.0; 2]);
    manager.close_all();
}

#[test]
fn changed_image_source_stops_cached_motion_and_stale_failure_cannot_disable_a_new_surface() {
    let (_runtime, manager, open, service) = open();
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::ImageUnavailable {
            generation: 1,
            surface: ViewerSurfaceId(19),
            message: "Old source changed".into(),
        }],
    );
    open.window.invoke_image_drag(5.0, 5.0);
    assert_eq!(open.image_camera.borrow().origin, [507.0; 2]);
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::ImageUnavailable {
            generation: 1,
            surface: ViewerSurfaceId(20),
            message: "Source changed".into(),
        }],
    );
    open.window.invoke_image_drag(10.0, 10.0);
    assert_eq!(open.image_camera.borrow().origin, [507.0; 2]);
    assert_eq!(open.window.get_status(), "Source changed");
    manager.close_all();
}

#[test]
fn early_image_open_failure_is_visible_and_cannot_replace_a_later_files_status() {
    let (_runtime, manager, open, service) = open();
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::ImageUnavailable {
            generation: 2,
            surface: ViewerSurfaceId(0),
            message: "Cannot open image".into(),
        }],
    );
    assert_eq!(open.window.get_status(), "Cannot open image");
    let mut replacement = frame();
    replacement.revision = 2;
    replacement.generation = 3;
    replacement.image.as_mut().unwrap().surface = ViewerSurfaceId(100);
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(Box::new(replacement))],
    );
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::ImageUnavailable {
            generation: 2,
            surface: ViewerSurfaceId(0),
            message: "Old failed open".into(),
        }],
    );
    assert_eq!(open.window.get_status(), "Test");
    manager.close_all();
}

fn match_fixture_viewport(open: &OpenWindow) -> slint::LogicalSize {
    let scale = open.window.window().scale_factor();
    let size = open.window.window().size().to_logical(scale);
    let size = slint::LogicalSize::new(
        size.width + 400.0 - open.window.get_content_width(),
        size.height + 240.0 - open.window.get_content_height(),
    );
    open.window.window().set_size(size);
    open.window
        .window()
        .dispatch_event(slint::platform::WindowEvent::Resized { size });
    assert!((open.window.get_content_width() - 400.0).abs() < 0.01);
    assert!((open.window.get_content_height() - 240.0).abs() < 0.01);
    let preferences = ViewerPreferences {
        image_zoom: ViewerZoomPreference::ActualPixels,
        ..Default::default()
    };
    open.session
        .restore_preferences(Arc::new(preferences))
        .unwrap();
    size
}

#[test]
fn zoom_is_one_accepted_command_anchored_to_displayed_pixels_and_refusal_saves_nothing() {
    let (_runtime, manager, open, _service) = open();
    match_fixture_viewport(&open);
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let received = submitted.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        received.borrow_mut().push(input);
        true
    });
    open.window.invoke_scroll(5.0, 7.0);
    let displayed = open.image_camera.borrow().origin;
    submitted.borrow_mut().clear();
    open.window.invoke_image_zoom(1.0, 100.0, 80.0);
    let inputs = submitted.borrow();
    let [
        ViewerInput::Image {
            frame: 1,
            input:
                ViewerImageInput::ViewZoom {
                    zoom,
                    source_anchor,
                    viewport_anchor,
                },
        },
    ] = inputs.as_slice()
    else {
        panic!("zoom must enqueue one atomic source-anchored command: {inputs:?}")
    };
    assert_eq!(*zoom, ViewerZoomPreference::ScaleBasisPoints(12500));
    assert_eq!(*source_anchor, [displayed[0] + 100.0, displayed[1] + 80.0]);
    assert_eq!(*viewport_anchor, [100.0, 80.0]);
    drop(inputs);
    let accepted = open.session.preferences();
    *open.input.borrow_mut() = Rc::new(|_| false);
    open.window.invoke_image_zoom(1.0, 100.0, 80.0);
    assert_eq!(open.session.preferences(), accepted);
    *open.input.borrow_mut() = Rc::new(|_| true);
    open.window.invoke_image_zoom(1.0, 100.0, 80.0);
    assert_eq!(
        open.session.preferences().image_zoom,
        ViewerZoomPreference::ScaleBasisPoints(15_625),
        "a refused enqueue must retain only the earlier accepted intent"
    );
    manager.close_all();
}

#[test]
fn pdf_zoom_steps_from_displayed_fit_instead_of_an_old_saved_document_scale() {
    let (_runtime, manager, open, service) = open();
    match_fixture_viewport(&open);
    open.session
        .restore_preferences(Arc::new(ViewerPreferences {
            document_zoom: ViewerZoomPreference::ScaleBasisPoints(50_000),
            ..Default::default()
        }))
        .unwrap();
    let mut document = frame();
    document.revision = 2;
    document.tiles = None;
    document.pixels = vec![255; 400 * 240 * 4].into();
    let info = document.image.as_mut().unwrap();
    info.page = 1;
    info.page_count = 2;
    info.zoom = 0.08;
    info.fitted = true;
    info.origin = [0.0; 2];
    info.visible_source = [5000.0, 3000.0];
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(Box::new(document))],
    );
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let received = submitted.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        received.borrow_mut().push(input);
        true
    });
    open.window.invoke_image_zoom(1.0, 100.0, 80.0);
    assert!(
        matches!(
            submitted.borrow().as_slice(),
            [ViewerInput::Image {
                input: ViewerImageInput::ViewZoom {
                    zoom: ViewerZoomPreference::ScaleBasisPoints(1000),
                    ..
                },
                ..
            }]
        ),
        "the first 1.25x step must use displayed 8%, not saved 500%: {:?}",
        submitted.borrow()
    );
    assert_eq!(
        open.session.preferences().document_zoom,
        ViewerZoomPreference::ScaleBasisPoints(1000)
    );
    open.window.invoke_image_zoom(1.0, 100.0, 80.0);
    assert_eq!(
        open.session.preferences().document_zoom,
        ViewerZoomPreference::ScaleBasisPoints(1250),
        "same-frame wheel bursts must accumulate every accepted step"
    );
    let mut intermediate = frame();
    intermediate.revision = 3;
    intermediate.tiles = None;
    intermediate.pixels = vec![255; 400 * 240 * 4].into();
    let info = intermediate.image.as_mut().unwrap();
    info.page = 1;
    info.page_count = 2;
    info.zoom = 0.1;
    info.origin = [0.0; 2];
    info.visible_source = [4000.0, 2400.0];
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(Box::new(intermediate))],
    );
    open.window.invoke_image_zoom(1.0, 100.0, 80.0);
    assert_eq!(
        open.session.preferences().document_zoom,
        ViewerZoomPreference::ScaleBasisPoints(1563),
        "an intermediate 10% frame must not overwrite the later accepted 12.5% intent"
    );
    let accepted = open.session.preferences();
    *open.input.borrow_mut() = Rc::new(|_| false);
    open.window.invoke_image_zoom(1.0, 100.0, 80.0);
    assert_eq!(open.session.preferences(), accepted);
    manager.close_all();
}

#[test]
fn returning_zoom_to_an_older_scale_does_not_mistake_an_old_frame_for_acknowledgment() {
    let (_runtime, manager, open, service) = open();
    match_fixture_viewport(&open);
    *open.input.borrow_mut() = Rc::new(|_| true);
    open.window.invoke_image_zoom(1.0, 100.0, 80.0);
    open.window.invoke_image_zoom(-1.0, 100.0, 80.0);
    assert_eq!(
        open.session.preferences().image_zoom,
        ViewerZoomPreference::ScaleBasisPoints(10_000)
    );
    for (revision, zoom) in [(2, 1.0), (3, 1.25)] {
        let mut intermediate = frame();
        intermediate.revision = revision;
        intermediate.tiles = None;
        intermediate.pixels = vec![255; 400 * 240 * 4].into();
        let info = intermediate.image.as_mut().unwrap();
        info.zoom = zoom;
        info.visible_source = [400.0 / zoom, 240.0 / zoom];
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![ViewerOutput::Frame(Box::new(intermediate))],
        );
    }
    open.window.invoke_image_zoom(1.0, 100.0, 80.0);
    assert_eq!(
        open.session.preferences().image_zoom,
        ViewerZoomPreference::ScaleBasisPoints(12_500),
        "older 100% and intermediate 125% frames cannot erase the last accepted 100% intent"
    );
    manager.close_all();
}

#[test]
fn zoom_after_pending_fit_enlarges_pdf_pages_but_keeps_raster_no_upscale_policy() {
    let (_runtime, manager, open, service) = open();
    match_fixture_viewport(&open);
    *open.input.borrow_mut() = Rc::new(|_| true);
    for (page, expected) in [(0, 12_500), (1, 50_000)] {
        let mut small = frame();
        small.revision = u64::from(page) + 2;
        small.tiles = None;
        small.pixels = vec![255; 400 * 240 * 4].into();
        let info = small.image.as_mut().unwrap();
        info.page = page;
        info.width = 100;
        info.height = 60;
        info.zoom = 0.5;
        info.visible_source = [800.0, 480.0];
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![ViewerOutput::Frame(Box::new(small))],
        );
        open.window.invoke_image_command(0);
        open.window.invoke_image_zoom(1.0, 100.0, 80.0);
        let preferences = open.session.preferences();
        assert_eq!(
            if page == 0 {
                preferences.image_zoom
            } else {
                preferences.document_zoom
            },
            ViewerZoomPreference::ScaleBasisPoints(expected)
        );
    }
    manager.close_all();
}

fn fit_toolbar_frame(
    generation: u64,
    revision: u64,
    viewport: [u32; 2],
    mut image: ViewerImageInfo,
) -> ViewerFrame {
    image.surface = ViewerSurfaceId(10_000 + revision);
    image.visible_source = viewport.map(|extent| f64::from(extent) / image.zoom);
    image.raster_scale = image.zoom.min(1.0);
    image.pan_sequence = 0;
    let dimensions = [image.width, image.height].map(f64::from);
    let extent = std::array::from_fn::<_, 2, _>(|axis| {
        (image.visible_source[axis] / dimensions[axis]).min(1.0)
    });
    let position = std::array::from_fn::<_, 2, _>(|axis| {
        if extent[axis] < 1.0 {
            image.origin[axis] / (dimensions[axis] - image.visible_source[axis])
        } else {
            0.0
        }
    });
    let low = image
        .origin
        .map(|origin| (origin * image.raster_scale / 256.0).floor() as i32 - 1);
    let high = std::array::from_fn::<_, 2, _>(|axis| {
        ((image.origin[axis] + image.visible_source[axis]) * image.raster_scale / 256.0).ceil()
            as i32
            + 1
    });
    let pixels = Arc::new(ViewerPixels::new(vec![255; 256 * 256 * 4], Arc::new(Pixels)).unwrap());
    let mut tiles = Vec::new();
    for y in low[1]..high[1] {
        for x in low[0]..high[0] {
            tiles.push(ViewerTile {
                id: ViewerTileId(100_000 + revision * 128 + tiles.len() as u64),
                position: std::array::from_fn(|axis| {
                    (f64::from([x, y][axis]) * 256.0 - image.origin[axis] * image.raster_scale)
                        as f32
                }),
                width: 256,
                height: 256,
                pixels: pixels.clone(),
            });
        }
    }
    assert!(tiles.len() <= 128);
    let mut frame = frame();
    frame.generation = generation;
    frame.revision = revision;
    frame.width = (image.visible_source[0] * image.raster_scale) as u32;
    frame.height = (image.visible_source[1] * image.raster_scale) as u32;
    frame.horizontal_extent = extent[0];
    frame.vertical_extent = extent[1];
    frame.horizontal_position = position[0];
    frame.position = position[1];
    frame.image = Some(image);
    frame.tiles = Some(tiles.into());
    frame
}

#[test]
fn toolbar_fit_restores_panned_rasters_at_native_dpi_through_late_frames_and_resize() {
    let (_runtime, manager, open, service) = open();
    open.timer.stop();
    open.image_redraw.stop();
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let received = submitted.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        received.borrow_mut().push(input);
        true
    });
    // Supplied backend frames isolate toolbar dispatch and retained presentation.
    // These literal Fit results do not invoke the engine's geometry calculation.
    for (
        index,
        (
            dpi,
            source,
            viewport,
            fit_zoom,
            fit_origin,
            resized_viewport,
            resized_zoom,
            resized_origin,
            zoom_in,
        ),
    ) in [
        (
            2.0,
            [4000, 1000],
            [800, 480],
            0.2,
            [0.0, -700.0],
            [1200, 600],
            0.3,
            [0.0, -500.0],
            2500,
        ),
        (
            2.25,
            [1000, 4000],
            [900, 540],
            0.135,
            [-2833.3333333333335, 0.0],
            [1350, 675],
            0.16875,
            [-3500.0, 0.0],
            1688,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let generation = 2 + index as u64;
        let revision = 10 + index as u64 * 10;
        open.window
            .window()
            .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: dpi });
        // The minimal adapter changes DPI without resizing its stored physical size.
        // Reseed a logical size that has exact physical dimensions at both native scales.
        open.window
            .window()
            .set_size(slint::LogicalSize::new(900.0, 600.0));
        let fixture_size = match_fixture_viewport(&open);
        let manual = ViewerImageInfo {
            width: source[0],
            height: source[1],
            zoom: 2.0,
            origin: [128.0, 256.0],
            fitted: false,
            ..info()
        };
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![ViewerOutput::Frame(Box::new(fit_toolbar_frame(
                generation,
                revision,
                viewport,
                manual.clone(),
            )))],
        );
        assert_eq!(open.frame.get(), revision);
        for (axis, &expected_extent) in viewport.iter().enumerate() {
            assert!((image_viewport(&open)[axis] - expected_extent as f32).abs() < 0.01);
        }
        open.window.invoke_image_drag(12.0, -8.0);
        let panned = open.image_camera.borrow().origin;
        assert_eq!(
            panned,
            [128.0 - 6.0 * f64::from(dpi), 256.0 + 4.0 * f64::from(dpi)]
        );
        assert_ne!(panned, manual.origin);
        submitted.borrow_mut().clear();
        open.window.invoke_image_command(0);
        {
            let inputs = submitted.borrow();
            let [
                ViewerInput::Image {
                    frame: command_frame,
                    input:
                        ViewerImageInput::ViewZoom {
                            zoom,
                            source_anchor,
                            viewport_anchor,
                        },
                },
            ] = inputs.as_slice()
            else {
                panic!("Fit must enqueue one atomic image command: {inputs:?}");
            };
            assert_eq!(*command_frame, revision);
            assert_eq!(*zoom, ViewerZoomPreference::Fit);
            for axis in 0..2 {
                let center = f64::from(viewport[axis]) / 2.0;
                assert!((f64::from(viewport_anchor[axis]) - center).abs() < 0.01);
                assert!((source_anchor[axis] - (panned[axis] + center / 2.0)).abs() < 0.01);
            }
        }
        assert_eq!(
            open.session.preferences().image_zoom,
            ViewerZoomPreference::Fit
        );
        assert!(!open.image_camera.borrow().active());
        assert!(!open.image_redraw.running());
        let pending_scale = || {
            open.image_zoom.borrow().scale(
                generation,
                open.image_info.borrow().as_ref().unwrap(),
                image_viewport(&open),
            )
        };
        assert!((pending_scale() - fit_zoom).abs() < 1e-8);

        open.window.invoke_image_command(2);
        open.window.invoke_image_command(0);
        let zooms: Vec<_> = submitted
            .borrow()
            .iter()
            .map(|input| match input {
                ViewerInput::Image {
                    input: ViewerImageInput::ViewZoom { zoom, .. },
                    ..
                } => *zoom,
                other => panic!("unexpected input during toolbar burst: {other:?}"),
            })
            .collect();
        assert_eq!(
            zooms,
            [
                ViewerZoomPreference::Fit,
                ViewerZoomPreference::ScaleBasisPoints(zoom_in),
                ViewerZoomPreference::Fit,
            ]
        );
        assert_eq!(
            open.session.preferences().image_zoom,
            ViewerZoomPreference::Fit
        );
        assert!((pending_scale() - fit_zoom).abs() < 1e-8);

        let mut intermediate = manual.clone();
        intermediate.zoom = f64::from(zoom_in) / 10_000.0;
        intermediate.origin = std::array::from_fn(|axis| {
            ((f64::from(source[axis]) - f64::from(viewport[axis]) / intermediate.zoom) / 2.0)
                .min(0.0)
        });
        let intermediate = fit_toolbar_frame(generation, revision + 1, viewport, intermediate);
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![ViewerOutput::Frame(Box::new(intermediate.clone()))],
        );
        assert_eq!(
            open.frame.get(),
            revision + 1,
            "the intermediate reply was admitted"
        );
        assert_eq!(
            open.session.preferences().image_zoom,
            ViewerZoomPreference::Fit
        );
        assert!((pending_scale() - fit_zoom).abs() < 1e-8);
        let fitted = ViewerImageInfo {
            zoom: fit_zoom,
            origin: fit_origin,
            fitted: true,
            ..manual.clone()
        };
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![ViewerOutput::Frame(Box::new(fit_toolbar_frame(
                generation,
                revision + 2,
                viewport,
                fitted.clone(),
            )))],
        );
        let fitted_context = open.window.get_position_context();
        for late in [false, true] {
            if late {
                present_outputs(
                    &open,
                    &service,
                    &|_| {},
                    vec![ViewerOutput::Frame(Box::new(intermediate.clone()))],
                );
            }
            assert_eq!(open.frame.get(), revision + 2);
            assert_eq!(open.image_camera.borrow().origin, fit_origin);
            assert!(!open.image_camera.borrow().active());
            assert_eq!(open.window.get_horizontal_extent(), 1.0);
            assert_eq!(open.window.get_vertical_extent(), 1.0);
            assert_eq!(open.window.get_horizontal_position(), 0.0);
            assert_eq!(open.window.get_position(), 0.0);
            assert_eq!(open.window.get_image_viewport_width(), viewport[0] as f32);
            assert_eq!(open.window.get_image_viewport_height(), viewport[1] as f32);
            assert!((f64::from(open.window.get_zoom_percent()) - fit_zoom * 100.0).abs() < 0.001);
            assert_eq!(open.window.get_position_context(), fitted_context);
            assert_eq!(
                open.session.preferences().image_zoom,
                ViewerZoomPreference::Fit
            );
        }

        // Preserve the requested logical size: reading back physical pixels loses
        // fractions at 2.25x and would change the intended 600x300 logical canvas.
        let size = slint::LogicalSize::new(fixture_size.width + 200.0, fixture_size.height + 60.0);
        open.window.window().set_size(size);
        open.window
            .window()
            .dispatch_event(slint::platform::WindowEvent::Resized { size });
        for (axis, &expected_extent) in resized_viewport.iter().enumerate() {
            assert!((image_viewport(&open)[axis] - expected_extent as f32).abs() < 0.01);
        }
        // The retained frame keeps its physical extent until the resized reply arrives.
        assert_eq!(open.window.get_image_viewport_width(), viewport[0] as f32);
        assert_eq!(open.window.get_image_viewport_height(), viewport[1] as f32);
        let resized = ViewerImageInfo {
            zoom: resized_zoom,
            origin: resized_origin,
            ..fitted
        };
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![ViewerOutput::Frame(Box::new(fit_toolbar_frame(
                generation,
                revision + 3,
                resized_viewport,
                resized,
            )))],
        );
        assert_eq!(open.frame.get(), revision + 3);
        assert_eq!(open.image_camera.borrow().origin, resized_origin);
        assert!(!open.image_camera.borrow().active());
        assert_eq!(open.window.get_horizontal_extent(), 1.0);
        assert_eq!(open.window.get_vertical_extent(), 1.0);
        assert_eq!(open.window.get_horizontal_position(), 0.0);
        assert_eq!(open.window.get_position(), 0.0);
        assert_eq!(
            open.window.get_image_viewport_width(),
            resized_viewport[0] as f32
        );
        assert_eq!(
            open.window.get_image_viewport_height(),
            resized_viewport[1] as f32
        );
        assert_ne!(open.window.get_position_context(), fitted_context);
        assert!((pending_scale() - resized_zoom).abs() < 1e-8);
        assert!((f64::from(open.window.get_zoom_percent()) - resized_zoom * 100.0).abs() < 0.001);
        assert_eq!(
            open.session.preferences().image_zoom,
            ViewerZoomPreference::Fit
        );
    }
    manager.close_all();
}

#[test]
fn restored_raster_zoom_replaces_the_previous_local_zoom_intent() {
    let (_runtime, manager, open, service) = open();
    match_fixture_viewport(&open);
    *open.input.borrow_mut() = Rc::new(|_| true);
    open.window.invoke_image_zoom(1.0, 100.0, 80.0);
    manager.restore_preferences(Arc::new(ViewerPreferences {
        image_zoom: ViewerZoomPreference::ScaleBasisPoints(20_000),
        ..Default::default()
    }));
    let mut restored = frame();
    restored.revision = 2;
    restored.tiles = None;
    restored.pixels = vec![255; 400 * 240 * 4].into();
    let info = restored.image.as_mut().unwrap();
    info.zoom = 2.0;
    info.visible_source = [200.0, 120.0];
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(Box::new(restored))],
    );
    open.window.invoke_image_zoom(1.0, 100.0, 80.0);
    assert_eq!(
        open.session.preferences().image_zoom,
        ViewerZoomPreference::ScaleBasisPoints(25_000)
    );
    manager.close_all();
}

#[test]
fn accepted_zoom_intent_cannot_cross_pdf_page_or_source_identity() {
    let (_runtime, manager, open, service) = open();
    match_fixture_viewport(&open);
    *open.input.borrow_mut() = Rc::new(|_| true);
    for (generation, revision, page, zoom, expected) in [
        (1, 2, 1, 1.0, 12_500),
        (1, 3, 2, 0.5, 6_250),
        (2, 4, 2, 0.25, 3_125),
    ] {
        let mut document = frame();
        document.generation = generation;
        document.revision = revision;
        document.tiles = None;
        document.pixels = vec![255; 400 * 240 * 4].into();
        let info = document.image.as_mut().unwrap();
        info.page = page;
        info.page_count = 2;
        info.zoom = zoom;
        info.visible_source = [400.0 / zoom, 240.0 / zoom];
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![ViewerOutput::Frame(Box::new(document))],
        );
        open.window.invoke_image_zoom(1.0, 100.0, 80.0);
        assert_eq!(
            open.session.preferences().document_zoom,
            ViewerZoomPreference::ScaleBasisPoints(expected),
            "a changed page/source must use its displayed scale"
        );
    }
    manager.close_all();
}

#[test]
fn replaced_file_frame_and_resized_coordinates_cannot_enqueue_or_save_a_zoom() {
    file_navigation_rejects_old_zoom("viewer.next-file");
}

#[test]
fn previous_file_frame_and_resized_coordinates_cannot_enqueue_or_save_a_zoom() {
    file_navigation_rejects_old_zoom("viewer.previous-file");
}

fn file_navigation_rejects_old_zoom(action: &str) {
    let (_runtime, manager, open, service) = open();
    match_fixture_viewport(&open);
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let received = submitted.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        received.borrow_mut().push(input);
        true
    });
    open.window
        .window()
        .set_size(slint::LogicalSize::new(700.0, 500.0));
    let before = open.session.preferences();
    open.window.invoke_image_zoom(1.0, 100.0, 80.0);
    assert!(submitted.borrow().is_empty());
    assert_eq!(open.session.preferences(), before);
    assert!(dispatch_key(&open, action));
    assert!(matches!(
        (action, submitted.borrow().last()),
        ("viewer.next-file", Some(ViewerInput::NextFile))
            | ("viewer.previous-file", Some(ViewerInput::PreviousFile))
    ));
    submitted.borrow_mut().clear();
    let mut old = frame();
    old.revision = 2;
    old.image.as_mut().unwrap().surface = ViewerSurfaceId(21);
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(Box::new(old))],
    );
    open.window.invoke_image_command(1);
    assert!(submitted.borrow().is_empty());
    assert_eq!(open.session.preferences(), before);
    manager.close_all();
}

#[test]
fn untiled_document_keeps_captured_raster_metrics_and_routes_both_scrollbars_to_backend() {
    let (_runtime, manager, open, service) = open();
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let received = submitted.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        received.borrow_mut().push(input);
        true
    });
    let mut document = frame();
    document.revision = 2;
    document.tiles = None;
    document.width = 2;
    document.height = 2;
    document.pixels = vec![255; 16].into();
    document.position = 0.3;
    document.horizontal_position = 0.4;
    document.image.as_mut().unwrap().page = 1;
    document.image.as_mut().unwrap().page_count = 2;
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(Box::new(document))],
    );
    assert!(!open.window.get_tiled());
    assert!((open.window.get_displayed_position() - 0.3).abs() < 0.0001);
    open.window.invoke_seek(0.7);
    open.window.invoke_pan(0.8);
    assert!(
        matches!(submitted.borrow().as_slice(),[ViewerInput::Seek(vertical),ViewerInput::Pan(horizontal)]
        if (*vertical-0.7).abs()<0.0001 && (*horizontal-0.8).abs()<0.0001)
    );
    manager.close_all();
}

#[test]
fn phased_precise_image_gesture_keeps_residual_pixels_until_guard_tiles_arrive() {
    use slint::winit_030::winit::{
        dpi::PhysicalPosition,
        event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent},
    };
    let (_runtime, manager, open, service) = open();
    open.timer.stop();
    *open.input.borrow_mut() = Rc::new(|_| true);
    assert!(!image_motion::observe(
        &open,
        &WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 0.0)),
            phase: TouchPhase::Started,
        }
    ));
    open.window.invoke_scroll(0.0, 400.0);
    assert!(
        open.image_camera.borrow().active(),
        "guard boundary retains unsent pixels"
    );
    let edge = open.image_camera.borrow().origin[1];
    assert!(edge > 512.0 && edge < 912.0);
    image_motion::observe(&open, &WindowEvent::RedrawRequested);
    assert!(
        open.image_camera.borrow().active(),
        "redraw discarded the direct gesture residual"
    );
    let mut ready = frame();
    ready.revision = 2;
    let mut tiles = ready.tiles.take().unwrap().to_vec();
    tiles.extend([
        tile(10, -256.0, 512.0),
        tile(11, 0.0, 512.0),
        tile(12, 256.0, 512.0),
    ]);
    ready.tiles = Some(tiles.into());
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(Box::new(ready))],
    );
    image_motion::observe(&open, &WindowEvent::RedrawRequested);
    assert_eq!(
        open.image_camera.borrow().origin[1],
        912.0,
        "guard refill lost precise pixels"
    );
    manager.close_all();
}
fn small_opening_frame(revision: u64, viewport: [u32; 2]) -> ViewerFrame {
    let mut frame = frame();
    frame.revision = revision;
    frame.width = viewport[0];
    frame.height = viewport[1];
    frame.vertical_extent = 1.0;
    frame.horizontal_extent = 1.0;
    let info = frame.image.as_mut().unwrap();
    info.surface = ViewerSurfaceId(1000 + revision);
    info.width = 128;
    info.height = 96;
    info.zoom = 1.0;
    info.fitted = true;
    info.visible_source = viewport.map(f64::from);
    info.origin = [
        (128.0 - f64::from(viewport[0])) / 2.0,
        (96.0 - f64::from(viewport[1])) / 2.0,
    ];
    let mut pixels = vec![0; 256 * 256 * 4];
    for y in 0..96 {
        for x in 0..128 {
            let color = if (16..36).contains(&x) && (24..36).contains(&y) {
                [239, 47, 101, 255]
            } else {
                [19, 171, 83, 255]
            };
            let offset = (y * 256 + x) * 4;
            pixels[offset..offset + 4].copy_from_slice(&color);
        }
    }
    frame.tiles = Some(
        vec![ViewerTile {
            id: ViewerTileId(5000 + revision),
            position: info.origin.map(|origin| -origin as f32),
            width: 256,
            height: 256,
            pixels: Arc::new(ViewerPixels::new(pixels, Arc::new(Pixels)).unwrap()),
        }]
        .into(),
    );
    frame
}

fn opening_color_bounds(color: [u8; 3]) -> [u32; 4] {
    let (size, pixels) = bitmap();
    let mut bounds = [size.width, size.height, 0, 0];
    for (index, pixel) in pixels.iter().enumerate() {
        if [pixel.r, pixel.g, pixel.b] == color {
            let x = index as u32 % size.width;
            let y = index as u32 / size.width;
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].min(y);
            bounds[2] = bounds[2].max(x + 1);
            bounds[3] = bounds[3].max(y + 1);
        }
    }
    assert!(
        bounds[0] < bounds[2] && bounds[1] < bounds[3],
        "missing image color {color:?}"
    );
    bounds
}

#[test]
fn opening_small_image_keeps_physical_pixels_while_dpi_and_compact_chrome_settle() {
    let (_runtime, manager, open, service) = open();
    open.timer.stop();
    open.image_redraw.stop();
    *open.input.borrow_mut() = Rc::new(|_| true);
    open.window
        .window()
        .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
    for (index, width) in [900.0, 420.0].into_iter().enumerate() {
        let compact = width < 500.0;
        open.window.set_compact(compact);
        open.window.set_narrow_typography(compact);
        open.window.set_image_mode(false);
        open.window
            .window()
            .set_size(slint::LogicalSize::new(width, 600.0));
        bitmap();
        let opening_height = open.window.get_content_height();
        // Reproduce a frame requested before native DPI and image-mode chrome
        // settled. Its 100% pixels must retain their physical size on arrival.
        let revision = 2 + index as u64 * 2;
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![ViewerOutput::Frame(Box::new(small_opening_frame(
                revision,
                [width as u32, 552],
            )))],
        );
        let first_image = opening_color_bounds([19, 171, 83]);
        let first_marker = opening_color_bounds([239, 47, 101]);
        if compact {
            assert!(open.window.get_content_height() < opening_height);
        }
        let viewport = image_viewport(&open).map(|value| value as u32);
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![ViewerOutput::Frame(Box::new(small_opening_frame(
                revision + 1,
                viewport,
            )))],
        );
        let settled_image = opening_color_bounds([19, 171, 83]);
        let settled_marker = opening_color_bounds([239, 47, 101]);
        for (name, first, settled, expected) in [
            ("image", first_image, settled_image, [128, 96]),
            ("marker", first_marker, settled_marker, [20, 12]),
        ] {
            for axis in 0..2 {
                let first_extent = first[axis + 2] - first[axis];
                let settled_extent = settled[axis + 2] - settled[axis];
                assert!(
                    first_extent.abs_diff(expected[axis]) <= 1,
                    "compact={compact} {name} axis={axis}: first={first_extent}, settled={settled_extent}, expected={}",
                    expected[axis],
                );
                assert!(settled_extent.abs_diff(expected[axis]) <= 1);
                assert!(first_extent.abs_diff(settled_extent) <= 1);
                assert!(first[axis].abs_diff(settled[axis]) <= 1);
            }
        }
    }
    manager.close_all();
}
#[test]
fn rejected_open_does_not_leave_a_visible_native_window() {
    let (_runtime, manager, _open, _service) = open();
    let path = std::env::temp_dir().join("invalid-image-preferences.bmp");
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
    let mut preferences = ViewerPreferences::default();
    preferences.text_font.size = 0;
    let result = manager.open(
        ViewerLaunch {
            find: None,
            associations: Default::default(),
            tree: None,
            generated: None,
            preferences: Arc::new(preferences),
            archive: None,
            view: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
            focused: path,
            basic: false,
            disabled_plugins: Arc::from([]),
        },
        ViewerAppearance {
            family: "monospace".into(),
            font_size: 14.0,
            foreground: [255; 4],
            background: [0, 0, 0, 255],
            selection: [55; 4],
            accent: [150; 4],
        },
        slint::LogicalSize::new(420.0, 320.0),
        Rc::new(|_, _| None),
        Rc::new(|_| {}),
    );
    assert!(result.is_err());
    assert!(TEST_WINDOW.with(|slot| {
        slot.borrow()
            .upgrade()
            .is_none_or(|adapter| !adapter.window().is_visible())
    }));
    assert_eq!(manager.windows.borrow().len(), 1);
    manager.close_all();
}
#[test]
fn dpi_change_preserves_first_drag_distance_before_resize_frame() {
    let (_runtime, manager, open, service) = open();
    open.timer.stop();
    open.image_redraw.stop();
    *open.input.borrow_mut() = Rc::new(|_| true);
    open.window
        .window()
        .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
    let mut initial = frame();
    initial.revision = 2;
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(Box::new(initial))],
    );
    for (scale, source_distance) in [(1.0, 10.0), (2.0, 20.0)] {
        open.window
            .window()
            .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
                scale_factor: scale,
            });
        let before = open.image_camera.borrow().origin;
        // No timer, redraw or replacement frame intervenes before this gesture.
        open.window.invoke_image_drag(-10.0, -10.0);
        let after = open.image_camera.borrow().origin;
        assert_eq!(after, before.map(|origin| origin + source_distance));
        assert_eq!(open.window.get_image_pixel_scale(), 1.0 / scale);
    }
    manager.close_all();
}
#[test]
fn source_resolution_tiles_reuse_pixels_across_magnified_camera_surfaces() {
    use slint::Model;

    let (_runtime, manager, open, service) = open();
    open.timer.stop();
    open.image_redraw.stop();
    *open.input.borrow_mut() = Rc::new(|_| true);
    open.window
        .window()
        .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
    open.window
        .window()
        .set_size(slint::LogicalSize::new(900.0, 600.0));
    bitmap();
    let viewport = image_viewport(&open).map(|value| value as u32);
    let source = small_opening_frame(2, viewport).tiles.as_ref().unwrap()[0]
        .pixels
        .clone();
    let expected_uploads = open.window.get_tile_upload_count() + 1;
    // Slint software renderer 1.18.1 stores the inverse texture scale in 8
    // fractional bits: these zooms use steps 204/256, 128/256 and 85/256.
    // Truncated sample positions can extend a solid block by a few pixels:
    // at 3x, ceil(128 * 256 / 85) = 386 and ceil(96 * 256 / 85) = 290.
    // Keep exact sampler expectations alongside strict destination geometry.
    for (index, (zoom, sampled_image, sampled_marker)) in [
        (1.25, [161, 121], [25, 15]),
        (2.0, [256, 192], [40, 24]),
        (3.0, [386, 290], [60, 36]),
    ]
    .into_iter()
    .enumerate()
    {
        let revision = 2 + index as u64;
        let mut enlarged = small_opening_frame(revision, viewport);
        let info = enlarged.image.as_mut().unwrap();
        info.zoom = zoom;
        info.fitted = false;
        info.visible_source = viewport.map(|extent| f64::from(extent) / zoom);
        info.origin = [
            (128.0 - info.visible_source[0]) / 2.0,
            (96.0 - info.visible_source[1]) / 2.0,
        ];
        enlarged.tiles = Some(
            vec![ViewerTile {
                id: ViewerTileId(9000),
                position: info.origin.map(|origin| -origin as f32),
                width: 256,
                height: 256,
                pixels: source.clone(),
            }]
            .into(),
        );
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![ViewerOutput::Frame(Box::new(enlarged))],
        );
        assert!(open.window.get_image_native_pixels());
        assert_eq!(open.window.get_tile_upload_count(), expected_uploads);
        let tile = open.window.get_image_tiles().row_data(0).unwrap();
        let canvas = image_viewport(&open).map(f64::from);
        let declared = [
            f64::from(open.window.get_image_viewport_width()),
            f64::from(open.window.get_image_viewport_height()),
        ];
        for axis in 0..2 {
            let tile_extent = f64::from([tile.width, tile.height][axis]) * declared[axis];
            assert!((tile_extent - 256.0 * zoom).abs() < 0.001);
            let coordinate = f64::from([tile.x, tile.y][axis]);
            let offset = f64::from(
                [
                    open.window.get_tile_offset_x(),
                    open.window.get_tile_offset_y(),
                ][axis],
            );
            let image_start =
                (canvas[axis] - declared[axis]) / 2.0 + (coordinate - offset) * declared[axis];
            let expected_start = (canvas[axis] - [128.0, 96.0][axis] * zoom) / 2.0;
            assert!((image_start - expected_start).abs() < 0.001);
        }
        let image = opening_color_bounds([19, 171, 83]);
        let marker = opening_color_bounds([239, 47, 101]);
        for (bounds, expected) in [(image, sampled_image), (marker, sampled_marker)] {
            assert_eq!(
                [bounds[2] - bounds[0], bounds[3] - bounds[1]],
                expected,
                "zoom={zoom}: software sample bounds={bounds:?}",
            );
        }
        let (size, pixels) = bitmap();
        for y in marker[1]..marker[3] {
            for x in marker[0]..marker[2] {
                let pixel = pixels[(y * size.width + x) as usize];
                assert_eq!([pixel.r, pixel.g, pixel.b], [239, 47, 101]);
            }
        }
    }
    // Reduced-density rasters and PDF retain their existing smooth sampler.
    for (index, page, raster_scale) in [(0, 0, 0.75), (1, 1, 1.0)] {
        let mut other = small_opening_frame(10 + index, viewport);
        let info = other.image.as_mut().unwrap();
        info.zoom = 2.0;
        info.page = page;
        info.raster_scale = raster_scale;
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![ViewerOutput::Frame(Box::new(other))],
        );
        assert!(!open.window.get_image_native_pixels());
    }
    manager.close_all();
}
