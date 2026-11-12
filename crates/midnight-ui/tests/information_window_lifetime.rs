//! Installed informational actions share native lifetime and modal input policy.
#[allow(dead_code)]
#[path = "support/modeless.rs"]
mod modeless;

use midnight_ui::{MainWindow, Presenter};
use slint::{
    ComponentHandle,
    platform::{WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

type NativeWindows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;

fn harness() -> (MainWindow, NativeWindows) {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let main = MainWindow::new().unwrap();
    let path = std::path::PathBuf::from("native-dialog-fixture");
    let presenter = Rc::new(RefCell::new(Presenter::new(path.clone(), path)));
    midnight_ui::install(&main, presenter, Rc::new(|_| true));
    main.show().unwrap();
    (main, windows)
}

#[test]
fn latest_startup_notice_waits_for_visible_owner_and_does_not_reopen_after_close() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let main = MainWindow::new().unwrap();
    let path = std::path::PathBuf::from("startup-notice-fixture");
    midnight_ui::install(
        &main,
        Rc::new(RefCell::new(Presenter::new(path.clone(), path))),
        Rc::new(|_| true),
    );
    for generation in [1, 2] {
        main.set_message_title("Startup diagnostic".into());
        main.set_message_body(format!("Generation {generation}").into());
        main.set_message_generation(generation);
    }
    assert_eq!(windows.borrow().len(), 1);
    assert!(!main.get_dialog_input_blocked());
    main.show().unwrap();
    main.invoke_refresh_information_windows();
    assert_eq!(windows.borrow().len(), 2);
    let notice = windows.borrow()[1].clone();
    modeless::draw(&notice, 640, 420, "startup-notice-latest");
    let texts = modeless::accessible_item(
        &notice,
        i_slint_core::items::AccessibleRole::Text,
        "Generation 2",
    );
    assert!(
        texts.is_some(),
        "the latest pending generation must be displayed"
    );
    notice.window().dispatch_event(WindowEvent::CloseRequested);
    main.invoke_refresh_information_windows();
    assert_eq!(windows.borrow().len(), 2);
    assert!(!notice.window().is_visible());
    assert!(!main.get_dialog_input_blocked());
}

#[test]
fn repeated_help_open_reuses_one_visible_window_and_native_x_releases_it() {
    let (main, windows) = harness();
    main.invoke_show_app_menu(0);
    main.invoke_show_app_menu(0);
    assert_eq!(windows.borrow().len(), 2);
    assert!(!main.get_dialog_input_blocked());
    let first = windows.borrow()[1].clone();
    first.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(!first.window().is_visible());
    assert!(main.window().is_visible());
    main.invoke_show_app_menu(0);
    assert_eq!(windows.borrow().len(), 3);
    let second = windows.borrow()[2].clone();
    assert!(second.window().is_visible());
    second.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(!second.window().is_visible());
}

#[test]
fn native_about_shows_the_application_version_and_keeps_main_usable() {
    let (main, windows) = harness();
    main.set_application_version("9.8.7".into());
    main.invoke_show_about();
    let about = windows.borrow()[1].clone();
    modeless::draw(&about, 320, 200, "native-about-version");
    assert!(
        modeless::accessible_item(
            &about,
            i_slint_core::items::AccessibleRole::Text,
            "Version 9.8.7"
        )
        .is_some()
    );
    assert!(!main.get_dialog_input_blocked());
    modeless::click(&about, "Close");
    assert!(!about.window().is_visible());
}

#[test]
fn only_notice_blocks_owner_and_shutdown_closes_all_information_windows() {
    let (main, windows) = harness();
    main.invoke_show_app_menu(0);
    main.invoke_show_about();
    assert!(!main.get_dialog_input_blocked());
    main.set_message_title("Diagnostic".into());
    main.set_message_body("Complete diagnostic text".into());
    main.set_message_generation(1);
    slint::platform::update_timers_and_animations();
    assert!(main.get_dialog_input_blocked());
    assert_eq!(windows.borrow().len(), 4);
    let notice = windows.borrow()[3].clone();
    notice.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(!main.get_dialog_input_blocked());
    assert!(windows.borrow()[1].window().is_visible());
    assert!(windows.borrow()[2].window().is_visible());
    main.invoke_close_information_windows();
    assert!(
        windows
            .borrow()
            .iter()
            .skip(1)
            .all(|window| !window.window().is_visible())
    );
    main.invoke_show_app_menu(0);
    main.invoke_show_about();
    main.set_message_generation(2);
    slint::platform::update_timers_and_animations();
    assert_eq!(
        windows.borrow().len(),
        4,
        "shutdown must not reopen children"
    );
    assert!(!main.get_dialog_input_blocked());
}

#[test]
fn failed_startup_notice_does_not_retry_on_refresh_or_leave_owner_blocked() {
    struct FailingBoundary {
        fail: Rc<Cell<bool>>,
        attempts: Rc<Cell<usize>>,
        windows: NativeWindows,
    }
    impl slint::platform::Platform for FailingBoundary {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            self.attempts.set(self.attempts.get() + 1);
            if self.fail.get() {
                return Err("injected information window creation failure".into());
            }
            let window = MinimalSoftwareWindow::new(
                slint::platform::software_renderer::RepaintBufferType::NewBuffer,
            );
            self.windows.borrow_mut().push(window.clone());
            Ok(window)
        }
    }
    let windows = Rc::new(RefCell::new(Vec::new()));
    let fail = Rc::new(Cell::new(false));
    let attempts = Rc::new(Cell::new(0));
    slint::platform::set_platform(Box::new(FailingBoundary {
        fail: fail.clone(),
        attempts: attempts.clone(),
        windows: windows.clone(),
    }))
    .unwrap();
    let main = MainWindow::new().unwrap();
    let path = std::path::PathBuf::from("failed-notice-fixture");
    midnight_ui::install(
        &main,
        Rc::new(RefCell::new(Presenter::new(path.clone(), path))),
        Rc::new(|_| true),
    );
    main.set_message_title("Startup diagnostic".into());
    main.set_message_generation(1);
    slint::platform::update_timers_and_animations();
    fail.set(true);
    main.show().unwrap();
    main.invoke_refresh_information_windows();
    assert_eq!(attempts.get(), 2);
    assert!(!main.get_dialog_input_blocked());
    assert!(
        main.get_desktop_status()
            .contains("injected information window creation failure")
    );
    for _ in 0..3 {
        main.invoke_refresh_information_windows();
    }
    assert_eq!(
        attempts.get(),
        2,
        "a failed generation must not retry on refresh"
    );
    fail.set(false);
    main.set_message_generation(2);
    slint::platform::update_timers_and_animations();
    assert_eq!(
        attempts.get(),
        3,
        "a new notice generation can open normally"
    );
    assert!(main.get_dialog_input_blocked());
    windows.borrow()[1]
        .window()
        .dispatch_event(WindowEvent::CloseRequested);
    assert!(!main.get_dialog_input_blocked());
}
