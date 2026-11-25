//! HTML wheel input travels like the text viewer's: notches glide through the shared
//! wheel motion and reach the page as one pixel delta per frame, touchpad motion and
//! its momentum go straight through, and both stop where the page stops painting. A
//! glide waits while the page stalls and ends only once it stays still at its edge.
use super::image_presentation_tests::{
    self, TestRuntime, TestWindows, advance_time, present as present_outputs,
};
use super::text_presentation_tests::frame as text_frame;
use super::*;
use slint::winit_030::winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent},
    keyboard::ModifiersState,
};
use std::sync::Arc;

const FRAME: Duration = Duration::from_millis(8);
const HTML_GENERATION: u64 = 8;

struct Fixture {
    open: Rc<OpenWindow>,
    service: Service,
    scroll: midnight_ui_input::NativeScroll,
    inputs: Rc<RefCell<Vec<ViewerInput>>>,
    revision: u64,
    pixels: Option<(u32, u32, Arc<[u8]>)>,
}

/// A notch `frames` presented frames after the previous one, in wheel lines.
#[derive(Clone, Copy)]
struct Notch {
    frames: u32,
    lines: (f32, f32),
}

fn down(frames: u32) -> Notch {
    Notch {
        frames,
        lines: (0.0, -1.0),
    }
}

fn wheel(delta: MouseScrollDelta, phase: TouchPhase) -> WindowEvent {
    WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta,
        phase,
    }
}

fn notch(lines: f32) -> WindowEvent {
    wheel(MouseScrollDelta::LineDelta(0.0, lines), TouchPhase::Moved)
}

fn sum(deltas: &[[f64; 2]]) -> [f64; 2] {
    deltas.iter().fold([0.0; 2], |total, delta| {
        [total[0] + delta[0], total[1] + delta[1]]
    })
}

/// One notch's logical distance along each axis, as the text viewer scrolls it
/// (`image_motion::notch_pixels`).
fn notch_distance(open: &OpenWindow) -> [f64; 2] {
    [true, false].map(|horizontal| f64::from(image_motion::notch_pixels(open, horizontal)))
}

impl Fixture {
    fn new(scale: f32) -> (TestRuntime, TestWindows, Self) {
        let (runtime, manager, open, service) = image_presentation_tests::open();
        open.timer.stop();
        if scale != 1.0 {
            open.window
                .window()
                .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
                    scale_factor: scale,
                });
            open.window
                .window()
                .set_size(slint::LogicalSize::new(900.0, 600.0));
        }
        let inputs = Rc::new(RefCell::new(Vec::new()));
        let capture = inputs.clone();
        *open.input.borrow_mut() = Rc::new(move |input| {
            capture.borrow_mut().push(input);
            true
        });
        let mut scroll =
            midnight_ui_input::NativeScroll::new(&open.window).synthesize_momentum(true);
        bind_kinetic(&open, &mut scroll);
        let fixture = Fixture {
            open,
            service,
            scroll,
            inputs,
            revision: 10,
            pixels: None,
        };
        (runtime, manager, fixture)
    }

    fn native(&mut self, event: WindowEvent) -> bool {
        observe_native_content(&self.open, &event, true, &mut self.scroll)
    }

    fn scale(&self) -> f64 {
        f64::from(self.open.window.window().scale_factor())
    }

    /// Point at the middle of the content, where the wheel acts on the page.
    fn point(&mut self) {
        let window = &self.open.window;
        let scale = window.window().scale_factor();
        let x = (window.get_content_left() + window.get_content_width() / 2.0) * scale;
        let y = (window.get_content_top() + window.get_content_height() / 2.0) * scale;
        self.native(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x.into(), y.into()),
        });
    }

    fn text(&mut self) {
        present_outputs(
            &self.open,
            &self.service,
            &|_| {},
            vec![text_frame(self.revision, 0, 0.2)],
        );
        assert!(!self.open.window.get_web_mode());
        self.point();
        self.inputs.borrow_mut().clear();
    }

    /// A newly painted page frame the size of the canvas, as the browser sends it.
    fn paint(&mut self) {
        self.revision += 1;
        let scale = self.open.window.window().scale_factor();
        let width = (self.open.window.get_content_width() * scale).round() as u32;
        let height = (self.open.window.get_content_height() * scale).round() as u32;
        let pixels = match &self.pixels {
            Some((w, h, pixels)) if (*w, *h) == (width, height) => pixels.clone(),
            _ => {
                let pixels: Arc<[u8]> = vec![255; width as usize * height as usize * 4].into();
                self.pixels = Some((width, height, pixels.clone()));
                pixels
            }
        };
        let frame = ViewerFrame {
            generation: HTML_GENERATION,
            raster: None,
            raster_rebase: None,
            media_control: None,
            position_sequence: [0; 2],
            revision: self.revision,
            mode: VIEWER_MODE_HTML,
            modes: vec![VIEWER_MODE_HTML],
            wrap_enabled: false,
            wrap_available: false,
            image: None,
            width,
            height,
            pixels,
            tiles: None,
            accessible_text: "HTML".into(),
            position: 0.0,
            vertical_extent: 1.0,
            horizontal_position: 0.0,
            horizontal_extent: 1.0,
            caption: "HTML document".into(),
            status: "Offline".into(),
        };
        present_outputs(
            &self.open,
            &self.service,
            &|_| {},
            vec![ViewerOutput::Frame(Box::new(frame))],
        );
        assert_eq!(self.open.frame.get(), self.revision);
    }

    fn web(&mut self) {
        self.paint();
        assert!(self.open.window.get_web_mode());
        self.point();
        self.inputs.borrow_mut().clear();
    }

    /// Scroll deltas sent since the last call, in content pixels (positive moves the
    /// view down or right): the text viewer's backend scrolls, or the page's wheels.
    fn take(&self) -> Vec<[f64; 2]> {
        self.inputs
            .borrow_mut()
            .drain(..)
            .filter_map(|input| match input {
                ViewerInput::Scroll { x, y } => Some([f64::from(x), f64::from(y)]),
                ViewerInput::Web {
                    input: viewer_web::CommandKind::Wheel(wheel),
                    ..
                } => Some([-wheel.delta_x, -wheel.delta_y]),
                _ => None,
            })
            .collect()
    }

    fn wheels(&self) -> Vec<viewer_web::Wheel> {
        self.inputs
            .borrow_mut()
            .drain(..)
            .filter_map(|input| match input {
                ViewerInput::Web {
                    input: viewer_web::CommandKind::Wheel(wheel),
                    ..
                } => Some(wheel),
                _ => None,
            })
            .collect()
    }

    /// One presented frame and what it sent; `painted`: the page then paints anew.
    fn frame(&mut self, painted: bool) -> Vec<[f64; 2]> {
        advance_time(FRAME);
        self.native(WindowEvent::RedrawRequested);
        let steps = self.take();
        if painted {
            self.paint();
        }
        steps
    }

    /// The notches, then frames until the motion is long over: the sum sent for each
    /// presented frame, including what a notch itself sent before it. The page, if
    /// any, paints every eighth frame, so it keeps moving.
    fn travel(&mut self, notches: &[Notch]) -> Vec<[f64; 2]> {
        let web = self.open.window.get_web_mode();
        let mut frames = Vec::new();
        let mut sent = Vec::new();
        let mut frame = |fixture: &mut Self, sent: &mut Vec<[f64; 2]>| {
            sent.extend(fixture.frame(web && frames.len() % 8 == 7));
            frames.push(sum(&std::mem::take(sent)));
        };
        for notch in notches {
            for _ in 0..notch.frames {
                frame(self, &mut sent);
            }
            let (x, y) = notch.lines;
            self.native(wheel(MouseScrollDelta::LineDelta(x, y), TouchPhase::Moved));
            sent.extend(self.take());
        }
        for _ in 0..60 {
            frame(self, &mut sent);
        }
        frames
    }
}

/// Notch schedules: one, a fast flick of three and of ten (24 ms apart), a reversal
/// and a horizontal notch.
fn schedules() -> Vec<(&'static str, Vec<Notch>)> {
    let flick = |count: usize| {
        std::iter::once(down(0))
            .chain(std::iter::repeat_n(down(3), count - 1))
            .collect::<Vec<_>>()
    };
    vec![
        ("one notch", flick(1)),
        ("three notches", flick(3)),
        ("ten notches", flick(10)),
        (
            "reversal",
            vec![
                down(0),
                down(2),
                Notch {
                    frames: 4,
                    lines: (0.0, 1.0),
                },
            ],
        ),
        (
            "horizontal",
            vec![Notch {
                frames: 0,
                lines: (-1.0, 0.0),
            }],
        ),
    ]
}

fn same_travel_as_the_text_viewer(scale: f32) {
    let (_runtime, manager, mut fixture) = Fixture::new(scale);
    let scale = fixture.scale();
    fixture.text();
    let text: Vec<_> = schedules()
        .into_iter()
        .map(|(_, notches)| fixture.travel(&notches))
        .collect();
    fixture.web();
    for ((name, notches), expected) in schedules().into_iter().zip(text) {
        let html = fixture.travel(&notches);
        assert_eq!(html.len(), expected.len(), "{name}");
        // A flick's notches 24 ms apart pace its glides to 50 ms: three notches move for
        // 13 frames of 8 ms, one for 25.
        let moving = expected.iter().filter(|delta| **delta != [0.0; 2]).count();
        assert!(
            moving >= 10,
            "{name}: the text viewer glides ({moving} frames)"
        );
        for (index, (html, text)) in html.iter().zip(&expected).enumerate() {
            for axis in 0..2 {
                assert!(
                    (html[axis] - text[axis]).abs() <= 1e-3 * scale.max(text[axis].abs()),
                    "{name} at {scale}x, frame {index}: page {html:?}, text viewer {text:?}"
                );
            }
        }
        if name != "reversal" {
            let total = sum(&html);
            let lines = notches.iter().fold([0.0; 2], |total, notch| {
                [
                    total[0] - f64::from(notch.lines.0),
                    total[1] - f64::from(notch.lines.1),
                ]
            });
            let notch = notch_distance(&fixture.open);
            for axis in 0..2 {
                assert!(
                    (total[axis] - lines[axis] * notch[axis] * scale).abs() < 1e-3 * scale,
                    "{name}: {total:?} for {lines:?} lines at {scale}x"
                );
            }
        }
    }
    manager.close_all();
}

#[test]
fn html_wheel_notches_glide_one_delta_per_frame_like_the_text_viewer() {
    same_travel_as_the_text_viewer(1.0);
}

#[test]
fn html_wheel_notches_match_the_text_viewer_at_twice_the_scale() {
    same_travel_as_the_text_viewer(2.0);
}

/// The frames, counted from 1, that sent the page a delta.
fn sending(frames: &[Vec<[f64; 2]>]) -> Vec<usize> {
    frames
        .iter()
        .enumerate()
        .filter(|(_, sent)| !sent.is_empty())
        .map(|(index, _)| index + 1)
        .collect()
}

#[test]
fn a_notch_glide_waits_for_the_pages_first_scroll_frame() {
    let (_runtime, manager, mut fixture) = Fixture::new(1.0);
    fixture.web();
    let scale = fixture.scale();
    // The page paints every delta: the whole notch arrives, one delta per frame.
    fixture.native(notch(-1.0));
    let mut moving = Vec::new();
    for _ in 0..40 {
        moving.extend(fixture.frame(true));
    }
    assert_eq!(moving.len(), 25, "200 ms of 8 ms frames");
    let notch_y = notch_distance(&fixture.open)[1] * scale;
    assert!((sum(&moving)[1] - notch_y).abs() < 1e-3);
    // A page that has not painted since the glide began may still be preparing its first
    // scroll frame, which took Chromium up to 650 ms: the glide pauses 120 ms after its
    // first delta and goes on once that frame is shown, here 504 ms after the notch.
    fixture.native(notch(-1.0));
    let waiting: Vec<_> = (1..=100).map(|frame| fixture.frame(frame >= 63)).collect();
    let sent = sending(&waiting);
    assert_eq!(sent[..15], (1..=15).collect::<Vec<_>>(), "{sent:?}");
    assert!(
        sent[15] > 63,
        "nothing goes out before the page paints: {sent:?}"
    );
    assert!((sum(&waiting.concat())[1] - notch_y).abs() < 1e-3);
    assert!(!web_input::gliding(&fixture.open));
    // A page already at its edge never paints: the glide pauses 120 ms after its first
    // delta, whatever notches follow. A second after it began, with no frame since, the
    // page gets the rest at once, here one page of a burst of 42 notches, which does
    // nothing at an edge, and the glide ends, instead of sending deltas into the edge
    // until 50 ms (twice the notches' 24 ms interval) after its last notch (129 frames).
    let page = f64::from(fixture.open.window.get_content_height()) * scale;
    fixture.native(notch(-1.0));
    let mut idle = Vec::new();
    let mut ended = None;
    for frame in 1..=160 {
        idle.push(fixture.frame(false));
        if frame % 3 == 0 && frame < 124 {
            fixture.native(notch(-1.0));
        }
        if ended.is_none() && !web_input::gliding(&fixture.open) {
            ended = Some(frame);
        }
    }
    let sent = sending(&idle);
    assert_eq!(
        sent,
        [(1..=15).collect::<Vec<usize>>(), vec![125]].concat(),
        "{sent:?}"
    );
    assert!(
        (idle[124][0][1] - page).abs() < 1.0,
        "{:?} of {page}",
        idle[124]
    );
    assert_eq!(ended, Some(126), "one second without a painted frame");
    // Focus loss stops a glide at once, as in the text viewer.
    fixture.native(notch(1.0));
    assert_eq!(fixture.frame(true).len(), 1);
    fixture.native(WindowEvent::Focused(false));
    for _ in 0..30 {
        assert!(fixture.frame(true).is_empty(), "focus loss stops the glide");
    }
    manager.close_all();
}

/// A page whose first scroll paints only after 1.5 s (on a loaded Windows machine first
/// content took seconds): the glide pauses 120 ms after its first delta and, a second after
/// it began without a frame since, hands the page the rest at once instead of dropping it,
/// so the notch moves its whole distance once the page paints.
#[test]
fn a_notch_moves_whole_on_a_page_whose_first_scroll_paints_after_a_second() {
    let (_runtime, manager, mut fixture) = Fixture::new(1.0);
    fixture.web();
    let notch_y = notch_distance(&fixture.open)[1] * fixture.scale();
    fixture.native(notch(-1.0));
    // No frame for 1.5 s (187 frames of 8 ms), then one on every frame.
    let frames: Vec<_> = (1..=240).map(|frame| fixture.frame(frame >= 188)).collect();
    let sent = sending(&frames);
    assert_eq!(
        sent,
        [(1..=15).collect::<Vec<usize>>(), vec![125]].concat(),
        "{sent:?}"
    );
    let total = sum(&frames.concat())[1];
    assert!((total - notch_y).abs() < 1e-3, "{total} of {notch_y}");
    assert!(!web_input::gliding(&fixture.open));
    manager.close_all();
}

/// The page moves for four frames and reaches its bottom while more notches arrive 40 ms
/// apart: the deltas stop 120 ms after its last new frame instead of going on until 80 ms
/// (twice the notches' interval) after the last notch (25 frames), and the glide ends,
/// having sent nothing more, [`web_input::EDGE`] after the first delta the page did not
/// paint (old rule: it ended after 120 ms).
#[test]
fn a_glide_into_the_page_edge_pauses_then_ends_within_the_bound() {
    let (_runtime, manager, mut fixture) = Fixture::new(1.0);
    fixture.web();
    fixture.native(notch(-1.0));
    let mut edge = Vec::new();
    let mut ended = None;
    for frame in 1..=200 {
        edge.push(fixture.frame(frame <= 4));
        if matches!(frame, 5 | 10 | 15) {
            fixture.native(notch(-1.0));
        }
        if !web_input::gliding(&fixture.open) {
            ended = Some(frame);
            break;
        }
    }
    let sent = sending(&edge);
    assert_eq!(sent, (1..=19).collect::<Vec<_>>(), "until the 120 ms stall");
    // The first delta the page did not paint went out on frame 5.
    let bound = (web_input::EDGE.as_nanos() / FRAME.as_nanos()) as usize;
    assert_eq!(ended, Some(5 + bound), "{} ms of frames", 8 * (5 + bound));
    manager.close_all();
}

/// One notch down whose first delta the page paints; then `stall` frames without a newer
/// page frame, during which the glide waits, and 40 frames that each paint again. What each
/// frame sent.
fn through_stall(fixture: &mut Fixture, stall: usize) -> Vec<Vec<[f64; 2]>> {
    fixture.native(notch(-1.0));
    let mut frames = vec![fixture.frame(true)];
    frames.extend((0..stall).map(|_| fixture.frame(false)));
    assert!(web_input::gliding(&fixture.open), "the stalled glide waits");
    frames.extend((0..40).map(|_| fixture.frame(true)));
    frames
}

/// Chromium paints nothing mid-scroll for 300 ms (a loaded machine): the glide's deltas stop
/// 120 ms after its last new frame, it keeps the rest, and once the page paints again the
/// rest glides on, so the notch arrives whole (old rule: about nine tenths of it), without
/// a jump.
#[test]
fn a_glide_through_a_300_ms_page_stall_still_arrives_whole() {
    let (_runtime, manager, mut fixture) = Fixture::new(1.0);
    fixture.web();
    let notch_y = notch_distance(&fixture.open)[1] * fixture.scale();
    // 38 frames, 304 ms, without a newer frame after the one with the first delta.
    let frames = through_stall(&mut fixture, 38);
    let sent = sending(&frames);
    // The delta of frame 2 went out after the last new frame.
    assert_eq!(sent[..16], (1..=16).collect::<Vec<_>>(), "{sent:?}");
    assert!(sent[16] > 40, "nothing goes out during the stall: {sent:?}");
    let deltas = frames.concat();
    let total = sum(&deltas)[1];
    assert!((total - notch_y).abs() < 1e-3, "{total} of {notch_y}");
    // The rest glides on from where it waited: no step is larger than the glide's first.
    let first = deltas[0][1];
    assert!(
        deltas[16..]
            .iter()
            .all(|delta| delta[1] > 0.0 && delta[1] <= first),
        "{deltas:?}"
    );
    assert!(!web_input::gliding(&fixture.open));
    manager.close_all();
}

/// A stall followed by a reversal: after a stall the glide still finishes its notch, and
/// a notch up brings the page all the way back. A notch up during the stall goes out on
/// the next frame, before the page paints again, and the waiting rest of the notch down
/// never does.
#[test]
fn a_reversal_after_or_during_a_page_stall_goes_back_at_once() {
    let (_runtime, manager, mut fixture) = Fixture::new(1.0);
    fixture.web();
    let notch_y = notch_distance(&fixture.open)[1] * fixture.scale();
    let down = sum(&through_stall(&mut fixture, 38).concat())[1];
    assert!((down - notch_y).abs() < 1e-3, "{down} of {notch_y}");
    fixture.native(notch(1.0));
    let mut up = Vec::new();
    for _ in 0..40 {
        up.extend(fixture.frame(true));
    }
    assert!((sum(&up)[1] + notch_y).abs() < 1e-3, "{up:?}");
    // 200 ms into a stall, the glide waiting for the page, a notch up.
    fixture.native(notch(-1.0));
    let mut before = vec![fixture.frame(true)];
    before.extend((0..25).map(|_| fixture.frame(false)));
    assert_eq!(sending(&before), (1..=16).collect::<Vec<_>>());
    let held = notch_y - sum(&before.concat())[1];
    assert!(held > 1.0, "{held} px wait");
    fixture.native(notch(1.0));
    let first = fixture.frame(false);
    assert!(first.len() == 1 && first[0][1] < 0.0, "{first:?}");
    let mut after = first;
    for _ in 0..40 {
        after.extend(fixture.frame(true));
    }
    assert!(after.iter().all(|delta| delta[1] < 0.0), "{after:?}");
    assert!((sum(&after)[1] + notch_y).abs() < 1e-3, "{after:?}");
    assert!(!web_input::gliding(&fixture.open));
    manager.close_all();
}

/// Notches that pile up while the page is stalled do not reach it until it paints again,
/// and then no more than one page of them glides on, over several frames: a long stall
/// never ends in a runaway scroll (old rule: the piled notches went out at once).
#[test]
fn notches_piling_up_during_a_page_stall_resume_at_most_one_page() {
    let (_runtime, manager, mut fixture) = Fixture::new(1.0);
    fixture.web();
    let scale = fixture.scale();
    let notch_y = notch_distance(&fixture.open)[1] * scale;
    let page = f64::from(fixture.open.window.get_content_height()) * scale;
    // 448 ms without a newer frame after the first delta, a notch every 24 ms.
    fixture.native(notch(-1.0));
    let mut stalled = vec![fixture.frame(true)];
    for frame in 1..=56 {
        stalled.push(fixture.frame(false));
        if frame % 3 == 0 {
            fixture.native(notch(-1.0));
        }
    }
    assert_eq!(sending(&stalled), (1..=16).collect::<Vec<_>>());
    let held = 19.0 * notch_y - sum(&stalled.concat())[1];
    assert!(held > page, "{held} px wait, a page is {page} px");
    fixture.paint();
    let resumed: Vec<_> = (0..60).map(|_| fixture.frame(true)).collect();
    let travel = sum(&resumed.concat())[1];
    assert!(
        (travel - page).abs() < 1.0,
        "{travel} px after the stall, a page is {page} px"
    );
    let steps = sending(&resumed).len();
    assert!(steps >= 8, "one page glides over {steps} frames");
    assert!(
        resumed.concat().iter().all(|delta| delta[1] < page / 2.0),
        "{resumed:?}"
    );
    assert!(!web_input::gliding(&fixture.open));
    manager.close_all();
}

#[test]
fn touchpad_momentum_over_the_page_stops_once_the_page_stops_painting() {
    let (_runtime, manager, mut fixture) = Fixture::new(1.0);
    fixture.web();
    let scale = fixture.scale();
    // `painted`: the page paints each finger report, as it does while it moves.
    let flick = |fixture: &mut Fixture, painted: bool| {
        advance_time(Duration::from_millis(500));
        for index in 0..7 {
            advance_time(Duration::from_millis(10));
            let (delta, phase) = match index {
                0 => (-20.0, TouchPhase::Started),
                6 => (0.0, TouchPhase::Ended),
                _ => (-20.0, TouchPhase::Moved),
            };
            fixture.native(wheel(
                MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, delta * scale)),
                phase,
            ));
            if painted && index < 6 {
                fixture.paint();
            }
        }
        let finger: Vec<_> = fixture
            .take()
            .into_iter()
            .filter(|delta| *delta != [0.0; 2])
            .collect();
        assert_eq!(finger.len(), 6, "finger motion: one delta per report");
        for delta in finger {
            assert!(delta[0] == 0.0 && (delta[1] - 20.0 * scale).abs() < 1e-3);
        }
    };
    let coast = |fixture: &mut Fixture| {
        let mut frames = 0;
        while !fixture.frame(false).is_empty() {
            frames += 1;
            assert!(
                frames < 300,
                "momentum must stop at a page that does not move"
            );
        }
        frames
    };
    // The page keeps painting: momentum keeps going, one delta per frame.
    flick(&mut fixture, true);
    for index in 0..40 {
        assert_eq!(fixture.frame(true).len(), 1, "momentum frame {index}");
    }
    // A notch replaces the momentum with its glide.
    fixture.native(notch(1.0));
    for _ in 0..40 {
        fixture.frame(true);
    }
    // The page moved with the finger and stops painting at its edge: momentum stops
    // 120 ms later instead of coasting on for seconds.
    flick(&mut fixture, true);
    let frames = coast(&mut fixture);
    assert!(
        frames <= 17,
        "{frames} momentum frames after the page stopped"
    );
    assert!(!fixture.scroll.kinetic_active());
    // A page that has not painted since the stream began may still be preparing its
    // first scroll frame: momentum keeps going for a second after the stream began,
    // then stops 120 ms later at a page that is already at its edge.
    flick(&mut fixture, false);
    let frames = coast(&mut fixture);
    assert!((120..=140).contains(&frames), "{frames} momentum frames");
    assert!(!fixture.scroll.kinetic_active());
    manager.close_all();
}

#[test]
fn touchpad_lines_and_acting_wheels_reach_the_page_at_once() {
    let (_runtime, manager, mut fixture) = Fixture::new(2.0);
    fixture.web();
    let scale = fixture.scale();
    // X11 finger lines: one delta per report at the shared line distance.
    for _ in 0..4 {
        advance_time(Duration::from_millis(10));
        assert!(fixture.native(notch(-0.2633)));
        let wheels = fixture.wheels();
        assert_eq!(wheels.len(), 1);
        let line = f64::from(midnight_ui_input::LINE_PIXELS);
        assert!(
            (wheels[0].delta_y + 0.2633 * line * scale).abs() < 1e-3,
            "{}",
            wheels[0].delta_y
        );
    }
    // Ctrl+wheel zooms in Chromium once per event: one unchanged event, no glide.
    advance_time(Duration::from_millis(500));
    fixture.native(WindowEvent::ModifiersChanged(
        ModifiersState::CONTROL.into(),
    ));
    assert!(fixture.native(notch(1.0)));
    let wheels = fixture.wheels();
    assert_eq!(wheels.len(), 1);
    assert!(wheels[0].modifiers.control);
    assert!((wheels[0].delta_y - 40.0 * scale).abs() < 1e-3);
    for _ in 0..30 {
        assert!(
            fixture.frame(false).is_empty(),
            "a zoom notch does not glide"
        );
    }
    manager.close_all();
}

/// With the system's animation effects off (`ViewerWindows::set_motion_preference`), a
/// notch reaches the text viewer and the page whole on the next frame, without a glide.
#[test]
fn with_reduced_motion_a_notch_moves_whole_on_the_next_frame() {
    let (_runtime, manager, mut fixture) = Fixture::new(1.0);
    manager.set_motion_preference(Rc::new(|| true));
    notch_moves_whole_on_the_next_frame(&mut fixture);
    manager.close_all();
}

/// Options › Appearance › Scrolling glide Off (`midnight_ui_input::glide_scale` 0) does
/// the same, at once in an open window and without the system's setting.
#[test]
fn with_the_glide_off_a_notch_moves_whole_on_the_next_frame() {
    let (_runtime, manager, mut fixture) = Fixture::new(1.0);
    fixture.text();
    // Gliding while the setting is on; Off applies to the already open window.
    fixture.native(notch(-1.0));
    let mut frames = vec![sum(&fixture.take())];
    for _ in 0..30 {
        frames.push(sum(&fixture.frame(false)));
    }
    assert!(
        frames.iter().filter(|delta| delta[1] != 0.0).count() > 2,
        "{frames:?}"
    );
    midnight_ui_input::set_glide_scale(0.0);
    notch_moves_whole_on_the_next_frame(&mut fixture);
    midnight_ui_input::set_glide_scale(1.0);
    manager.close_all();
}

fn notch_moves_whole_on_the_next_frame(fixture: &mut Fixture) {
    for web in [false, true] {
        if web {
            fixture.web();
        } else {
            fixture.text();
        }
        let distance = notch_distance(&fixture.open)[1] * fixture.scale();
        fixture.native(notch(-1.0));
        let mut frames = vec![sum(&fixture.take())];
        for _ in 0..30 {
            frames.push(sum(&fixture.frame(false)));
        }
        let moving: Vec<_> = frames
            .iter()
            .enumerate()
            .filter(|(_, delta)| delta[1] != 0.0)
            .collect();
        assert_eq!(moving.len(), 1, "web {web}: {frames:?}");
        assert!(
            moving[0].0 <= 1,
            "web {web}: moved first on frame {}",
            moving[0].0
        );
        assert!(
            (moving[0].1[1] - distance).abs() < 0.5,
            "web {web}: {} of {distance}",
            moving[0].1[1]
        );
    }
}

/// With the system's animation effects off, a touchpad flick over the page stops with the
/// finger: no momentum is synthesised.
#[test]
fn with_reduced_motion_a_touchpad_flick_has_no_momentum() {
    let (_runtime, manager, mut fixture) = Fixture::new(1.0);
    manager.set_motion_preference(Rc::new(|| true));
    touchpad_flick_has_no_momentum(&mut fixture);
    manager.close_all();
}

/// Scrolling glide Off: a touchpad flick over the page stops with the finger too.
#[test]
fn with_the_glide_off_a_touchpad_flick_has_no_momentum() {
    let (_runtime, manager, mut fixture) = Fixture::new(1.0);
    midnight_ui_input::set_glide_scale(0.0);
    touchpad_flick_has_no_momentum(&mut fixture);
    midnight_ui_input::set_glide_scale(1.0);
    manager.close_all();
}

fn touchpad_flick_has_no_momentum(fixture: &mut Fixture) {
    fixture.web();
    for _ in 0..6 {
        advance_time(Duration::from_millis(10));
        fixture.native(wheel(
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -20.0)),
            TouchPhase::Moved,
        ));
    }
    advance_time(Duration::from_millis(10));
    fixture.native(wheel(
        MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 0.0)),
        TouchPhase::Ended,
    ));
    fixture.take();
    let mut sent = Vec::new();
    for _ in 0..30 {
        sent.extend(fixture.frame(true));
    }
    assert!(
        sent.is_empty(),
        "momentum after the finger lifted: {sent:?}"
    );
}

/// Everything one input sends the text viewer or the page: its own and later frames.
fn input_travel(fixture: &mut Fixture, input: WindowEvent) -> f64 {
    let web = fixture.open.window.get_web_mode();
    fixture.native(input);
    let mut total = sum(&fixture.take());
    for _ in 0..60 {
        total = sum(&[total, sum(&fixture.frame(web))]);
    }
    total[1]
}

/// Options › Appearance › Scrolling speed (`midnight_ui_input::wheel_speed_scale`): a
/// notch in the text viewer and over a page travels that share of the system's notch at
/// each stop, applied at once in the open window; touchpad pixels travel as before.
#[test]
fn the_wheel_speed_scales_a_notch_but_not_touchpad_pixels() {
    let (_runtime, manager, mut fixture) = Fixture::new(1.0);
    for web in [false, true] {
        if web {
            fixture.web();
        } else {
            fixture.text();
        }
        let system = notch_distance(&fixture.open)[1] * fixture.scale();
        let finger = input_travel(
            &mut fixture,
            wheel(
                MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -20.0)),
                TouchPhase::Moved,
            ),
        );
        for scale in [0.5, 0.75, 1.0, 1.5, 2.0] {
            midnight_ui_input::set_wheel_speed_scale(scale);
            let moved = input_travel(&mut fixture, notch(-1.0));
            assert!(
                (moved - system * f64::from(scale)).abs() < 0.5,
                "web {web}, speed {scale}: {moved} of {system}"
            );
            let pixels = input_travel(
                &mut fixture,
                wheel(
                    MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -20.0)),
                    TouchPhase::Moved,
                ),
            );
            assert!(
                (pixels - finger).abs() < 0.01,
                "web {web}, speed {scale}: touchpad {pixels}, not {finger}"
            );
        }
        midnight_ui_input::set_wheel_speed_scale(1.0);
    }
    manager.close_all();
}
