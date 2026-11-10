//! Informational forms must be independent native windows, not owner-sized popups.
#[path = "support/modeless.rs"]
mod modeless;

use i_slint_core::window::WindowInner;
use midnight_ui::{MainWindow, Presenter};
use modeless::Windows;
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, rc::Rc};

fn negotiate_preferred_size(window: &MinimalSoftwareWindow) -> slint::PhysicalSize {
    // The software adapter has no compositor to choose an initial size. Use
    // the component's published layout request, then render at that exact size.
    let inner = WindowInner::from_pub(window.window());
    let component = inner.component();
    let component = i_slint_core::item_tree::ItemTreeRc::borrow_pin(&component);
    let horizontal = component
        .as_ref()
        .layout_info(i_slint_core::layout::Orientation::Horizontal);
    let vertical = component
        .as_ref()
        .layout_info(i_slint_core::layout::Orientation::Vertical);
    slint::LogicalSize::new(horizontal.preferred_bounded(), vertical.preferred_bounded())
        .to_physical(window.window().scale_factor())
}

struct Harness {
    main: MainWindow,
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
}

impl Harness {
    fn new() -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
        let main = MainWindow::new().unwrap();
        let path = std::env::temp_dir().join("native-dialogs-文档");
        let state = Rc::new(RefCell::new(Presenter::new(path.clone(), path)));
        midnight_ui::install(&main, state, Rc::new(|_| true));
        main.show().unwrap();
        modeless::draw(&windows.borrow()[0], 1000, 700, "information-main");
        Self { main, windows }
    }

    fn child(&self) -> Rc<MinimalSoftwareWindow> {
        slint::platform::update_timers_and_animations();
        assert_eq!(
            self.windows.borrow().len(),
            2,
            "the dialog must create its own native window adapter"
        );
        assert!(
            WindowInner::from_pub(self.main.window())
                .active_popups()
                .is_empty(),
            "an independent dialog must not retain a main-window popup"
        );
        let child = self.windows.borrow()[1].clone();
        assert!(child.window().is_visible());
        child
    }

    fn escape(&self, child: &MinimalSoftwareWindow) {
        child.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Escape.into(),
        });
        child.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Escape.into(),
        });
        assert!(!child.window().is_visible(), "Escape must close the child");
        assert!(self.main.window().is_visible(), "main must stay visible");
    }
}

#[test]
fn help_action_opens_independent_resizable_window_and_escape_closes_it() {
    let h = Harness::new();
    h.main.invoke_show_app_menu(0);
    let child = h.child();
    modeless::draw(&child, 320, 200, "native-help-small");
    modeless::draw(&child, 900, 650, "native-help-large");
    h.escape(&child);
}

#[test]
fn about_action_opens_independent_window_with_a_working_close_button() {
    let h = Harness::new();
    h.main.invoke_show_about();
    let child = h.child();
    modeless::draw(&child, 320, 200, "native-about-small");
    modeless::click(&child, "Close");
    assert!(!child.window().is_visible());
    assert!(h.main.window().is_visible());
}

#[test]
fn notice_generation_opens_independent_window_and_preserves_full_copy_details() {
    let h = Harness::new();
    let copied = Rc::new(RefCell::new(0));
    let observed = copied.clone();
    h.main.on_copy_message(move || *observed.borrow_mut() += 1);
    h.main.set_message_title("Read failed".into());
    h.main
        .set_message_body("Access denied\nPath\nC:\\文档\\example.txt".into());
    h.main.set_message_error(true);
    h.main.set_message_generation(1);
    let child = h.child();
    modeless::draw(&child, 600, 400, "native-notice");
    modeless::click(&child, "Copy details");
    assert_eq!(*copied.borrow(), 1);
    h.escape(&child);
}

#[test]
fn modal_notice_blocks_keyboard_input_to_the_main_command_editor() {
    let h = Harness::new();
    h.main.set_command_text("preserved".into());
    h.main.invoke_focus_command();
    h.main.set_message_title("Attention".into());
    h.main
        .set_message_body("Acknowledge before returning to the editor".into());
    h.main.set_message_generation(1);
    let child = h.child();
    assert!(h.main.get_dialog_input_blocked());
    h.main
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: "x".into() });
    h.main
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: "x".into() });
    assert_eq!(h.main.get_command_text(), "preserved");
    h.escape(&child);
    assert!(!h.main.get_dialog_input_blocked());
}

#[test]
fn notice_initial_height_tracks_wrapped_content_instead_of_a_large_empty_canvas() {
    let h = Harness::new();
    h.main.set_message_title("Information".into());
    h.main.set_message_body("The operation completed.".into());
    h.main.set_message_generation(1);
    let short = h.child();
    let short_size = negotiate_preferred_size(&short);
    assert!((200..=260).contains(&short_size.height), "{short_size:?}");
    modeless::draw(
        &short,
        short_size.width,
        short_size.height,
        "notice-content-short",
    );
    modeless::click(&short, "Close");
    h.main.set_message_body(
        "A long diagnostic path and explanation.\n"
            .repeat(24)
            .into(),
    );
    h.main.set_message_generation(2);
    slint::platform::update_timers_and_animations();
    let long = h.windows.borrow()[2].clone();
    let long_size = negotiate_preferred_size(&long);
    assert!(long_size.height > short_size.height + 80, "{long_size:?}");
    assert!(
        long_size.height <= 440,
        "long messages scroll instead of filling the monitor"
    );
    modeless::draw(
        &long,
        long_size.width,
        long_size.height,
        "notice-content-long",
    );
    modeless::click(&long, "Close");
}
