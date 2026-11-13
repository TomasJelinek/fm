use super::*;
use std::sync::Arc;

struct Fixture {
    manager: ViewerWindows,
    open: Rc<OpenWindow>,
    service: Service,
    inputs: Rc<RefCell<Vec<ViewerInput>>>,
    accepting: Rc<Cell<bool>>,
    results: Rc<RefCell<Vec<Result<(), String>>>>,
    _runtime: tokio::runtime::Runtime,
}

impl Fixture {
    fn new() -> Self {
        let results = Rc::new(RefCell::new(Vec::new()));
        let received = results.clone();
        let (runtime, manager, open, service) =
            opening_tests::opening(Box::new(move |result| received.borrow_mut().push(result)));
        open.timer.stop();
        let inputs = Rc::new(RefCell::new(Vec::new()));
        let accepting = Rc::new(Cell::new(true));
        let captured = inputs.clone();
        let admitted = accepting.clone();
        *open.input.borrow_mut() = Rc::new(move |input| {
            captured.borrow_mut().push(input);
            admitted.get()
        });
        Self {
            manager,
            open,
            service,
            inputs,
            accepting,
            results,
            _runtime: runtime,
        }
    }

    fn deliver(&self, outputs: Vec<ViewerOutput>) {
        present_outputs(
            &self.open,
            &self.service,
            &|_| panic!("layout metadata cannot copy content"),
            outputs,
        );
    }

    fn select(&self, token: u64, mode: u16, content_generation: u64) {
        self.deliver(vec![ViewerOutput::LayoutSelected {
            token,
            mode,
            content_generation,
        }]);
    }

    fn take_inputs(&self) -> Vec<ViewerInput> {
        std::mem::take(&mut *self.inputs.borrow_mut())
    }

    fn take_ack(&self, expected: u64) -> ViewerGeometry {
        let mut inputs = self.take_inputs();
        assert_eq!(
            inputs.len(),
            1,
            "one layout acknowledgement, no other input"
        );
        match inputs.pop().unwrap() {
            ViewerInput::LayoutReady { token, geometry } => {
                assert_eq!(token, expected);
                let [_, _, width, height] = self.canvas();
                assert!(width.is_finite() && height.is_finite() && width > 1.0 && height > 1.0);
                assert!(geometry.width > 1 && geometry.height > 1);
                assert!(geometry.scale.is_finite() && geometry.scale > 0.0);
                geometry
            }
            input => panic!("layout acknowledgement expected, received {input:?}"),
        }
    }

    fn canvas(&self) -> [f32; 4] {
        [
            self.open.window.get_content_left(),
            self.open.window.get_content_top(),
            self.open.window.get_content_width(),
            self.open.window.get_content_height(),
        ]
    }

    fn resize(&self, width: f32, height: f32) {
        let size = slint::LogicalSize::new(width, height);
        self.open.window.window().set_size(size);
        self.open
            .window
            .window()
            .dispatch_event(slint::platform::WindowEvent::Resized { size });
    }

    fn assert_no_opening_success(&self) {
        assert!(
            self.results.borrow().iter().all(Result::is_err),
            "layout metadata cannot report content readiness; fixture startup errors remain valid"
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.manager.close_all();
    }
}

fn geometry_key(geometry: ViewerGeometry) -> (u32, u32, u32) {
    (geometry.width, geometry.height, geometry.scale.to_bits())
}

fn assert_canvas(actual: [f32; 4], expected: [f32; 4]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 0.001,
            "canvas changed: {actual} != {expected}"
        );
    }
}

fn frame(generation: u64, revision: u64, mode: u16) -> ViewerOutput {
    ViewerOutput::Frame(Box::new(ViewerFrame {
        generation,
        position_sequence: [0; 2],
        revision,
        mode,
        modes: vec![1, 2, 3, VIEWER_MODE_HTML],
        wrap_enabled: false,
        wrap_available: false,
        image: None,
        raster: None,
        raster_rebase: None,
        width: 2,
        height: 2,
        pixels: Arc::from([255; 16]),
        tiles: None,
        accessible_text: format!("generation {generation}").into(),
        position: 0.0,
        vertical_extent: 1.0,
        horizontal_position: 0.0,
        horizontal_extent: 1.0,
        caption: "Layout handshake fixture".into(),
        status: "Offline".into(),
    }))
}

#[test]
fn html_layout_acknowledges_positive_canvas_before_content_without_granting_readiness() {
    let fixture = Fixture::new();
    let initial_surface = fixture.open.window.get_surface().size();
    fixture.select(1, VIEWER_MODE_HTML, 1);
    let acknowledged = fixture.take_ack(1);
    let canvas = fixture.canvas();
    assert!(fixture.open.window.get_web_mode());
    assert!(!fixture.open.window.get_image_mode());
    assert!(!fixture.open.window.get_font_available());
    assert!(canvas[1] > 0.0 && canvas[2] > 1.0 && canvas[3] > 1.0);
    assert_eq!(
        geometry_key(acknowledged),
        geometry_key(window_geometry(&fixture.open.window))
    );
    assert_eq!(fixture.open.frame.get(), 0);
    assert_eq!(fixture.open.window.get_surface().size(), initial_surface);
    assert!(fixture.open.window.get_accessible_text().is_empty());
    assert!(!fixture.open.window.get_web_network());
    assert!(!fixture.open.window.get_web_scripts());
    fixture.assert_no_opening_success();

    fixture.deliver(vec![frame(1, 1, VIEWER_MODE_HTML)]);
    pump_geometry(&fixture.open);
    assert_eq!(fixture.open.frame.get(), 1);
    assert_eq!(fixture.open.window.get_surface().size().width, 2);
    assert_canvas(fixture.canvas(), canvas);
    assert_eq!(
        fixture.open.last_geometry.get(),
        geometry_key(acknowledged),
        "the first HTML picture must use the already acknowledged canvas"
    );
    assert!(
        fixture.take_inputs().is_empty(),
        "the first frame must not trigger Resize"
    );
}

#[test]
fn unchanged_canvas_still_acknowledges_each_new_layout_token_once() {
    let fixture = Fixture::new();
    fixture.select(4, VIEWER_MODE_HTML, 1);
    let first = fixture.take_ack(4);
    fixture.deliver(vec![frame(1, 1, VIEWER_MODE_HTML)]);
    fixture.take_inputs();
    fixture.select(5, VIEWER_MODE_HTML, 2);
    let next = fixture.take_ack(5);
    assert_eq!(geometry_key(next), geometry_key(first));
    fixture.select(5, VIEWER_MODE_HTML, 2);
    pump_geometry(&fixture.open);
    assert!(
        fixture.take_inputs().is_empty(),
        "a consumed token cannot be acknowledged twice"
    );
}

#[test]
fn refused_layout_ack_retries_latest_resized_canvas_without_advancing_last_geometry() {
    let fixture = Fixture::new();
    let previous = fixture.open.last_geometry.get();
    fixture.accepting.set(false);
    fixture.select(7, VIEWER_MODE_HTML, 1);
    let refused = fixture.take_ack(7);
    assert_eq!(fixture.open.last_geometry.get(), previous);

    for (width, height, compact, narrow) in [
        (1000.0, 700.0, false, false),
        (600.0, 400.0, false, true),
        (320.0, 200.0, true, true),
    ] {
        fixture.resize(width, height);
        pump_geometry(&fixture.open);
        let attempt = fixture.take_ack(7);
        assert_eq!(
            geometry_key(attempt),
            geometry_key(window_geometry(&fixture.open.window))
        );
        assert_eq!(fixture.open.window.get_compact(), compact);
        assert_eq!(fixture.open.window.get_narrow_typography(), narrow);
        assert_eq!(fixture.open.last_geometry.get(), previous);
    }

    fixture.resize(900.0, 600.0);
    fixture.accepting.set(true);
    pump_geometry(&fixture.open);
    let accepted = fixture.take_ack(7);
    assert_ne!(geometry_key(accepted), geometry_key(refused));
    assert_eq!(
        geometry_key(accepted),
        geometry_key(window_geometry(&fixture.open.window))
    );
    assert_eq!(fixture.open.last_geometry.get(), geometry_key(accepted));
    pump_geometry(&fixture.open);
    assert!(
        fixture.take_inputs().is_empty(),
        "accepted acknowledgement suppresses redundant Resize"
    );
}

#[test]
fn stale_layout_tokens_and_generations_cannot_reconfigure_or_acknowledge_the_viewer() {
    let fixture = Fixture::new();
    fixture.select(10, VIEWER_MODE_HTML, 4);
    fixture.take_ack(10);
    let canvas = fixture.canvas();
    for (token, generation) in [(0, 9), (9, 5), (11, 3), (10, 4)] {
        fixture.select(token, 1, generation);
        pump_geometry(&fixture.open);
        assert!(fixture.open.window.get_web_mode());
        assert_canvas(fixture.canvas(), canvas);
        assert!(
            fixture.take_inputs().is_empty(),
            "stale token={token}, generation={generation}"
        );
    }
    fixture.select(11, VIEWER_MODE_HTML, 5);
    fixture.take_ack(11);
}

#[test]
fn close_retires_pending_layout_acknowledgement_and_ignores_later_controls() {
    let fixture = Fixture::new();
    fixture.accepting.set(false);
    fixture.select(12, VIEWER_MODE_HTML, 1);
    fixture.take_ack(12);
    fixture.accepting.set(true);
    fixture.deliver(vec![
        ViewerOutput::LayoutSelected {
            token: 13,
            mode: 1,
            content_generation: 2,
        },
        ViewerOutput::Closed,
    ]);
    pump_geometry(&fixture.open);
    fixture.select(14, VIEWER_MODE_HTML, 3);
    pump_geometry(&fixture.open);
    assert!(fixture.open.closed.get());
    assert!(fixture.take_inputs().is_empty());
    assert_eq!(fixture.open.frame.get(), 0);
    fixture.assert_no_opening_success();
}

#[test]
fn html_replacement_and_native_fallback_frames_keep_the_selected_canvas() {
    let fixture = Fixture::new();
    for (generation, mode) in [
        (1, VIEWER_MODE_HTML),
        (2, VIEWER_MODE_HTML),
        (3, 1),
        (4, 2),
        (5, 3),
    ] {
        fixture.select(20 + generation, mode, generation);
        let geometry = fixture.take_ack(20 + generation);
        let canvas = fixture.canvas();
        assert_eq!(fixture.open.window.get_web_mode(), mode == VIEWER_MODE_HTML);
        assert_eq!(fixture.open.window.get_source_mode(), mode == 2);
        assert_eq!(
            fixture.open.window.get_font_available(),
            mode != 3 && mode != VIEWER_MODE_HTML
        );
        fixture.deliver(vec![frame(generation, generation, mode)]);
        pump_geometry(&fixture.open);
        assert_eq!(fixture.open.frame_generation.get(), generation);
        assert_canvas(fixture.canvas(), canvas);
        assert_eq!(fixture.open.last_geometry.get(), geometry_key(geometry));
        assert!(
            fixture.take_inputs().is_empty(),
            "mode {mode} changed chrome after its first frame"
        );
    }
}

#[test]
fn layout_prepass_rejects_earlier_batch_frames_below_the_announced_generation_floor() {
    let fixture = Fixture::new();
    fixture.deliver(vec![
        frame(1, 100, 1),
        ViewerOutput::LayoutSelected {
            token: 30,
            mode: VIEWER_MODE_HTML,
            content_generation: 2,
        },
    ]);
    fixture.take_ack(30);
    assert!(fixture.open.window.get_web_mode());
    assert_eq!(fixture.open.frame.get(), 0);
    assert!(fixture.open.window.get_accessible_text().is_empty());
    fixture.assert_no_opening_success();
    let canvas = fixture.canvas();

    fixture.deliver(vec![frame(2, 101, 1)]);
    assert_eq!(
        fixture.open.frame.get(),
        0,
        "the selected generation also requires the selected mode"
    );
    assert!(fixture.open.window.get_web_mode());
    assert!(fixture.take_inputs().is_empty());
    fixture.deliver(vec![frame(2, 1, VIEWER_MODE_HTML)]);
    pump_geometry(&fixture.open);
    assert_eq!(
        fixture.open.frame.get(),
        1,
        "rejected frames cannot advance the revision fence"
    );
    assert_eq!(fixture.open.window.get_accessible_text(), "generation 2");
    assert_canvas(fixture.canvas(), canvas);
    assert!(fixture.take_inputs().is_empty());
}

#[test]
fn layout_metadata_cannot_grant_web_permissions_or_accept_content_input_before_a_frame() {
    let fixture = Fixture::new();
    fixture.select(40, VIEWER_MODE_HTML, 8);
    fixture.take_ack(40);
    let policy = || ViewerOutput::WebState {
        generation: 8,
        policy: viewer_web::Policy {
            network: true,
            scripts: true,
        },
        zoom: 1.0,
    };

    fixture.deliver(vec![policy()]);
    assert_eq!(fixture.open.frame.get(), 0);
    assert!(!fixture.open.window.get_web_network());
    assert!(!fixture.open.window.get_web_scripts());
    fixture.assert_no_opening_success();
    fixture.open.window.invoke_web_policy(true, true);
    assert!(!fixture.open.window.get_web_network());
    assert!(!fixture.open.window.get_web_scripts());
    assert!(!fixture.open.enqueue(ViewerInput::Web {
        frame: fixture.open.frame.get(),
        input: viewer_web::CommandKind::Text("not ready".into()),
    }));
    assert!(
        fixture.take_inputs().is_empty(),
        "neither the policy callback nor content input may reach the sender before a real frame"
    );

    fixture.deliver(vec![frame(8, 1, VIEWER_MODE_HTML), policy()]);
    assert_eq!(fixture.open.frame.get(), 1);
    assert!(fixture.open.window.get_web_network());
    assert!(fixture.open.window.get_web_scripts());
    assert!(fixture.open.enqueue(ViewerInput::Web {
        frame: fixture.open.frame.get(),
        input: viewer_web::CommandKind::Text("ready".into()),
    }));
    let inputs = fixture.take_inputs();
    assert!(
        matches!(inputs.as_slice(), [ViewerInput::Web {
            frame: 1, input: viewer_web::CommandKind::Text(text)
        }] if text == "ready"),
        "the matching frame releases the content gate"
    );
}

#[test]
fn image_failure_below_selected_layout_generation_keeps_current_route_state() {
    let fixture = Fixture::new();
    let info = ViewerImageInfo {
        surface: ViewerSurfaceId(7),
        origin: [1.0; 2],
        visible_source: [2.0; 2],
        raster_scale: 1.0,
        pan_sequence: 0,
        width: 4,
        height: 4,
        zoom: 1.0,
        fitted: false,
        page: 0,
        page_count: 1,
    };
    let ViewerOutput::Frame(mut previous) = frame(1, 1, VIEWER_MODE_IMAGE) else {
        unreachable!()
    };
    previous.image = Some(info.clone());
    previous.modes = vec![VIEWER_MODE_IMAGE];
    fixture.deliver(vec![ViewerOutput::Frame(previous)]);
    assert_eq!(fixture.open.frame_generation.get(), 1);
    assert_eq!(fixture.open.window.get_surface().size().width, 2);

    fixture.select(50, VIEWER_MODE_IMAGE, 2);
    fixture.take_ack(50);
    fixture.deliver(vec![ViewerOutput::Status("Preparing the new image".into())]);
    let context = fixture.open.window.get_position_context();
    let positions = fixture.open.text_position.borrow().positions();
    let context_available = fixture.open.text_position.borrow().context_available();
    let camera = fixture.open.image_camera.borrow().incoming(&info);
    let opening_successes = fixture
        .results
        .borrow()
        .iter()
        .filter(|result| result.is_ok())
        .count();

    fixture.deliver(vec![ViewerOutput::ImageUnavailable {
        generation: 1,
        surface: ViewerSurfaceId(0),
        message: "Stale image failure".into(),
    }]);
    assert_eq!(fixture.open.window.get_status(), "Preparing the new image");
    assert_eq!(fixture.open.frame_generation.get(), 1);
    assert_eq!(fixture.open.frame.get(), 1);
    assert_eq!(fixture.open.window.get_position_context(), context);
    assert_eq!(fixture.open.text_position.borrow().positions(), positions);
    assert_eq!(
        fixture.open.text_position.borrow().context_available(),
        context_available
    );
    assert_eq!(fixture.open.image_camera.borrow().incoming(&info), camera);
    assert_eq!(
        fixture
            .results
            .borrow()
            .iter()
            .filter(|result| result.is_ok())
            .count(),
        opening_successes,
        "a retired image failure cannot acknowledge the new route"
    );
    assert!(fixture.take_inputs().is_empty());

    fixture.deliver(vec![ViewerOutput::ImageUnavailable {
        generation: 2,
        surface: ViewerSurfaceId(0),
        message: "Current image failure".into(),
    }]);
    assert_eq!(fixture.open.window.get_status(), "Current image failure");
    assert_eq!(fixture.open.frame_generation.get(), 2);
}

fn retained_frame(generation: u64, revision: u64, surface: u64) -> ViewerOutput {
    let ViewerOutput::Frame(mut value) = frame(generation, revision, 2) else {
        unreachable!()
    };
    value.raster = Some(ViewerRasterInfo {
        surface: ViewerSurfaceId(surface),
        pixel_id: revision,
        origin: [0.0; 2],
        raster_origin: [0.0; 2],
        viewport: [2; 2],
        coverage_min: [0.0; 2],
        coverage_max: [0.0; 2],
        limit_min: [Some(0.0); 2],
        limit_max: [Some(0.0); 2],
        pan_sequence: 0,
        scroll_per_pixel: [0.1; 2],
        scroll_map: None,
        settled: true,
        gutter: None,
    });
    ViewerOutput::Frame(value)
}

#[test]
fn old_raster_rebase_in_new_layout_batch_cannot_block_the_matching_frame() {
    let fixture = Fixture::new();
    fixture.deliver(vec![retained_frame(1, 1, 90)]);
    assert!(fixture.open.window.get_retained_raster());
    assert_eq!(fixture.open.frame.get(), 1);
    assert_eq!(
        fixture
            .open
            .retained_raster
            .borrow()
            .camera
            .info()
            .map(|info| info.surface),
        Some(ViewerSurfaceId(90))
    );
    fixture.take_inputs();
    fixture.deliver(vec![
        ViewerOutput::LayoutSelected {
            token: 60,
            mode: VIEWER_MODE_HTML,
            content_generation: 2,
        },
        ViewerOutput::RasterRebase {
            surface: ViewerSurfaceId(90),
            transition: 1,
        },
    ]);
    fixture.take_ack(60);
    assert!(
        fixture.open.window.get_retained_raster(),
        "keep the old pixels until replacement"
    );
    assert!(!raster_rebase::waiting(&fixture.open));
    assert!(!fixture.open.raster_redraw.running());

    fixture.deliver(vec![frame(2, 2, VIEWER_MODE_HTML)]);
    pump_geometry(&fixture.open);
    assert_eq!(
        fixture.open.frame.get(),
        2,
        "the new route needs no retired rebase acknowledgement"
    );
    assert_eq!(fixture.open.frame_generation.get(), 2);
    assert!(!fixture.open.window.get_retained_raster());
    assert_eq!(fixture.open.window.get_accessible_text(), "generation 2");
    assert!(fixture.take_inputs().is_empty());
}

#[test]
fn old_raster_failure_during_layout_transition_cannot_replace_the_new_route_status() {
    let fixture = Fixture::new();
    fixture.deliver(vec![retained_frame(1, 1, 91)]);
    assert!(fixture.open.window.get_retained_raster());
    assert_eq!(
        fixture
            .open
            .retained_raster
            .borrow()
            .camera
            .info()
            .map(|info| info.surface),
        Some(ViewerSurfaceId(91))
    );
    fixture.take_inputs();
    fixture.select(61, VIEWER_MODE_HTML, 2);
    fixture.take_ack(61);
    fixture.deliver(vec![ViewerOutput::Status(
        "Preparing the new HTML route".into(),
    )]);
    let origin = fixture.open.retained_raster.borrow().camera.origin;
    let retained_surface = fixture
        .open
        .retained_raster
        .borrow()
        .camera
        .info()
        .map(|info| info.surface);
    let context = fixture.open.window.get_position_context();
    let unavailable = || ViewerOutput::RasterUnavailable {
        surface: ViewerSurfaceId(91),
        message: "Retired raster failed".into(),
    };
    fixture.deliver(vec![unavailable()]);
    assert_eq!(
        fixture.open.window.get_status(),
        "Preparing the new HTML route"
    );
    assert_eq!(fixture.open.window.get_position_context(), context);
    assert_eq!(fixture.open.retained_raster.borrow().camera.origin, origin);
    assert_eq!(
        fixture
            .open
            .retained_raster
            .borrow()
            .camera
            .info()
            .map(|info| info.surface),
        retained_surface
    );
    assert_eq!(fixture.open.frame_generation.get(), 1);
    assert!(!raster_rebase::waiting(&fixture.open));
    assert!(fixture.take_inputs().is_empty());

    fixture.deliver(vec![frame(2, 2, VIEWER_MODE_HTML)]);
    assert_eq!(fixture.open.frame.get(), 2);
    let current_status = fixture.open.window.get_status();
    fixture.deliver(vec![unavailable()]);
    assert_eq!(fixture.open.window.get_status(), current_status);
    assert_eq!(fixture.open.frame_generation.get(), 2);
    assert!(fixture.take_inputs().is_empty());
}

#[derive(Debug)]
struct LayoutPixels;
impl ViewerPixelOwner for LayoutPixels {}

fn tiled_image_frame(generation: u64, revision: u64, surface: u64) -> ViewerOutput {
    let ViewerOutput::Frame(mut value) = frame(generation, revision, VIEWER_MODE_IMAGE) else {
        unreachable!()
    };
    value.modes = vec![VIEWER_MODE_IMAGE];
    value.image = Some(ViewerImageInfo {
        surface: ViewerSurfaceId(surface),
        origin: [0.0; 2],
        visible_source: [2.0; 2],
        raster_scale: 1.0,
        pan_sequence: 0,
        width: 4,
        height: 4,
        zoom: 1.0,
        fitted: false,
        page: 0,
        page_count: 1,
    });
    value.pixels = Arc::from([]);
    value.tiles = Some(
        vec![ViewerTile {
            id: ViewerTileId(revision),
            position: [0.0; 2],
            width: 256,
            height: 256,
            pixels: Arc::new(
                ViewerPixels::new(vec![255; 256 * 256 * 4], Arc::new(LayoutPixels)).unwrap(),
            ),
        }]
        .into(),
    );
    ViewerOutput::Frame(value)
}

#[test]
fn image_motion_waits_for_the_new_layout_frame_and_resumes_on_its_surface() {
    let fixture = Fixture::new();
    fixture.deliver(vec![tiled_image_frame(1, 1, 70)]);
    assert!(fixture.open.window.get_tiled());
    fixture.take_inputs();
    image_motion::seek(&fixture.open, 1, 0.25);
    assert!(fixture.open.image_camera.borrow().origin[1] > 0.0);
    fixture.take_inputs();

    fixture.select(70, VIEWER_MODE_IMAGE, 2);
    fixture.take_ack(70);
    let origin = fixture.open.image_camera.borrow().origin;
    let offsets = [
        fixture.open.window.get_tile_offset_x(),
        fixture.open.window.get_tile_offset_y(),
    ];
    assert!(!image_motion::push(&fixture.open, 1.0, 1.0, true));
    assert!(!image_motion::push(&fixture.open, 1.0, 1.0, false));
    image_motion::seek(&fixture.open, 1, 0.9);
    image_motion::fallback(&fixture.open);
    assert_eq!(fixture.open.image_camera.borrow().origin, origin);
    assert_eq!(
        [
            fixture.open.window.get_tile_offset_x(),
            fixture.open.window.get_tile_offset_y()
        ],
        offsets
    );
    assert!(!fixture.open.image_redraw.running());
    assert!(
        fixture.take_inputs().is_empty(),
        "old image content cannot receive transition input"
    );

    fixture.deliver(vec![tiled_image_frame(2, 2, 71)]);
    assert_eq!(fixture.open.frame.get(), 2);
    assert_eq!(fixture.open.frame_generation.get(), 2);
    fixture.take_inputs();
    image_motion::seek(&fixture.open, 1, 0.75);
    assert!(fixture.open.image_camera.borrow().origin[1] > 0.0);
    assert!(
        fixture.take_inputs().iter().any(|input| matches!(
            input,
            ViewerInput::Image {
                frame: 2,
                input: ViewerImageInput::PanTo {
                    surface: ViewerSurfaceId(71),
                    ..
                }
            }
        )),
        "the matching frame restores input to its new surface"
    );
}
