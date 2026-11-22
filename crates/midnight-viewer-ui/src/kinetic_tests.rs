//! Touchpad momentum through the viewer's native hook into the backend scroll path.
use super::image_presentation_tests::{self, advance_time, present as present_outputs};
use super::text_presentation_tests::frame;
use super::*;
use slint::winit_030::winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent},
};

struct Fixture {
    open: Rc<OpenWindow>,
    scroll: midnight_ui_input::NativeScroll,
    scrolls: Rc<RefCell<Vec<f32>>>,
    at: slint::LogicalPosition,
}

impl Fixture {
    fn native(&mut self, event: WindowEvent) -> bool {
        observe_native_content(&self.open, &event, true, &mut self.scroll)
    }

    fn take(&self) -> Vec<f32> {
        self.scrolls.borrow_mut().drain(..).collect()
    }

    /// One presented frame; `progress` models the worker acknowledging a new position.
    fn frame(&mut self, progress: bool) -> Vec<f32> {
        advance_time(Duration::from_millis(8));
        self.native(WindowEvent::RedrawRequested);
        let steps = self.take();
        if progress && !steps.is_empty() {
            let window = &self.open.window;
            window.set_position(window.get_position() + 0.001);
        }
        steps
    }

    /// Wayland touchpad report. Input the hook leaves alone reaches the content
    /// natively, as Slint would deliver it.
    fn finger(&mut self, y: f32, phase: TouchPhase) {
        advance_time(Duration::from_millis(10));
        let scale = self.open.window.window().scale_factor();
        let consumed = self.native(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, (y * scale).into())),
            phase,
        });
        if !consumed && y != 0.0 {
            self.open.window.window().dispatch_event(
                slint::platform::WindowEvent::PointerScrolled {
                    position: self.at,
                    delta_x: 0.0,
                    delta_y: y,
                },
            );
        }
    }

    fn point(&mut self, at: slint::LogicalPosition) {
        let scale = self.open.window.window().scale_factor();
        self.at = at;
        self.native(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new((at.x * scale).into(), (at.y * scale).into()),
        });
    }

    fn flick(&mut self, y: f32) {
        self.finger(y, TouchPhase::Started);
        for _ in 0..5 {
            self.finger(y, TouchPhase::Moved);
        }
        self.finger(0.0, TouchPhase::Ended);
    }
}

fn fixture() -> (
    image_presentation_tests::TestRuntime,
    image_presentation_tests::TestWindows,
    Fixture,
) {
    let (runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    present_outputs(&open, &service, &|_| {}, vec![frame(10, 0, 0.2)]);
    let scrolls = Rc::new(RefCell::new(Vec::new()));
    let received = scrolls.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        if let ViewerInput::Scroll { y, .. } = input {
            received.borrow_mut().push(y);
        }
        true
    });
    let mut scroll = midnight_ui_input::NativeScroll::new(&open.window).synthesize_momentum(true);
    bind_kinetic(&open, &mut scroll);
    let at = slint::LogicalPosition::new(
        open.window.get_content_left() + open.window.get_content_width() / 2.0,
        open.window.get_content_top() + open.window.get_content_height() / 2.0,
    );
    let mut fixture = Fixture {
        open,
        scroll,
        scrolls,
        at,
    };
    fixture.point(at);
    (runtime, manager, fixture)
}

#[test]
fn text_viewer_touchpad_flick_coasts_through_backend_scrolls_and_stops_at_the_end() {
    let (_runtime, manager, mut fixture) = fixture();
    let scale = fixture.open.window.window().scale_factor();
    fixture.flick(-20.0);
    assert_eq!(fixture.take(), vec![20.0 * scale; 6], "finger motion");
    let mut previous = f32::INFINITY;
    for index in 0..30 {
        let steps = fixture.frame(true);
        assert_eq!(
            steps.len(),
            1,
            "frame {index}: one backend scroll per frame"
        );
        let step = steps[0] / scale;
        assert!(step > 0.0 && step < 20.0, "frame {index}: {step}");
        assert!(step <= previous + 0.001, "frame {index}: friction");
        previous = step;
    }
    // The 0.8 s time constant keeps most of the speed: 240 ms after the lift an
    // 8 ms frame still moves about 12 of the 16 px it started with.
    assert!(previous > 9.0, "the glide keeps going: {previous}");
    // The document end: the position reaches the bottom and stops changing there.
    fixture.open.window.set_position(1.0);
    let mut frames = 0;
    while !fixture.frame(false).is_empty() {
        frames += 1;
        assert!(frames < 30, "momentum must stop at a stalled document end");
    }
    assert!(!fixture.scroll.kinetic_active());
    manager.close_all();
}

/// The worker defers motion while it fetches the next page: the shown position stands
/// still mid-document for longer than momentum's stall timeout. That is no edge: the glide
/// goes on, and only the same standstill at the end stops it.
#[test]
fn a_page_that_takes_long_to_arrive_mid_document_does_not_end_the_glide() {
    let (_runtime, manager, mut fixture) = fixture();
    fixture.flick(-20.0);
    fixture.take();
    for index in 0..5 {
        assert_eq!(fixture.frame(true).len(), 1, "frame {index}");
    }
    // 40 frames of 8 ms without a new position: far past the 120 ms stall timeout.
    for index in 0..40 {
        assert_eq!(
            fixture.frame(false).len(),
            1,
            "frame {index}: the glide goes on while the page is fetched"
        );
    }
    assert!(fixture.scroll.kinetic_active());
    fixture.open.window.set_position(1.0);
    let mut frames = 0;
    while !fixture.frame(false).is_empty() {
        frames += 1;
        assert!(frames < 30, "momentum must stop at a stalled document end");
    }
    manager.close_all();
}

#[test]
fn text_viewer_click_stops_momentum_and_x11_lines_skip_wheel_easing() {
    let (_runtime, manager, mut fixture) = fixture();
    let scale = fixture.open.window.window().scale_factor();
    // X11: fractional line deltas (here 3.95 px over libinput's 15 px line) reach the
    // content's pixel path directly, not the wheel glide.
    for _ in 0..6 {
        advance_time(Duration::from_millis(10));
        assert!(
            fixture.native(WindowEvent::MouseWheel {
                device_id: DeviceId::dummy(),
                delta: MouseScrollDelta::LineDelta(0.0, -0.2633),
                phase: TouchPhase::Moved,
            }),
            "touchpad lines are delivered by the shared observer"
        );
    }
    let finger = fixture.take();
    assert_eq!(finger.len(), 6, "one backend scroll per report: {finger:?}");
    for step in finger {
        assert!(
            (step / scale - 0.2633 * midnight_ui_input::LINE_PIXELS).abs() < 0.01,
            "{step}"
        );
    }
    assert!(fixture.scroll.kinetic_active());
    // The stream ends after a short gap and momentum takes over.
    let mut steps = Vec::new();
    for _ in 0..8 {
        steps.extend(fixture.frame(true));
    }
    assert!(!steps.is_empty() && steps.iter().all(|&step| step > 0.0));
    fixture.native(WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state: ElementState::Pressed,
        button: MouseButton::Left,
    });
    fixture.native(WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state: ElementState::Released,
        button: MouseButton::Left,
    });
    assert!(!fixture.scroll.kinetic_active());
    for _ in 0..10 {
        assert!(
            fixture.frame(true).is_empty(),
            "a click stops momentum at once"
        );
    }
    manager.close_all();
}

#[test]
fn a_flick_over_the_toolbar_has_no_momentum() {
    let (_runtime, manager, mut fixture) = fixture();
    let window = &fixture.open.window;
    let toolbar = slint::LogicalPosition::new(
        window.get_content_left() + window.get_content_width() / 2.0,
        2.0,
    );
    assert!(toolbar.y < window.get_content_top());
    fixture.point(toolbar);
    fixture.flick(-20.0);
    fixture.take();
    assert!(
        fixture.frame(true).is_empty(),
        "no momentum outside the content"
    );
    assert!(!fixture.scroll.kinetic_active());
    manager.close_all();
}
