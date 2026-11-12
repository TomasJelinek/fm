//! Dragging the real scrollbar must not wait for a delayed content renderer.
use slint::{
    ComponentHandle,
    platform::{
        Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::RefCell, rc::Rc};

slint::slint! {
    import { PositionBar } from "../ui/viewer-position-bar.slint";
    export component PositionFixture inherits Window {
        width: 440px; height: 440px; background: #000000;
        in-out property <float> position: 0.25;
        in-out property <float> extent: 0.1;
        in-out property <bool> horizontal;
        in-out property <string> context-key;
        callback seek(float);
        PositionBar {
            x: root.horizontal ? 20px : 16px;
            y: root.horizontal ? 16px : 20px;
            width: root.horizontal ? 400px : 14px;
            height: root.horizontal ? 14px : 400px;
            position: root.position; extent: root.extent;
            horizontal: root.horizontal; accent: #00ff00;
            context-key: root.context-key;
            seek(value) => { root.seek(value); }
        }
    }
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

struct Fixture {
    window: PositionFixture,
    native: Rc<MinimalSoftwareWindow>,
    seeks: Rc<RefCell<Vec<f32>>>,
}

impl Fixture {
    fn new() -> Self {
        let native = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
        let window = PositionFixture::new().unwrap();
        let seeks = Rc::new(RefCell::new(Vec::new()));
        let received = seeks.clone();
        window.on_seek(move |value| received.borrow_mut().push(value));
        window.show().unwrap();
        native.set_size(slint::PhysicalSize::new(440, 440));
        window.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(440.0, 440.0),
        });
        Self {
            window,
            native,
            seeks,
        }
    }

    fn thumb(&self) -> (usize, usize) {
        self.window.window().request_redraw();
        let mut pixels = vec![slint::Rgb8Pixel::default(); 440 * 440];
        self.native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 440);
        });
        let horizontal = self.window.get_horizontal();
        let offsets: Vec<_> = (0..440)
            .filter(|&at| {
                let pixel = pixels[if horizontal {
                    23 * 440 + at
                } else {
                    at * 440 + 23
                }];
                pixel.r == 0 && pixel.g == 255 && pixel.b == 0
            })
            .collect();
        (*offsets.first().unwrap(), *offsets.last().unwrap())
    }

    fn pointer(&self, at: f32, down: Option<bool>, button: PointerEventButton) {
        let position = if self.window.get_horizontal() {
            slint::LogicalPosition::new(at, 23.0)
        } else {
            slint::LogicalPosition::new(23.0, at)
        };
        let event = match down {
            Some(true) => WindowEvent::PointerPressed { position, button },
            Some(false) => WindowEvent::PointerReleased { position, button },
            None => WindowEvent::PointerMoved { position },
        };
        self.window.window().dispatch_event(event);
    }
}

#[test]
fn drag_moves_pixels_immediately_and_ignores_delayed_backend_position() {
    let fixture = Fixture::new();
    for horizontal in [false, true] {
        fixture.window.set_horizontal(horizontal);
        fixture.window.set_position(0.25);
        fixture.window.set_extent(0.1);
        let before = fixture.thumb();
        let grab = before.0 as f32 + 5.0;
        fixture.pointer(grab, None, PointerEventButton::Left);
        fixture.pointer(grab, Some(true), PointerEventButton::Left);
        assert_eq!(fixture.thumb(), before, "grabbing off-center must not jump");
        fixture.pointer(grab + 100.0, None, PointerEventButton::Left);
        let moved = fixture.thumb();
        assert_eq!(
            moved,
            (before.0 + 100, before.1 + 100),
            "thumb waits for backend"
        );
        fixture.window.set_position(0.1);
        fixture.window.set_extent(0.2);
        assert_eq!(
            fixture.thumb(),
            moved,
            "late page metrics must not pull the drag backwards"
        );
        fixture.pointer(grab + 60.0, None, PointerEventButton::Left);
        assert_eq!(fixture.thumb(), (before.0 + 60, before.1 + 60));
        assert!((fixture.seeks.borrow().last().unwrap() - (0.25 + 60.0 / 360.0)).abs() < 0.001);
        fixture.pointer(grab + 60.0, Some(false), PointerEventButton::Left);
    }
    fixture.window.hide().unwrap();
}

#[test]
fn changing_content_stops_the_old_captured_drag() {
    let fixture = Fixture::new();
    let before = fixture.thumb();
    let grab = before.0 as f32 + 5.0;
    fixture.pointer(grab, None, PointerEventButton::Left);
    fixture.pointer(grab, Some(true), PointerEventButton::Left);
    fixture.pointer(grab + 30.0, None, PointerEventButton::Left);
    fixture.window.set_context_key("next-file".into());
    fixture.window.set_position(0.0);
    fixture.thumb();
    fixture.seeks.borrow_mut().clear();
    fixture.pointer(grab + 60.0, None, PointerEventButton::Left);
    assert!(
        fixture.seeks.borrow().is_empty(),
        "old gesture navigated the new file"
    );
    assert_eq!(fixture.thumb().0, 20);
    fixture.pointer(grab + 60.0, Some(false), PointerEventButton::Left);
    fixture.window.hide().unwrap();
}

#[test]
fn grabbing_and_releasing_without_moving_does_not_reload_the_document() {
    let fixture = Fixture::new();
    let before = fixture.thumb();
    let grab = before.0 as f32 + 5.0;
    fixture.pointer(grab, None, PointerEventButton::Left);
    fixture.pointer(grab, Some(true), PointerEventButton::Left);
    fixture.pointer(grab, Some(false), PointerEventButton::Left);
    assert!(
        fixture.seeks.borrow().is_empty(),
        "an unmoved thumb is not navigation"
    );
    assert_eq!(fixture.thumb(), before);
    fixture.window.hide().unwrap();
}

#[test]
fn non_left_drag_does_not_change_view_and_left_track_click_can_reach_both_ends() {
    let fixture = Fixture::new();
    fixture.thumb();
    fixture.pointer(120.0, None, PointerEventButton::Right);
    fixture.pointer(120.0, Some(true), PointerEventButton::Right);
    fixture.pointer(300.0, None, PointerEventButton::Right);
    fixture.pointer(300.0, Some(false), PointerEventButton::Right);
    assert!(fixture.seeks.borrow().is_empty());
    for (at, expected) in [(21.0, 0.0), (419.0, 1.0)] {
        fixture.pointer(at, None, PointerEventButton::Left);
        fixture.pointer(at, Some(true), PointerEventButton::Left);
        assert_eq!(*fixture.seeks.borrow().last().unwrap(), expected);
        fixture.pointer(at, Some(false), PointerEventButton::Left);
    }
    fixture.window.hide().unwrap();
}
