//! Home and End over an image larger than the window go to its start and end along the
//! direction it scrolls in. The retained camera owns a tiled image's view, so they move
//! it as the scrollbar does instead of sending the backend a Seek it would not show.
use super::image_presentation_tests::{self, present as present_outputs};
use super::*;

/// The camera origins the window asked the backend to paint since the last call.
fn pans(inputs: &RefCell<Vec<ViewerInput>>) -> Vec<[f64; 2]> {
    let pans = inputs
        .borrow()
        .iter()
        .filter_map(|input| match input {
            ViewerInput::Image {
                input: ViewerImageInput::PanTo { origin, .. },
                ..
            } => Some(*origin),
            _ => None,
        })
        .collect();
    assert!(
        !inputs
            .borrow()
            .iter()
            .any(|input| matches!(input, ViewerInput::Seek(_))),
        "a Seek would leave the retained image where it is"
    );
    inputs.borrow_mut().clear();
    pans
}

#[test]
fn home_and_end_go_to_the_top_and_bottom_of_a_tall_image_and_across_a_wide_one() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let capture = inputs.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        capture.borrow_mut().push(input);
        true
    });
    // 4000 × 3000 at actual pixels in a 400 × 240 view, at x 512, y 512.
    assert!(open.window.get_tiled());
    assert!(dispatch_key(&open, "viewer.last"));
    assert_eq!(open.window.get_position(), 1.0);
    assert_eq!(open.image_camera.borrow().position(1), 1.0);
    assert_eq!(pans(&inputs).last(), Some(&[512.0, 3000.0 - 240.0]));
    assert!(dispatch_key(&open, "viewer.first"));
    assert_eq!(open.window.get_position(), 0.0);
    assert_eq!(pans(&inputs).last(), Some(&[512.0, 0.0]));

    // An image only wider than the view scrolls across: Home and End go to its sides.
    let mut wide = image_presentation_tests::frame();
    wide.revision = 2;
    let info = wide.image.as_mut().unwrap();
    info.surface = ViewerSurfaceId(21);
    info.height = 200;
    info.extent = [4000.0, 200.0];
    info.origin = [512.0, -20.0];
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::Frame(Box::new(wide))],
    );
    inputs.borrow_mut().clear();
    assert!(dispatch_key(&open, "viewer.last"));
    assert_eq!(open.window.get_horizontal_position(), 1.0);
    assert_eq!(pans(&inputs).last(), Some(&[4000.0 - 400.0, -20.0]));
    assert!(dispatch_key(&open, "viewer.first"));
    assert_eq!(open.window.get_horizontal_position(), 0.0);
    assert_eq!(pans(&inputs).last(), Some(&[0.0, -20.0]));
    manager.close_all();
}

/// With the system's animation effects off, a wheel notch and a scroll key move a tiled
/// image (or document) at once instead of gliding.
#[test]
fn with_reduced_motion_a_notch_and_a_key_move_a_tiled_image_at_once() {
    use slint::winit_030::winit::{
        dpi::PhysicalPosition,
        event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent},
    };
    let (_runtime, manager, open, _service) = image_presentation_tests::open();
    open.timer.stop();
    manager.set_motion_preference(Rc::new(|| true));
    *open.input.borrow_mut() = Rc::new(|_| true);
    assert!(open.window.get_tiled());
    let scale = f64::from(open.window.window().scale_factor());
    image_motion::observe(
        &open,
        &WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(
                f64::from(open.window.get_content_left() + 20.0) * scale,
                f64::from(open.window.get_content_top() + 20.0) * scale,
            ),
        },
    );
    let before = open.image_camera.borrow().origin;
    image_motion::observe(
        &open,
        &WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, -1.0),
            phase: TouchPhase::Moved,
        },
    );
    assert!(!open.image_camera.borrow().active(), "the notch glides");
    let after = open.image_camera.borrow().origin;
    assert!(
        after[1] > before[1],
        "the notch did not move: {before:?} {after:?}"
    );
    assert!(dispatch_key(&open, "viewer.down"));
    assert!(!open.image_camera.borrow().active(), "the key glides");
    assert!(open.image_camera.borrow().origin[1] > after[1]);
    manager.close_all();
}
