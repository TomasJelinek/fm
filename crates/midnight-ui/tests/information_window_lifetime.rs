//! Modeless information tools and embedded notices retain their own lifetimes.
#[allow(dead_code)]
#[path = "support/modeless.rs"]
mod modeless;

use i_slint_core::{items::AccessibleRole, window::WindowInner};
use midnight_ui::{MainWindow, NoticePresentation, Presenter};
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

fn assert_notice_open(main: &MainWindow) {
    slint::platform::update_timers_and_animations();
    assert!(!main.global::<NoticePresentation>().get_token().is_empty());
    assert!(main.get_embedded_modal_open());
    assert!(!main.get_dialog_input_blocked());
    assert_eq!(
        WindowInner::from_pub(main.window()).active_popups().len(),
        1
    );
}

fn assert_notice_closed(main: &MainWindow) {
    slint::platform::update_timers_and_animations();
    assert!(main.global::<NoticePresentation>().get_token().is_empty());
    assert!(!main.get_embedded_modal_open());
    assert!(!main.get_dialog_input_blocked());
    assert!(
        WindowInner::from_pub(main.window())
            .active_popups()
            .is_empty()
    );
    assert!(main.window().is_visible());
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
    assert_notice_open(&main);
    assert_eq!(windows.borrow().len(), 1);
    let notice = windows.borrow()[0].clone();
    modeless::draw(&notice, 640, 420, "startup-notice-latest");
    let texts = modeless::accessible_popup_item(&notice, AccessibleRole::Text, "Generation 2");
    assert!(
        texts.is_some(),
        "the latest pending generation must be displayed"
    );
    assert_eq!(main.global::<NoticePresentation>().get_generation(), 2);
    notice.window().dispatch_event(WindowEvent::CloseRequested);
    assert_notice_closed(&main);
    main.invoke_refresh_information_windows();
    assert_notice_closed(&main);
    assert_eq!(windows.borrow().len(), 1);
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
fn embedded_notice_blocks_background_input_and_shutdown_closes_information_tools() {
    let (main, windows) = harness();
    main.invoke_show_app_menu(0);
    main.invoke_show_about();
    assert!(!main.get_dialog_input_blocked());
    main.invoke_set_command("preserved command".into(), 17);
    main.set_message_title("Diagnostic".into());
    main.set_message_body("Complete diagnostic text".into());
    main.set_message_generation(1);
    assert_notice_open(&main);
    assert_eq!(windows.borrow().len(), 3);
    main.window()
        .dispatch_event(WindowEvent::KeyPressed { text: "x".into() });
    main.window()
        .dispatch_event(WindowEvent::KeyReleased { text: "x".into() });
    assert_eq!(main.get_command_text(), "preserved command");
    assert!(windows.borrow()[1].window().is_visible());
    assert!(windows.borrow()[2].window().is_visible());
    main.invoke_close_information_windows();
    assert_notice_closed(&main);
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
        3,
        "shutdown must not reopen children"
    );
    assert_notice_closed(&main);
}

#[test]
fn startup_and_replacement_notices_never_request_another_adapter() {
    struct MainAdapterOnly {
        attempts: Rc<Cell<usize>>,
        windows: NativeWindows,
    }
    impl slint::platform::Platform for MainAdapterOnly {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            self.attempts.set(self.attempts.get() + 1);
            if self.attempts.get() != 1 {
                return Err("embedded notices must reuse the main adapter".into());
            }
            let window = MinimalSoftwareWindow::new(
                slint::platform::software_renderer::RepaintBufferType::NewBuffer,
            );
            self.windows.borrow_mut().push(window.clone());
            Ok(window)
        }
    }
    let windows = Rc::new(RefCell::new(Vec::new()));
    let attempts = Rc::new(Cell::new(0));
    slint::platform::set_platform(Box::new(MainAdapterOnly {
        attempts: attempts.clone(),
        windows: windows.clone(),
    }))
    .unwrap();
    let main = MainWindow::new().unwrap();
    let path = std::path::PathBuf::from("embedded-notice-fixture");
    midnight_ui::install(
        &main,
        Rc::new(RefCell::new(Presenter::new(path.clone(), path))),
        Rc::new(|_| true),
    );
    main.set_message_title("Startup diagnostic".into());
    main.set_message_body("First diagnostic".into());
    main.set_message_generation(1);
    slint::platform::update_timers_and_animations();
    main.show().unwrap();
    // The software adapter has no preferred-size negotiation. Establish the
    // client viewport before testing native-close delivery to visible controls.
    let host = windows.borrow()[0].clone();
    modeless::draw(&host, 640, 420, "replacement-notice-main");
    main.invoke_refresh_information_windows();
    assert_notice_open(&main);
    assert_eq!(attempts.get(), 1);
    let first = main.global::<NoticePresentation>().get_token();
    for _ in 0..3 {
        main.invoke_refresh_information_windows();
    }
    assert_eq!(
        attempts.get(),
        1,
        "refresh must not allocate a native notice"
    );
    assert_eq!(main.global::<NoticePresentation>().get_token(), first);
    main.set_message_body("Replacement diagnostic".into());
    main.set_message_generation(2);
    assert_notice_open(&main);
    assert_eq!(
        attempts.get(),
        1,
        "replacement also reuses the main adapter"
    );
    let presentation = main.global::<NoticePresentation>();
    let replacement = presentation.get_token();
    assert_ne!(first, replacement);
    presentation.invoke_dismiss(first);
    assert_eq!(presentation.get_token(), replacement);
    assert_notice_open(&main);
    assert!(
        modeless::accessible_popup_item(&host, AccessibleRole::Button, "Close")
            .is_some_and(|item| item.is_visible()),
        "native close must target the replacement's visible controls"
    );
    windows.borrow()[0]
        .window()
        .dispatch_event(WindowEvent::CloseRequested);
    assert_notice_closed(&main);
    main.invoke_refresh_information_windows();
    assert_notice_closed(&main);
    assert_eq!(attempts.get(), 1);
}
