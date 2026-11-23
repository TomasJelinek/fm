//! Wheel glide and touchpad momentum through the comparator window's own native
//! scroll input and content handler, with a controlled clock.
use super::*;
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use slint::winit_030::winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent},
    keyboard::ModifiersState,
};

/// A typical libinput touchpad report on X11: 5.35 px over a 15 px line.
const FINGER_LINE: f32 = -0.3567;

thread_local! {
    static CLOCK: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

struct ClockPlatform;
impl Platform for ClockPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        window.set_size(slint::PhysicalSize::new(900, 600));
        Ok(window)
    }
    fn duration_since_start(&self) -> Duration {
        CLOCK.with(Cell::get)
    }
}

fn wait(time: Duration) {
    CLOCK.with(|clock| clock.set(clock.get() + time));
}

struct Fixture {
    window: ComparatorWindow,
    scroll: midnight_ui_input::NativeScroll,
    scrolled: Rc<RefCell<Vec<(f32, f32)>>>,
    at: slint::LogicalPosition,
}

impl Fixture {
    fn new() -> Self {
        slint::platform::set_platform(Box::new(ClockPlatform)).unwrap();
        let window = ComparatorWindow::new().unwrap();
        window.show().unwrap();
        window.window().set_size(slint::PhysicalSize::new(900, 600));
        let scrolled = Rc::new(RefCell::new(Vec::new()));
        let received = scrolled.clone();
        window.on_scroll(move |x, y| received.borrow_mut().push((x, y)));
        let at = slint::LogicalPosition::new(
            window.get_content_left() + window.get_content_width() / 2.0,
            window.get_content_top() + window.get_content_height() / 2.0,
        );
        assert!(window.get_content_width() > 100.0 && window.get_content_height() > 100.0);
        let mut fixture = Self {
            scroll: native_scroll(&window).synthesize_momentum(true),
            window,
            scrolled,
            at,
        };
        fixture.point(at);
        fixture
    }

    fn point(&mut self, at: slint::LogicalPosition) {
        self.at = at;
        self.native(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(at.x.into(), at.y.into()),
        });
    }

    fn notch(&mut self) -> bool {
        self.native(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, -1.0),
            phase: TouchPhase::Moved,
        })
    }

    fn native(&mut self, event: WindowEvent) -> bool {
        self.scroll.observe(self.window.window(), &event, None)
    }

    fn take(&self) -> Vec<f32> {
        self.scrolled
            .borrow_mut()
            .drain(..)
            .map(|(_, y)| y)
            .collect()
    }

    /// One presented frame; `progress` models the worker moving the text position.
    fn frame(&mut self, progress: bool) -> Vec<f32> {
        wait(Duration::from_millis(8));
        self.native(WindowEvent::RedrawRequested);
        let steps = self.take();
        if progress && !steps.is_empty() {
            self.window.set_position(self.window.get_position() + 0.001);
        }
        steps
    }

    /// Wayland touchpad report. Input the hook leaves alone reaches the content
    /// natively, as Slint would deliver it.
    fn finger(&mut self, y: f32, phase: TouchPhase) {
        wait(Duration::from_millis(10));
        let consumed = self.native(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, y.into())),
            phase,
        });
        if !consumed && y != 0.0 {
            self.window
                .window()
                .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                    position: self.at,
                    delta_x: 0.0,
                    delta_y: y,
                });
        }
    }

    fn flick(&mut self, y: f32) {
        self.finger(y, TouchPhase::Started);
        for _ in 0..5 {
            self.finger(y, TouchPhase::Moved);
        }
        self.finger(0.0, TouchPhase::Ended);
    }
}

#[test]
fn comparison_wheel_notch_glides_in_small_steps_with_the_exact_total() {
    let mut fixture = Fixture::new();
    assert!(fixture.notch());
    assert!(fixture.take().is_empty(), "no jump before the first frame");
    let mut steps = Vec::new();
    for _ in 0..40 {
        steps.extend(fixture.frame(false));
    }
    assert!(steps.len() >= 10, "{} glide frames", steps.len());
    assert!(
        steps.iter().all(|&step| step < 0.0 && step > -15.0),
        "{steps:?}"
    );
    // The platform's notch: 100 px at Windows' three lines, Chromium's fixed 120 on Linux.
    let notch =
        midnight_ui_input::notch_pixels_for(midnight_ui_input::WheelSetting::default(), 0.0);
    let total: f32 = steps.iter().sum();
    assert!((total + notch).abs() < 0.01, "{total} of {}", -notch);
}

#[test]
fn comparison_touchpad_flick_coasts_and_stops_at_the_document_end_or_a_click() {
    let mut fixture = Fixture::new();
    fixture.flick(-20.0);
    assert_eq!(
        fixture.take(),
        vec![-20.0; 6],
        "finger motion arrives unchanged"
    );
    // The worker keeps moving the text: momentum continues and slows down.
    let mut previous = f32::INFINITY;
    for frame in 0..30 {
        let steps = fixture.frame(true);
        assert_eq!(
            steps.len(),
            1,
            "frame {frame}: one coalesced scroll per frame"
        );
        let step = -steps[0];
        assert!(step > 0.0 && step < 20.0, "frame {frame}: {step}");
        assert!(step <= previous + 0.001, "frame {frame}: friction");
        previous = step;
    }
    // The 0.8 s time constant keeps most of the speed: 240 ms after the lift an
    // 8 ms frame still moves about 12 of the 16 px it started with.
    assert!(previous > 9.0, "the glide keeps going: {previous}");
    // The document end: the position stops changing there, so momentum ends. (Between the
    // edges a still position is a wait for the worker's next surface, not an end.)
    fixture.window.set_position(1.0);
    let mut frames = 0;
    while !fixture.frame(false).is_empty() {
        frames += 1;
        assert!(frames < 30, "momentum must stop at a stalled document end");
    }
    assert!(!fixture.scroll.kinetic_active());

    fixture.flick(-20.0);
    fixture.take();
    assert!(!fixture.frame(true).is_empty());
    fixture.native(WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state: ElementState::Pressed,
        button: MouseButton::Left,
    });
    assert!(!fixture.scroll.kinetic_active());
    for _ in 0..10 {
        assert!(
            fixture.frame(true).is_empty(),
            "a click stops momentum at once"
        );
    }
}

#[test]
fn ctrl_wheel_over_comparison_reaches_the_content_once_instead_of_gliding() {
    let mut fixture = Fixture::new();
    fixture.native(WindowEvent::ModifiersChanged(
        ModifiersState::CONTROL.into(),
    ));
    // Image comparisons zoom once per Ctrl+wheel event: one notch is one event.
    assert!(!fixture.notch(), "the notch stays native");
    for _ in 0..40 {
        assert!(
            fixture.frame(false).is_empty(),
            "no glide frames to repeat it"
        );
    }
    fixture.native(WindowEvent::ModifiersChanged(
        ModifiersState::empty().into(),
    ));
    assert!(fixture.notch(), "an unmodified notch glides again");
}

#[test]
fn x11_momentum_reaches_a_host_sink_in_lines() {
    let mut fixture = Fixture::new();
    let sunk = Rc::new(RefCell::new(Vec::new()));
    let received = sunk.clone();
    fixture.scroll.set_kinetic_sink(Box::new(move |event| {
        if let WindowEvent::MouseWheel { delta, .. } = event {
            received.borrow_mut().push(*delta);
        }
        true
    }));
    for _ in 0..6 {
        wait(Duration::from_millis(10));
        assert!(fixture.native(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, FINGER_LINE),
            phase: TouchPhase::Moved,
        }));
    }
    // Finger reports reach the content directly, at the shared line distance.
    let finger = fixture.take();
    assert_eq!(finger.len(), 6);
    assert!(
        finger
            .iter()
            .all(|&step| (step - FINGER_LINE * midnight_ui_input::LINE_PIXELS).abs() < 0.01)
    );
    for _ in 0..10 {
        assert!(fixture.frame(true).is_empty(), "the sink took the momentum");
    }
    let sunk = sunk.borrow();
    assert!(!sunk.is_empty(), "the stream ended and momentum followed");
    for delta in sunk.iter() {
        // A browser converts lines with its own distance, as it did the finger's.
        let MouseScrollDelta::LineDelta(x, y) = *delta else {
            panic!("momentum of a line stream must stay in lines: {delta:?}");
        };
        assert_eq!(x, 0.0);
        assert!(y < 0.0 && y > FINGER_LINE * 2.0, "{y}");
    }
}

#[test]
fn a_flick_over_the_toolbar_has_no_momentum() {
    let mut fixture = Fixture::new();
    let toolbar = slint::LogicalPosition::new(fixture.at.x, 2.0);
    assert!(toolbar.y < fixture.window.get_content_top());
    fixture.point(toolbar);
    fixture.flick(-20.0);
    fixture.take();
    assert!(fixture.frame(true).is_empty());
    assert!(
        !fixture.scroll.kinetic_active(),
        "momentum needs scrollable content"
    );
}
