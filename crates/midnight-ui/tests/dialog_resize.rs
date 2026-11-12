//! Pointer resize checks for the actual shared embedded dialog frame.
use slint::{
    ComponentHandle, LogicalPosition,
    platform::{
        Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::rc::Rc;

slint::slint! {
    import { DialogFrame } from "../ui/dialog-frame.slint";
    import { LineEdit } from "std-widgets.slint";
    export component ResizeFixture inherits Window {
        preferred-width: 1000px; preferred-height: 800px;
        min-width: 0px; min-height: 0px;
        out property <length> dialog-left: frame.body-left;
        out property <length> dialog-top: frame.body-top;
        out property <length> dialog-width: frame.body-width;
        out property <length> dialog-height: frame.body-height;
        out property <string> draft: editor.text;
        public function reset() { frame.reset(); }
        public function focus-editor() { editor.focus(); }
        frame := DialogFrame {
            width: parent.width; height: parent.height;
            desired-width: 600px; desired-height: 400px;
            Rectangle {
                background: #333;
                VerticalLayout {
                    padding: 12px;
                    editor := LineEdit { text: "draft 文档"; }
                    Rectangle { vertical-stretch: 1; }
                }
            }
        }
    }
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn draw(native: &MinimalSoftwareWindow, width: usize, height: usize) {
    slint::platform::update_timers_and_animations();
    native.draw_if_needed(|renderer| {
        renderer.render(
            &mut vec![slint::Rgb8Pixel::default(); width * height],
            width,
        );
    });
}

fn drag(window: &ResizeFixture, start: LogicalPosition, dx: f32, dy: f32) {
    let end = LogicalPosition::new(start.x + dx, start.y + dy);
    for event in [
        WindowEvent::PointerMoved { position: start },
        WindowEvent::PointerPressed {
            position: start,
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerMoved { position: end },
        WindowEvent::PointerReleased {
            position: end,
            button: PointerEventButton::Left,
        },
    ] {
        window.window().dispatch_event(event);
        slint::platform::update_timers_and_animations();
    }
}

#[test]
fn edges_and_corners_resize_the_open_frame_without_replacing_the_draft() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = ResizeFixture::new().unwrap();
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    native.set_size(slint::PhysicalSize::new(1000, 800));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1000.0, 800.0),
    });
    draw(&native, 1000, 800);
    for (horizontal, vertical) in [
        (1_i32, 0_i32),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (-1, -1),
        (1, -1),
        (-1, 1),
    ] {
        window.invoke_reset();
        draw(&native, 1000, 800);
        let left = window.get_dialog_left();
        let top = window.get_dialog_top();
        let width = window.get_dialog_width();
        let height = window.get_dialog_height();
        window.invoke_focus_editor();
        let draft = window.get_draft();
        let start = LogicalPosition::new(
            left + if horizontal < 0 {
                2.0
            } else if horizontal > 0 {
                width - 2.0
            } else {
                width / 2.0
            },
            top + if vertical < 0 {
                2.0
            } else if vertical > 0 {
                height - 2.0
            } else {
                height / 2.0
            },
        );
        drag(
            &window,
            start,
            horizontal as f32 * 60.0,
            vertical as f32 * 40.0,
        );
        draw(&native, 1000, 800);
        assert!(
            (window.get_dialog_width() - width - horizontal.unsigned_abs() as f32 * 60.0).abs()
                < 1.0,
            "horizontal edge {horizontal}/{vertical} did not resize"
        );
        assert!(
            (window.get_dialog_height() - height - vertical.unsigned_abs() as f32 * 40.0).abs()
                < 1.0,
            "vertical edge {horizontal}/{vertical} did not resize"
        );
        if horizontal < 0 {
            assert!(
                (window.get_dialog_left() + window.get_dialog_width() - left - width).abs() < 1.0
            );
        } else {
            assert!((window.get_dialog_left() - left).abs() < 1.0);
        }
        if vertical < 0 {
            assert!(
                (window.get_dialog_top() + window.get_dialog_height() - top - height).abs() < 1.0
            );
        } else {
            assert!((window.get_dialog_top() - top).abs() < 1.0);
        }
        assert_eq!(window.get_draft(), draft);
        window
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: "!".into() });
        window
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: "!".into() });
        assert_eq!(
            window.get_draft().len(),
            draft.len() + 1,
            "resize stole text input focus"
        );
    }
    native.set_size(slint::PhysicalSize::new(320, 200));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(320.0, 200.0),
    });
    draw(&native, 320, 200);
    let corner = LogicalPosition::new(
        window.get_dialog_left() + window.get_dialog_width() - 2.0,
        window.get_dialog_top() + window.get_dialog_height() - 2.0,
    );
    drag(&window, corner, 1000.0, 1000.0);
    draw(&native, 320, 200);
    assert!(window.get_dialog_left() >= 0.0 && window.get_dialog_top() >= 0.0);
    assert!(window.get_dialog_left() + window.get_dialog_width() <= 320.5);
    assert!(window.get_dialog_top() + window.get_dialog_height() <= 200.5);
}
