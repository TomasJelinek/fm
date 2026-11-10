//! Bulk selection shortcuts follow actual file-list focus and preserve editors.
use midnight_ui::{MainWindow, Theme};
use slint::{
    ComponentHandle, SharedString,
    platform::{
        Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use crate::platform_keys;

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn key(window: &MainWindow, text: impl Into<SharedString>) {
    platform_keys::key(window, text.into());
}

fn chord(window: &MainWindow, modifiers: &[Key], text: impl Into<SharedString>) {
    for modifier in modifiers {
        window.window().dispatch_event(WindowEvent::KeyPressed {
            text: (*modifier).into(),
        });
    }
    key(window, text);
    for modifier in modifiers.iter().rev() {
        window.window().dispatch_event(WindowEvent::KeyReleased {
            text: (*modifier).into(),
        });
    }
}

fn click(window: &MainWindow, position: slint::LogicalPosition) {
    let button = PointerEventButton::Left;
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window
        .window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
}

#[test]
fn bulk_selection_keys_target_only_the_focused_file_pane() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    midnight_ui::install_file_keys(&window);
    window.set_left_path("left 文档".into());
    window.set_right_path("right 🧪".into());
    let requested = Rc::new(RefCell::new(Vec::new()));
    let captured = requested.clone();
    window.on_bulk_selection(move |pane, operation| {
        captured.borrow_mut().push((pane, operation));
    });
    let submitted = Rc::new(Cell::new(0));
    let captured = submitted.clone();
    window.on_submit_command(move |_| {
        captured.set(captured.get() + 1);
        false
    });
    let opened = Rc::new(Cell::new(0));
    let captured = opened.clone();
    window.on_open(move |_, _| captured.set(captured.get() + 1));
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    native.set_size(slint::PhysicalSize::new(1340, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1340.0, 820.0),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); 1340 * 820];
    let draw = |pixels: &mut [slint::Rgb8Pixel]| {
        native.draw_if_needed(|renderer| {
            renderer.render(pixels, 1340);
        });
    };

    for pane in [0, 1] {
        window.set_active_pane(pane);
        window.invoke_focus_files();
        assert_eq!(window.get_focused_pane(), pane);
        for (letter, operation) in [("a", 0), ("d", 1), ("i", 2), ("A", 0)] {
            let previous = requested.borrow().len();
            chord(&window, &[Key::Control], letter);
            assert_eq!(
                requested.borrow().get(previous),
                Some(&(pane, operation)),
                "Ctrl+{letter} must reach the focused file pane exactly once"
            );
            assert_eq!(requested.borrow().len(), previous + 1);
        }
        let previous = requested.borrow().len();
        for modifiers in [
            vec![],
            vec![Key::Shift],
            vec![Key::Alt],
            vec![Key::Control, Key::Shift],
            vec![Key::Control, Key::Alt],
            vec![Key::Control, Key::Meta],
        ] {
            for letter in ["a", "d", "i"] {
                chord(&window, &modifiers, letter);
            }
        }
        assert_eq!(requested.borrow().len(), previous);
    }

    let previous = requested.borrow().len();
    window.invoke_set_command("echo marker".into(), 11);
    chord(&window, &[Key::Control], "a");
    key(&window, "replacement 文档");
    assert_eq!(window.get_command_text().as_str(), "replacement 文档");
    for letter in ["d", "i"] {
        chord(&window, &[Key::Control], letter);
    }
    assert_eq!(requested.borrow().len(), previous);
    assert!(window.get_command_focused());
    assert_eq!(
        submitted.get(),
        0,
        "selection keys cannot execute a command"
    );
    assert_eq!(opened.get(), 0, "selection keys cannot activate a file");

    window.invoke_focus_files();
    chord(&window, &[Key::Control], "g");
    assert_eq!(window.get_focused_pane(), -1);
    let navigated = Rc::new(RefCell::new(Vec::new()));
    let captured = navigated.clone();
    window.on_navigate(move |pane, path| captured.borrow_mut().push((pane, path)));
    chord(&window, &[Key::Control], "a");
    key(&window, "new path 🧪");
    key(&window, Key::Return);
    assert_eq!(*navigated.borrow(), vec![(1, "new path 🧪".into())]);
    assert_eq!(requested.borrow().len(), previous);
    chord(&window, &[Key::Control], "g");
    for letter in ["d", "i"] {
        chord(&window, &[Key::Control], letter);
    }
    assert_eq!(requested.borrow().len(), previous);

    window.invoke_focus_files();
    draw(&mut pixels);
    let theme = window.global::<Theme>();
    let command_y = 820.0
        - theme.get_window_padding()
        - theme.get_bottom_height()
        - theme.get_gap()
        - theme.get_command_height() / 2.0;
    click(&window, slint::LogicalPosition::new(1240.0, command_y));
    draw(&mut pixels);
    assert_eq!(window.get_focused_pane(), -1);
    for letter in ["a", "d", "i"] {
        chord(&window, &[Key::Control], letter);
    }
    assert_eq!(
        requested.borrow().len(),
        previous,
        "Options keeps its input"
    );
    key(&window, Key::Escape);
    window.invoke_focus_files();
    key(&window, Key::F1);
    draw(&mut pixels);
    for letter in ["a", "d", "i"] {
        chord(&window, &[Key::Control], letter);
    }
    assert_eq!(
        requested.borrow().len(),
        previous,
        "popups cannot select files"
    );
    assert_eq!(submitted.get(), 0);
    assert_eq!(opened.get(), 0);
    window.hide().unwrap();
}
