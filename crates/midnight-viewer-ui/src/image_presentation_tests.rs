use super::*;
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::sync::Arc;

struct TestPlatform;
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
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
    let mut stale = frame();
    stale.revision = 2;
    stale.position = 0.1;
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(Box::new(stale))],
    );
    assert!((open.window.get_position() - 0.9).abs() < 0.0001);
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

fn match_fixture_viewport(open: &OpenWindow) {
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
    assert!(dispatch_key(&open, "viewer.next-file"));
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
