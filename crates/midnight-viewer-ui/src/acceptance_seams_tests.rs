//! The acceptance seams reach a viewer as winit's events do: through the window's own
//! native observer, with frames that follow by themselves and the settings the suite chose
//! (`acceptance_seams`).
use super::*;
use crate::image_presentation_tests::{self, TestRuntime, TestWindows, advance_time};
use crate::text_presentation_tests::frame as text_frame;
use slint::winit_030::winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent},
};

/// Frames after a notch: far longer than its glide and the seam's settling time.
const FRAMES: usize = 60;
/// One notch down, in winit's lines.
const DOWN: NativeWheel = NativeWheel::Lines { x: 0.0, y: -1.0 };

/// A text viewer at rest whose backend inputs are captured.
fn text_viewer() -> (
    TestRuntime,
    TestWindows,
    Rc<OpenWindow>,
    Rc<RefCell<Vec<ViewerInput>>>,
) {
    let (runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    image_presentation_tests::present(&open, &service, &|_| {}, vec![text_frame(10, 0, 0.2)]);
    assert!(!open.window.get_web_mode() && !open.window.get_tiled());
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let capture = inputs.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        capture.borrow_mut().push(input);
        true
    });
    (runtime, manager, open, inputs)
}

/// The middle of the content, where the wheel moves the text.
fn middle(open: &OpenWindow) -> slint::LogicalPosition {
    let window = &open.window;
    slint::LogicalPosition::new(
        window.get_content_left() + window.get_content_width() / 2.0,
        window.get_content_top() + window.get_content_height() / 2.0,
    )
}

enum Step {
    Notch,
    Frame,
}

/// What a notch and each wheel frame after it sent the backend: the sum of its scrolls.
fn travel(inputs: &RefCell<Vec<ViewerInput>>, mut deliver: impl FnMut(Step)) -> Vec<[f32; 2]> {
    let sent = || {
        inputs
            .borrow_mut()
            .drain(..)
            .fold([0.0; 2], |sum, input| match input {
                ViewerInput::Scroll { x, y } => [sum[0] + x, sum[1] + y],
                _ => sum,
            })
    };
    inputs.borrow_mut().clear();
    deliver(Step::Notch);
    let mut frames = vec![sent()];
    for _ in 0..FRAMES {
        advance_time(midnight_ui_input::WHEEL_FRAME);
        deliver(Step::Frame);
        frames.push(sent());
    }
    frames
}

/// A seam notch moves the text as a native notch does (`observe_native_content` with a
/// wheel state of its own and a `RedrawRequested` per frame, as `web_wheel_tests` drives
/// it), frame for frame, while the seam's frames come by themselves with Slint's timers.
#[test]
fn a_seam_notch_moves_the_text_as_a_native_notch_frame_for_frame() {
    let (_runtime, manager, open, inputs) = text_viewer();
    let _settings = override_settings(SystemSettings {
        wheel: Some(WheelSetting::Lines(3)),
        reduce_motion: Some(false),
    });
    let at = middle(&open);
    let window = open.window.clone_strong();
    let seam = travel(&inputs, |step| match step {
        Step::Notch => assert_eq!(wheel(&manager, &window, at, DOWN), Some(true)),
        Step::Frame => slint::platform::update_timers_and_animations(),
    });
    // The seam sends no frames once the window rests; the wheel's speed starts over.
    advance_time(Duration::from_secs(2));
    slint::platform::update_timers_and_animations();
    let sent = open.seam.sent.get();
    for _ in 0..FRAMES {
        advance_time(midnight_ui_input::WHEEL_FRAME);
        slint::platform::update_timers_and_animations();
    }
    assert_eq!(open.seam.sent.get(), sent, "frames continue at rest");
    let mut scroll = midnight_ui_input::NativeScroll::new(&open.window);
    bind_kinetic(&open, &mut scroll);
    let scale = f64::from(open.window.window().scale_factor());
    let native = travel(&inputs, |step| match step {
        Step::Notch => {
            for event in [
                WindowEvent::CursorMoved {
                    device_id: DeviceId::dummy(),
                    position: PhysicalPosition::new(
                        f64::from(at.x) * scale,
                        f64::from(at.y) * scale,
                    ),
                },
                WindowEvent::MouseWheel {
                    device_id: DeviceId::dummy(),
                    delta: MouseScrollDelta::LineDelta(0.0, -1.0),
                    phase: TouchPhase::Moved,
                },
            ] {
                observe_native_content(&open, &event, true, &mut scroll);
            }
        }
        Step::Frame => {
            observe_native_content(&open, &WindowEvent::RedrawRequested, true, &mut scroll);
        }
    });
    assert_eq!(seam, native);
    let moving = seam.iter().filter(|sent| sent[1] != 0.0).count();
    assert!(moving > 2, "the notch did not glide: {seam:?}");
    let distance: f32 = seam.iter().map(|sent| sent[1]).sum();
    let expected = midnight_ui_input::notch_pixels_for(WheelSetting::Lines(3), 0.0) * scale as f32;
    assert!(
        (distance - expected).abs() < 0.5,
        "the notch moved {distance} of {expected}"
    );
    manager.close_all();
}

/// A retained text frame of `surface` with the camera at world y `origin` and pixels for
/// 600 rows above and below it; `rebase` acknowledges a corrected surface.
fn retained_frame(
    open: &OpenWindow,
    revision: u64,
    surface: u64,
    origin: f64,
    rebase: Option<u64>,
) -> ViewerOutput {
    let viewport = [
        open.window.get_content_width().round() as u32,
        open.window.get_content_height().round() as u32,
    ];
    let height = 1200 + viewport[1];
    let ViewerOutput::Frame(mut frame) = text_frame(revision, 0, 0.5) else {
        unreachable!()
    };
    frame.width = viewport[0];
    frame.height = height;
    frame.pixels = vec![255; viewport[0] as usize * height as usize * 4].into();
    frame.raster_rebase = rebase;
    frame.raster = Some(ViewerRasterInfo {
        surface: ViewerSurfaceId(surface),
        pixel_id: revision,
        origin: [0.0, origin],
        raster_origin: [0.0, origin - 600.0],
        viewport,
        coverage_min: [0.0, origin - 600.0],
        coverage_max: [0.0, origin + 600.0],
        limit_min: [Some(0.0), None],
        limit_max: [Some(0.0), None],
        pan_sequence: 0,
        scroll_per_pixel: [0.0, 1e-6],
        scroll_map: None,
        settled: true,
        gutter: None,
        bands: None,
    });
    ViewerOutput::Frame(frame)
}

/// `retained_frame` as prepared strips (`ViewerRasterBands`) of 256 rows, as the text,
/// Source, Markdown and hex views publish it.
fn banded_frame(open: &OpenWindow, revision: u64, surface: u64, origin: f64) -> ViewerOutput {
    let ViewerOutput::Frame(mut frame) = retained_frame(open, revision, surface, origin, None)
    else {
        unreachable!()
    };
    let info = frame.raster.as_mut().unwrap();
    let viewport = info.viewport;
    let top = ((origin - 600.0) / 256.0).floor() * 256.0;
    let count = ((origin + 600.0 + f64::from(viewport[1]) - top) / 256.0).ceil() as u32;
    let list: std::sync::Arc<[ViewerRasterBand]> = (0..count)
        .map(|index| ViewerRasterBand {
            id: 1_000_000 * surface + u64::from(index),
            top: top + f64::from(index * 256),
            height: 256,
            pixels: vec![200; viewport[0] as usize * 256 * 4].into(),
            gutter: None,
        })
        .collect();
    let bottom = top + f64::from(count * 256);
    info.raster_origin = [0.0, top];
    info.coverage_min = [0.0, top];
    info.coverage_max = [0.0, bottom - f64::from(viewport[1])];
    info.bands = Some(ViewerRasterBands {
        width: viewport[0],
        left: 0.0,
        gutter_width: 0,
        list,
    });
    frame.height = count * 256;
    frame.pixels = std::sync::Arc::from([]);
    ViewerOutput::Frame(frame)
}

/// The shown camera's world y.
fn camera(open: &OpenWindow) -> f64 {
    open.retained_raster.borrow().camera.origin[1]
}

/// What reaches the window as winit would deliver it: wheel notches and the frames
/// (`RedrawRequested`) on which their glide steps.
enum Driver {
    /// The seam's `wheel`, and the frames the seam then sends by itself.
    Seam(Rc<OpenWindow>),
    /// The native events straight to the window's observer, with a frame every wheel frame:
    /// winit's, which give one for each redraw a moving window asks for.
    Winit(Rc<OpenWindow>, Box<midnight_ui_input::NativeScroll>),
}

impl Driver {
    fn open(&self) -> &Rc<OpenWindow> {
        match self {
            Driver::Seam(open) | Driver::Winit(open, _) => open,
        }
    }

    /// `lines` of one wheel report in winit's units: a positive `y` scrolls up.
    fn wheel(&mut self, manager: &ViewerWindows, lines: f32) {
        let at = middle(self.open());
        match self {
            Driver::Seam(open) => {
                let window = open.window.clone_strong();
                let wheel = NativeWheel::Lines { x: 0.0, y: lines };
                assert_eq!(super::wheel(manager, &window, at, wheel), Some(true));
            }
            Driver::Winit(open, scroll) => {
                let scale = f64::from(open.window.window().scale_factor());
                for event in [
                    WindowEvent::CursorMoved {
                        device_id: DeviceId::dummy(),
                        position: PhysicalPosition::new(
                            f64::from(at.x) * scale,
                            f64::from(at.y) * scale,
                        ),
                    },
                    WindowEvent::MouseWheel {
                        device_id: DeviceId::dummy(),
                        delta: MouseScrollDelta::LineDelta(0.0, lines),
                        phase: TouchPhase::Moved,
                    },
                ] {
                    observe_native_content(open, &event, true, scroll);
                }
            }
        }
    }

    /// Let `duration` pass in wheel frames while Slint's timers run (the seam's too).
    fn run(&mut self, duration: Duration) {
        for _ in 0..duration.as_millis() / midnight_ui_input::WHEEL_FRAME.as_millis() {
            advance_time(midnight_ui_input::WHEEL_FRAME);
            slint::platform::update_timers_and_animations();
            if let Driver::Winit(open, scroll) = self {
                observe_native_content(open, &WindowEvent::RedrawRequested, true, scroll);
            }
        }
    }
}

/// After a notch down and a rest, a notch up whose glide waits for a corrected surface
/// (`RasterRebase`: a page read above the shown one changes the context of its first row,
/// as the first one above a scrollbar position did while that page started inside a row)
/// longer than the seam's settling time moves its whole distance once the surface comes,
/// and a burst after it moves all its notches.
fn notch_up_across_a_corrected_surface(seam: bool) {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let capture = inputs.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        capture.borrow_mut().push(input);
        true
    });
    let _settings = override_settings(SystemSettings {
        wheel: Some(WheelSetting::Lines(3)),
        reduce_motion: Some(false),
    });
    let present = |outputs| image_presentation_tests::present(&open, &service, &|_| {}, outputs);
    present(vec![retained_frame(&open, 10, 70, 4000.0, None)]);
    assert!(open.window.get_retained_raster());
    let mut driver = if seam {
        Driver::Seam(open.clone())
    } else {
        let mut scroll = midnight_ui_input::NativeScroll::new(&open.window);
        bind_kinetic(&open, &mut scroll);
        Driver::Winit(open.clone(), Box::new(scroll))
    };
    let scale = f64::from(open.window.window().scale_factor());
    let notch = f64::from(midnight_ui_input::notch_pixels_for(
        WheelSetting::Lines(3),
        0.0,
    )) * scale;
    // One notch down, then a rest.
    let start = camera(&open);
    driver.wheel(&manager, -1.0);
    driver.run(Duration::from_secs(1));
    let down = camera(&open) - start;
    assert!((down - notch).abs() < 0.5, "a notch down moved {down:.1}");
    // One notch up: two frames of its glide, then the worker corrects its page and the
    // window hands it the camera it shows.
    let start = camera(&open);
    driver.wheel(&manager, 1.0);
    driver.run(2 * midnight_ui_input::WHEEL_FRAME);
    let shown = camera(&open);
    assert!(shown < start, "the glide did not start");
    inputs.borrow_mut().clear();
    present(vec![ViewerOutput::RasterRebase {
        surface: ViewerSurfaceId(70),
        transition: 1,
    }]);
    let handed = inputs.borrow().iter().find_map(|input| match input {
        ViewerInput::RasterRebase {
            transition: 1,
            origin,
        } => Some(origin[1]),
        _ => None,
    });
    assert_eq!(
        handed,
        Some(shown),
        "the window hands over the camera it shows"
    );
    // The corrected surface comes later than the seam's settling time after the notch.
    driver.run(Duration::from_millis(400));
    present(vec![retained_frame(&open, 11, 71, shown, Some(1))]);
    driver.run(Duration::from_secs(1));
    let up = camera(&open) - start;
    assert!(
        (up + notch).abs() < 0.5,
        "a notch up moved {up:.1} of {:.1}",
        -notch
    );
    // Five notches down after it, at once.
    let start = camera(&open);
    for _ in 0..5 {
        driver.wheel(&manager, -1.0);
    }
    driver.run(Duration::from_secs(1));
    let burst = camera(&open) - start;
    assert!(
        (burst - 5.0 * notch).abs() < 0.5,
        "five notches moved {burst:.1}"
    );
    if seam {
        // At rest the seam sends no frames.
        let sent = open.seam.sent.get();
        driver.run(Duration::from_millis(500));
        assert_eq!(open.seam.sent.get(), sent, "frames continue at rest");
    }
    drop(driver);
    manager.close_all();
}

/// The window keeps the whole travel when winit gives it its frames: it asks for them
/// again when the corrected surface comes.
#[test]
fn a_notch_up_across_a_corrected_surface_moves_its_whole_distance_with_winits_frames() {
    notch_up_across_a_corrected_surface(false);
}

/// The seam too. It ended its frames while the glide waited for the corrected surface, as
/// if the window rested, so the rest of the glide got none: the view stayed 15 to 42 px up
/// and did not move on (acceptance VIEW-NOTCH-UP).
#[test]
fn a_notch_up_across_a_corrected_surface_moves_its_whole_distance_with_the_seams_frames() {
    notch_up_across_a_corrected_surface(true);
}

/// An HTML page's browser reads the pointer only from winit's events: the seam's move,
/// press and release reach it as a native pointer does (the HTML link check's hover and
/// clicks), where Slint's own pointer events reach only Slint.
#[test]
fn seam_pointer_reaches_an_html_page_as_winits_pointer_does() {
    let (_runtime, manager, open, inputs) = text_viewer();
    open.window.set_image_mode(false);
    open.window.set_web_mode(true);
    let window = open.window.clone_strong();
    let at = middle(&open);
    let pointers = || -> Vec<(viewer_web::PointerKind, viewer_web::PointerButton)> {
        inputs
            .borrow_mut()
            .drain(..)
            .filter_map(|input| match input {
                ViewerInput::Web {
                    input: viewer_web::CommandKind::Pointer(pointer),
                    ..
                } => Some((pointer.kind, pointer.button)),
                _ => None,
            })
            .collect()
    };
    inputs.borrow_mut().clear();
    use viewer_web::{PointerButton as Button, PointerKind as Kind};
    for (action, expected) in [
        (NativePointer::Move, vec![(Kind::Move, Button::None)]),
        (
            NativePointer::Press,
            vec![(Kind::Move, Button::None), (Kind::Down, Button::Left)],
        ),
        (
            NativePointer::Release,
            vec![(Kind::Move, Button::None), (Kind::Up, Button::Left)],
        ),
    ] {
        assert_eq!(pointer(&manager, &window, at, action), Some(true));
        assert_eq!(pointers(), expected, "{action:?}");
    }
    // Outside the page the browser hears nothing of a move.
    let outside = slint::LogicalPosition::new(at.x, open.window.get_content_top() - 4.0);
    pointer(&manager, &window, outside, NativePointer::Move);
    assert_eq!(pointers(), []);
    manager.close_all();
}

/// One notch down, one up and five down, each after the previous came to rest, as the
/// acceptance suite's `viewers_scroll` sends them: the five in one burst, then again a
/// frame apart. Every movement goes its whole distance at the glide scale `glide` (0:
/// Options › Scrolling glide Off, or the system's less motion).
fn down_up_and_five_down(seam: bool, glide: f32, banded: bool) {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    *open.input.borrow_mut() = Rc::new(|_| true);
    let _settings = override_settings(SystemSettings {
        wheel: Some(WheelSetting::Lines(3)),
        reduce_motion: Some(false),
    });
    midnight_ui_input::set_glide_scale(glide);
    let mut driver = if seam {
        Driver::Seam(open.clone())
    } else {
        let mut scroll = midnight_ui_input::NativeScroll::new(&open.window);
        bind_kinetic(&open, &mut scroll);
        Driver::Winit(open.clone(), Box::new(scroll))
    };
    let notch = f64::from(midnight_ui_input::notch_pixels_for(
        WheelSetting::Lines(3),
        0.0,
    )) * f64::from(open.window.window().scale_factor());
    let mut moved = Vec::new();
    for (surface, frame_apart) in [(70, false), (71, true)] {
        // Pixels 600 rows around the camera, wherever the previous sequence left it.
        let origin = if surface == 70 { 4000.0 } else { camera(&open) };
        image_presentation_tests::present(
            &open,
            &service,
            &|_| {},
            vec![if banded {
                banded_frame(&open, surface, surface, origin)
            } else {
                retained_frame(&open, surface, surface, origin, None)
            }],
        );
        assert!(open.window.get_retained_raster());
        driver.run(Duration::from_millis(100));
        for (count, lines) in [(1, -1.0), (1, 1.0), (5, -1.0)] {
            let start = camera(&open);
            for _ in 0..count {
                driver.wheel(&manager, lines);
                if frame_apart {
                    driver.run(midnight_ui_input::WHEEL_FRAME);
                }
            }
            driver.run(Duration::from_secs(1));
            moved.push((camera(&open) - start).round());
        }
    }
    midnight_ui_input::set_glide_scale(1.0);
    let once = [notch.round(), -notch.round(), (5.0 * notch).round()];
    assert_eq!(
        moved,
        [once, once].concat(),
        "seam {seam}, glide {glide}, strips {banded}: down, up, five down; twice"
    );
    drop(driver);
    manager.close_all();
}

#[test]
fn down_up_and_five_down_move_their_whole_distances_through_the_seam() {
    down_up_and_five_down(true, 1.0, false);
}

#[test]
fn down_up_and_five_down_move_their_whole_distances_with_winits_frames() {
    down_up_and_five_down(false, 1.0, false);
}

#[test]
fn down_up_and_five_down_move_their_whole_distances_without_gliding() {
    down_up_and_five_down(true, 0.0, false);
}

#[test]
fn down_up_and_five_down_move_their_whole_distances_over_strips() {
    down_up_and_five_down(true, 1.0, true);
}

#[test]
fn down_up_and_five_down_move_their_whole_distances_over_strips_with_winits_frames() {
    down_up_and_five_down(false, 1.0, true);
}

#[test]
fn down_up_and_five_down_move_their_whole_distances_with_a_long_glide() {
    down_up_and_five_down(true, 4.0, false);
}

/// The pointer moving over the text while a wheel glide runs asks the worker what it is
/// over (links, `links.rs`); that question, and the next one once the answer comes, must
/// not stop the glide. They did: every input other than a camera request counted as
/// interrupting motion, so a burst of five notches (each with the pointer's position, as
/// winit reports it) moved 0 to 5 px of 500 once the first hover answer came back
/// (acceptance `viewers_scroll`, release run of 23 November).
fn glide_with_hovers(seam: bool) {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let capture = inputs.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        capture.borrow_mut().push(input);
        true
    });
    let _settings = override_settings(SystemSettings {
        wheel: Some(WheelSetting::Lines(3)),
        reduce_motion: Some(false),
    });
    let present = |outputs| image_presentation_tests::present(&open, &service, &|_| {}, outputs);
    present(vec![banded_frame(&open, 10, 80, 4000.0)]);
    assert!(open.window.get_retained_raster());
    let mut driver = if seam {
        Driver::Seam(open.clone())
    } else {
        let mut scroll = midnight_ui_input::NativeScroll::new(&open.window);
        bind_kinetic(&open, &mut scroll);
        Driver::Winit(open.clone(), Box::new(scroll))
    };
    let notch = f64::from(midnight_ui_input::notch_pixels_for(
        WheelSetting::Lines(3),
        0.0,
    )) * f64::from(open.window.window().scale_factor());
    let start = camera(&open);
    let at = middle(&open);
    for _ in 0..5 {
        driver.wheel(&manager, -1.0);
        // The pointer's position reaches the text as winit reports it before each notch.
        open.window.invoke_hover_point(at.x, at.y);
    }
    let hovers = |inputs: &RefCell<Vec<ViewerInput>>| {
        inputs
            .borrow_mut()
            .drain(..)
            .filter(|input| {
                matches!(
                    input,
                    ViewerInput::Raster {
                        input: ViewerRasterInput::Hover { .. },
                        ..
                    }
                )
            })
            .count()
    };
    assert!(hovers(&inputs) >= 1, "the pointer asked what it is over");
    // The worker answers during the glide; the window asks its next question.
    driver.run(3 * midnight_ui_input::WHEEL_FRAME);
    present(vec![ViewerOutput::LinkHover(None)]);
    assert!(hovers(&inputs) >= 1, "the next hover question");
    driver.run(Duration::from_secs(1));
    let moved = (camera(&open) - start).round();
    assert_eq!(
        moved,
        (5.0 * notch).round(),
        "seam {seam}: the burst moved {moved}"
    );
    drop(driver);
    manager.close_all();
}

#[test]
fn hover_questions_during_a_glide_leave_it_its_whole_distance_through_the_seam() {
    glide_with_hovers(true);
}

#[test]
fn hover_questions_during_a_glide_leave_it_its_whole_distance_with_winits_frames() {
    glide_with_hovers(false);
}

/// Overridden settings apply when the window next gains the focus, as the system's do, and
/// end with their guard: one screen per notch and less motion move a page step (87.5 % of
/// the view) at once.
#[test]
fn overridden_settings_apply_from_the_next_focus_until_their_guard_drops() {
    let (_runtime, manager, open, inputs) = text_viewer();
    let window = open.window.clone_strong();
    let at = middle(&open);
    {
        let _settings = override_settings(SystemSettings {
            wheel: Some(WheelSetting::Page),
            reduce_motion: Some(true),
        });
        assert_eq!(focus(&manager, &window, true), Some(()));
        assert_eq!(image_motion::wheel_setting(&open), WheelSetting::Page);
        assert!(image_motion::reduce_motion(&open));
        let frames = travel(&inputs, |step| match step {
            Step::Notch => {
                wheel(&manager, &window, at, DOWN);
            }
            Step::Frame => slint::platform::update_timers_and_animations(),
        });
        let moving: Vec<_> = frames
            .iter()
            .enumerate()
            .filter(|(_, sent)| sent[1] != 0.0)
            .collect();
        assert_eq!(moving.len(), 1, "less motion glides: {frames:?}");
        assert!(moving[0].0 <= 1, "moved first on frame {}", moving[0].0);
        // One screen per notch moves Chromium's page step: 87.5 % of the view.
        let screen = midnight_ui_input::notch_pixels_for(
            WheelSetting::Page,
            open.window.get_content_height(),
        ) * open.window.window().scale_factor();
        assert!(
            (moving[0].1[1] - screen).abs() < 0.5,
            "a notch moved {} of a {screen} pixel page step",
            moving[0].1[1]
        );
    }
    // The guard dropped: the next focus reads the system and the host again.
    assert_eq!(focus(&manager, &window, true), Some(()));
    assert_eq!(
        image_motion::wheel_setting(&open),
        crate::wheel_setting::wheel_setting()
    );
    assert!(!image_motion::reduce_motion(&open));
    manager.close_all();
}

/// Guards restore what was in force before them, in the reverse order of their creation.
#[test]
fn nested_overrides_restore_in_turn() {
    let outer = SystemSettings {
        wheel: Some(WheelSetting::Lines(1)),
        reduce_motion: None,
    };
    let inner = SystemSettings {
        wheel: None,
        reduce_motion: Some(true),
    };
    let before = (wheel_override(), reduce_motion_override());
    let first = override_settings(outer);
    let second = override_settings(inner);
    assert_eq!(
        (wheel_override(), reduce_motion_override()),
        (None, Some(true))
    );
    drop(second);
    assert_eq!(
        (wheel_override(), reduce_motion_override()),
        (Some(WheelSetting::Lines(1)), None)
    );
    drop(first);
    assert_eq!((wheel_override(), reduce_motion_override()), before);
}

/// Focus reaches the window as winit reports it: an inactive window paints its selection
/// with the theme's inactive pair (`appearance::activation_changed`), and Slint's window
/// is active as the focus is.
#[test]
fn focus_reaches_the_window_as_winit_reports_it() {
    let (_runtime, manager, open, _inputs) = text_viewer();
    let window = open.window.clone_strong();
    let active = || WindowInner::from_pub(open.window.window()).active();
    assert_eq!(focus(&manager, &window, true), Some(()));
    assert!(open.active.get() && active());
    assert_eq!(focus(&manager, &window, false), Some(()));
    assert!(!open.active.get() && !active());
    assert_eq!(focus(&manager, &window, true), Some(()));
    assert!(open.active.get() && active());
    manager.close_all();
}

/// Native keys make native digit shortcuts authoritative over Slint's (`ShortcutState`),
/// but a window that winit does not back gets no native keys: after seam events, its
/// Slint keys still resolve digit shortcuts.
#[test]
fn seam_events_leave_slint_keys_in_charge_of_shortcuts() {
    let (_runtime, manager, open, _inputs) = text_viewer();
    let window = open.window.clone_strong();
    let asked = Rc::new(RefCell::new(Vec::new()));
    let record = asked.clone();
    *open.key_resolver.borrow_mut() = Rc::new(move |chord: &ShortcutChord, _| {
        record.borrow_mut().push(chord.key.clone());
        None
    });
    assert_eq!(focus(&manager, &window, false), Some(()));
    assert_eq!(focus(&manager, &window, true), Some(()));
    assert_eq!(wheel(&manager, &window, middle(&open), DOWN), Some(true));
    let mut key = i_slint_core::input::KeyEvent::default();
    key.text = "1".into();
    key.modifiers.control = true;
    open.window.invoke_input_key(key, false);
    assert_eq!(
        *asked.borrow(),
        ["Digit1"],
        "a seam event took the Slint keys"
    );
    manager.close_all();
}

/// A window these viewers did not open, or one that is closed, gets nothing.
#[test]
fn other_and_closed_windows_get_nothing() {
    let (_runtime, manager, open, _inputs) = text_viewer();
    let at = middle(&open);
    let other = ViewerWindow::new().unwrap();
    assert_eq!(focus(&manager, &other, true), None);
    assert_eq!(wheel(&manager, &other, at, DOWN), None);
    let window = open.window.clone_strong();
    manager.close_all();
    assert_eq!(focus(&manager, &window, true), None);
    assert_eq!(wheel(&manager, &window, at, DOWN), None);
}
