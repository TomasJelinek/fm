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
    // The seam's frames end once the window rests; the wheel's speed starts over.
    advance_time(Duration::from_secs(2));
    slint::platform::update_timers_and_animations();
    assert!(!open.seam.frames.running(), "frames continue at rest");
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

/// Overridden settings apply when the window next gains the focus, as the system's do, and
/// end with their guard: one screen per notch and less motion move a screen at once.
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
        let screen = open.window.get_content_height() * open.window.window().scale_factor();
        assert!(
            (moving[0].1[1] - screen).abs() < 0.5,
            "a notch moved {} of a {screen} pixel screen",
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
