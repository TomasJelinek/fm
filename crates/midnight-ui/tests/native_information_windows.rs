//! Modeless information tools stay native; blocking notices remain in the main client area.
#[path = "support/modeless.rs"]
mod modeless;

use i_slint_core::{items::AccessibleRole, window::WindowInner};
use midnight_ui::{HelpWindow, MainWindow, Presenter};
use modeless::Windows;
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, rc::Rc};

struct Harness {
    main: MainWindow,
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
}

/// The shared software windows, plus a record of every URL Slint asks to open.
struct UrlRecorder {
    windows: Windows,
    opened: Rc<RefCell<Vec<String>>>,
}
impl slint::platform::Platform for UrlRecorder {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        slint::platform::Platform::create_window_adapter(&self.windows)
    }
    fn open_url(&self, url: &str) -> Result<(), slint::PlatformError> {
        self.opened.borrow_mut().push(url.to_owned());
        Ok(())
    }
}

impl Harness {
    fn new() -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        Self::with_platform(Box::new(Windows(windows.clone())), windows)
    }

    fn recording_urls() -> (Self, Rc<RefCell<Vec<String>>>) {
        let windows = Rc::new(RefCell::new(Vec::new()));
        let opened = Rc::new(RefCell::new(Vec::new()));
        let platform = UrlRecorder {
            windows: Windows(windows.clone()),
            opened: opened.clone(),
        };
        (Self::with_platform(Box::new(platform), windows), opened)
    }

    fn with_platform(
        platform: Box<dyn slint::platform::Platform>,
        windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    ) -> Self {
        slint::platform::set_platform(platform).unwrap();
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

    fn notice(&self) -> Rc<MinimalSoftwareWindow> {
        slint::platform::update_timers_and_animations();
        let main = self.windows.borrow()[0].clone();
        modeless::draw(&main, 1000, 700, "embedded-notice-main");
        assert_eq!(
            self.windows.borrow().len(),
            1,
            "notices create no OS window"
        );
        assert_eq!(
            WindowInner::from_pub(self.main.window())
                .active_popups()
                .len(),
            1
        );
        assert!(self.main.get_embedded_modal_open());
        main
    }

    fn escape_notice(&self) {
        let main = self.windows.borrow()[0].clone();
        main.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Escape.into(),
        });
        main.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Escape.into(),
        });
        slint::platform::update_timers_and_animations();
        assert!(self.main.window().is_visible());
        assert!(!self.main.get_embedded_modal_open());
        assert!(
            WindowInner::from_pub(self.main.window())
                .active_popups()
                .is_empty()
        );
    }
}

fn notice_content_height(window: &MinimalSoftwareWindow, title: &str) -> f32 {
    let title = modeless::accessible_popup_item(window, AccessibleRole::Text, title).unwrap();
    let close = modeless::accessible_popup_item(window, AccessibleRole::Button, "Close").unwrap();
    close.map_to_native_window(close.geometry().origin).y + close.geometry().height()
        - title.map_to_native_window(title.geometry().origin).y
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

slint::slint! {
    import { AboutSlint } from "std-widgets.slint";
    // Slint's attribution without the About window's cover, in a column of the
    // same width: the reference for the covered one.
    export component BareAttribution inherits Window {
        in property <length> column;
        VerticalLayout {
            alignment: start;
            AboutSlint { width: root.column; }
        }
    }
}

/// An item's rectangle in its native window: x, y, width, height.
fn bounds(item: &i_slint_core::item_tree::ItemRc) -> [f32; 4] {
    let rect = item.geometry();
    let at = item.map_to_native_window(rect.origin);
    [at.x, at.y, rect.width(), rect.height()]
}

fn assert_near(actual: f32, expected: f32, what: &str) {
    assert!(
        (actual - expected).abs() < 0.5,
        "{what}: {actual} instead of {expected}"
    );
}

#[test]
fn about_window_keeps_slint_attribution_visible_without_opening_its_link() {
    let (h, opened) = Harness::recording_urls();
    h.main.invoke_show_about();
    let child = h.child();
    let reference = BareAttribution::new().unwrap();
    reference.show().unwrap();
    let bare = h.windows.borrow().last().unwrap().clone();
    assert_eq!(
        h.windows.borrow().len(),
        3,
        "the reference has its own window"
    );
    for (width, height) in [(900, 650), (320, 200)] {
        modeless::draw(&child, width, height, "native-about-attribution");
        let find = |role, label| modeless::accessible_item(&child, role, label).unwrap();
        let tagline = bounds(&find(
            AccessibleRole::Text,
            "A focused, two-pane file manager.",
        ));
        let badge = bounds(&find(AccessibleRole::Image, "#MadeWithSlint"));
        let version = bounds(&find(
            AccessibleRole::Text,
            "Version 1.18.1\nhttps://slint.dev/",
        ));
        let size = format!("{width}x{height}");
        // The attribution keeps the bare widget's place, whatever the platform's
        // fonts: the whole column below the tagline, inside AboutSlint's 12px
        // padding, with its 8px spacing before the version text.
        assert_near(badge[0], tagline[0] + 12.0, &format!("{size} badge x"));
        assert_near(badge[2], tagline[2] - 24.0, &format!("{size} badge width"));
        assert_near(
            badge[1],
            tagline[1] + tagline[3] + 8.0 + 12.0,
            &format!("{size} badge y"),
        );
        assert_near(version[0], badge[0], &format!("{size} version x"));
        assert_near(version[2], badge[2], &format!("{size} version width"));
        assert_near(
            version[1],
            badge[1] + badge[3] + 8.0,
            &format!("{size} version y"),
        );
        assert!(
            badge[3] >= 48.0 && version[3] > 0.0,
            "{size}: {badge:?} {version:?}"
        );
        // ... and the bare widget's badge width at that column width.
        reference.set_column(tagline[2]);
        modeless::layout(&bare, tagline[2].ceil() as u32, 1000);
        let expected = bounds(
            &modeless::accessible_item(&bare, AccessibleRole::Image, "#MadeWithSlint").unwrap(),
        );
        assert_near(badge[2], expected[2], &format!("{size} bare badge width"));
        if width != 900 {
            // At 320x200 the About text scrolls, so its rows keep their minimum
            // heights, as without the cover: the badge keeps AboutSlint's 48px
            // minimum and the version text lies below the viewport.
            continue;
        }
        // With room for the whole text, the badge has the bare widget's height too.
        assert_near(badge[3], expected[3], "900x650 bare badge height");
        assert!(find(AccessibleRole::Text, "Version 1.18.1\nhttps://slint.dev/").is_visible());
        // Slint's own link area spans the whole widget, padding included: no click
        // on it may open the URL.
        let [left, top, across, tall] = badge;
        let middle = left + across / 2.0;
        let points = [
            (middle, top + tall / 2.0),
            (left + 1.0, top + 1.0),
            (left + across - 1.0, top + tall - 1.0),
            (middle, top - 6.0),
            (middle, version[1] + version[3] / 2.0),
            (middle, version[1] + version[3] + 6.0),
        ];
        for (px, py) in points {
            let position = slint::LogicalPosition::new(px, py);
            let button = slint::platform::PointerEventButton::Left;
            for event in [
                WindowEvent::PointerMoved { position },
                WindowEvent::PointerPressed { position, button },
                WindowEvent::PointerReleased { position, button },
            ] {
                child.window().dispatch_event(event);
            }
        }
        slint::platform::update_timers_and_animations();
        assert!(
            opened.borrow().is_empty(),
            "the attribution must not open a URL: {:?}",
            opened.borrow()
        );
    }
    assert!(child.window().is_visible(), "the clicks leave About open");
}

#[test]
fn notice_generation_opens_in_main_window_and_preserves_full_copy_details() {
    let h = Harness::new();
    let copied = Rc::new(RefCell::new(0));
    let observed = copied.clone();
    h.main.on_copy_message(move || *observed.borrow_mut() += 1);
    h.main.set_message_title("Read failed".into());
    h.main
        .set_message_body("Access denied\nPath\nC:\\文档\\example.txt".into());
    h.main.set_message_error(true);
    h.main.set_message_generation(1);
    let main = h.notice();
    modeless::click(&main, "Copy details");
    assert_eq!(*copied.borrow(), 1);
    h.escape_notice();
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
    h.notice();
    assert!(
        !h.main.get_dialog_input_blocked(),
        "the OS window remains enabled for popup input"
    );
    h.main
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: "x".into() });
    h.main
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: "x".into() });
    assert_eq!(h.main.get_command_text(), "preserved");
    h.escape_notice();
    assert!(!h.main.get_dialog_input_blocked());
}

#[test]
fn notice_initial_height_tracks_wrapped_content_instead_of_a_large_empty_canvas() {
    let h = Harness::new();
    h.main.set_message_title("Information".into());
    h.main.set_message_body("The operation completed.".into());
    h.main.set_message_generation(1);
    let main = h.notice();
    let short_height = notice_content_height(&main, "Information");
    assert!((120.0..=260.0).contains(&short_height), "{short_height}");
    modeless::draw(&main, 1000, 700, "notice-content-short");
    modeless::click(&main, "Close");
    h.main.set_message_body(
        "A long diagnostic path and explanation.\n"
            .repeat(24)
            .into(),
    );
    h.main.set_message_generation(2);
    let main = h.notice();
    let long_height = notice_content_height(&main, "Information");
    assert!(long_height > short_height + 80.0, "{long_height}");
    assert!(
        long_height <= 440.0,
        "long messages scroll instead of filling the monitor"
    );
    modeless::draw(&main, 1000, 700, "notice-content-long");
    modeless::draw(&main, 320, 200, "notice-content-minimum");
    modeless::click(&main, "Close");
}

#[test]
fn native_main_close_request_dismisses_the_embedded_notice_before_exiting_the_app() {
    let h = Harness::new();
    h.main
        .on_exit(|| panic!("a modal close request must not exit the app"));
    h.main.set_message_title("Attention".into());
    h.main.set_message_body("Dismiss this notice first.".into());
    h.main.set_message_generation(1);
    let main = h.notice();
    main.window().dispatch_event(WindowEvent::CloseRequested);
    slint::platform::update_timers_and_animations();
    assert!(h.main.window().is_visible());
    assert!(!h.main.get_embedded_modal_open());
}

#[test]
fn closing_options_beneath_a_notice_preserves_notice_keyboard_focus() {
    let h = Harness::new();
    let main = h.windows.borrow()[0].clone();
    h.main.set_options_open(true);
    slint::platform::update_timers_and_animations();
    modeless::draw(&main, 1000, 700, "options-before-notice");
    assert_eq!(
        WindowInner::from_pub(h.main.window()).active_popups().len(),
        1
    );
    h.main.set_message_title("Save result".into());
    h.main
        .set_message_body("Acknowledge this result after Options closes.".into());
    h.main.set_message_generation(1);
    slint::platform::update_timers_and_animations();
    modeless::draw(&main, 1000, 700, "notice-above-options");
    assert_eq!(
        WindowInner::from_pub(h.main.window()).active_popups().len(),
        2
    );

    // A completed asynchronous Options save publishes this same property change.
    h.main.set_options_open(false);
    slint::platform::update_timers_and_animations();
    modeless::draw(&main, 1000, 700, "notice-after-options-close");
    assert_eq!(
        WindowInner::from_pub(h.main.window()).active_popups().len(),
        1
    );
    assert_eq!(h.windows.borrow().len(), 1);
    h.escape_notice();
}

#[test]
fn long_help_keeps_close_visible_at_small_size_in_both_densities() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    for density in 0..2 {
        let help = HelpWindow::new().unwrap();
        help.invoke_apply_preferences(0, density, "".into(), 11, 15);
        help.set_shortcuts("Ctrl+A — Select all files\n".repeat(100).into());
        let weak = help.as_weak();
        help.on_request_dismiss(move || weak.upgrade().unwrap().hide().unwrap());
        help.show().unwrap();
        let child = windows.borrow().last().unwrap().clone();
        modeless::draw(
            &child,
            320,
            200,
            &format!("native-help-close-density-{density}"),
        );
        let close = modeless::accessible_item(&child, AccessibleRole::Button, "Close")
            .expect("native Help Close button");
        assert!(close.is_visible());
        let rect = close.geometry();
        let origin = close.map_to_native_window(rect.origin);
        assert!(origin.x >= 0.0 && origin.y >= 0.0);
        assert!(origin.x + rect.width() <= 320.0 && origin.y + rect.height() <= 200.0);
        assert!(rect.height() >= if density == 0 { 24.0 } else { 40.0 });
        modeless::click(&child, "Close");
        assert!(
            !help.window().is_visible(),
            "Close must dismiss density {density} Help"
        );
    }
}
