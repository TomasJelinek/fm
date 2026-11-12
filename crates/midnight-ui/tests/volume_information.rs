//! Actual Ctrl+L/Ctrl+F1 request, detail display, copy and dismissed-result lifecycle.
#[path = "support/modeless.rs"]
mod modeless;
#[path = "support/ui.rs"]
mod ui;
use i_slint_core::{item_tree::ItemRc, items::AccessibleRole, window::WindowInner};
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, install, refresh};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

fn result(request: &VolumeInformationRequest) -> Event {
    Event::VolumeInformationLoaded(Box::new(VolumeInformationResult {
        request: request.request,
        pane: request.pane,
        navigation: request.navigation,
        path: request.path.clone(),
        result: Ok(VolumeInformation {
            root: Some(request.path.clone()),
            label: Some("資料 disk 🦀".into()),
            filesystem: Some("MeasuredFS".into()),
            source: Some("Native device".into()),
            capacity: VolumeCapacity {
                total: Some(8_000_000_000),
                free: Some(3_000_000_000),
                available: Some(1_000_000_000),
            },
            storage: StorageKind::Unknown,
            read_only: Some(false),
            properties: vec![VolumeProperty::SparseFiles, VolumeProperty::HardLinks],
            unavailable: vec![],
        }),
    }))
}

#[test]
fn drive_details_are_measured_and_late_result_does_not_reopen_a_closed_notice() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    window.set_preferences_ready(true);
    let owner = windows.borrow()[0].clone();
    let root = std::env::temp_dir().join("資料_".repeat(150));
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let captured = commands.clone();
    install(
        &window,
        presenter.clone(),
        Rc::new(move |command| {
            captured.borrow_mut().push(command);
            true
        }),
    );
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    modeless::draw(&owner, 1340, 820, "volume-information-owner");
    window.invoke_focus_files();
    ui::chord(&window, &[Key::Control], "l");
    slint::platform::update_timers_and_animations();
    let request = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::QueryVolumeInformation(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(request.path, root);
    assert!(window.get_message_body().contains("Reading native"));
    assert_eq!(
        windows.borrow().len(),
        2,
        "notice needs its own native adapter"
    );
    let loading = windows.borrow()[1].clone();
    assert!(!Rc::ptr_eq(&owner, &loading));
    assert!(loading.window().is_visible());
    assert!(window.get_dialog_input_blocked());
    assert!(!ui::has_popup(&window));
    modeless::draw(&loading, 600, 400, "volume-information-loading");
    modeless::click(&loading, "Close");
    slint::platform::update_timers_and_animations();
    assert!(!loading.window().is_visible());
    assert!(!window.get_dialog_input_blocked());
    assert!(window.window().is_visible());
    presenter.borrow_mut().apply(result(&request));
    refresh(&window, &presenter.borrow());
    slint::platform::update_timers_and_animations();
    assert!(
        !loading.window().is_visible(),
        "late reply must not reopen the child"
    );
    assert_eq!(
        windows.borrow().len(),
        2,
        "late reply must not allocate another notice"
    );
    assert!(!window.get_dialog_input_blocked());

    window.invoke_focus_files();
    ui::chord(&window, &[Key::Control], Key::F1);
    slint::platform::update_timers_and_animations();
    assert_eq!(
        windows.borrow().len(),
        3,
        "a fresh query opens a fresh native notice"
    );
    let notice = windows.borrow()[2].clone();
    assert!(notice.window().is_visible());
    assert!(window.get_dialog_input_blocked());
    assert!(!ui::has_popup(&window));
    let request = commands
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::QueryVolumeInformation(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    presenter.borrow_mut().apply(result(&request));
    refresh(&window, &presenter.borrow());
    slint::platform::update_timers_and_animations();
    assert_eq!(
        windows.borrow().len(),
        3,
        "reply updates the existing native notice"
    );
    assert!(notice.window().is_visible());
    let body = window.get_message_body();
    assert!(body.contains("MeasuredFS"));
    assert!(body.contains("資料 disk 🦀"));
    assert!(body.contains("Used: 5000000000 bytes"));
    assert!(body.contains("Storage: Unavailable"));
    assert!(!window.get_message_error());
    modeless::draw(&notice, 600, 400, "volume-information");
    let child_root = ItemRc::new_root(WindowInner::from_pub(notice.window()).component());
    assert!(
        ui::visible_text_values(child_root)
            .iter()
            .any(|text| text == &body)
    );
    modeless::click(&notice, "Copy details");
    assert!(commands.borrow().iter().any(
        |command| matches!(command, Command::CopyText { text, .. } if text.contains(body.as_str()))
    ));
    let child_size = notice.window().size();
    modeless::draw(&owner, 1040, 600, "volume-information-owner-small");
    assert_eq!(
        notice.window().size(),
        child_size,
        "owner resize must not size the child"
    );
    modeless::draw(&notice, 320, 200, "volume-information-small");
    let close = modeless::accessible_item(&notice, AccessibleRole::Button, "Close").unwrap();
    let geometry = close.geometry();
    let origin = close.map_to_native_window(geometry.origin);
    assert!(close.is_visible());
    assert!(origin.x >= 0.0 && origin.x + geometry.width() <= 320.0);
    assert!(origin.y >= 0.0 && origin.y + geometry.height() <= 200.0);
    for event in [
        WindowEvent::KeyPressed {
            text: Key::Escape.into(),
        },
        WindowEvent::KeyReleased {
            text: Key::Escape.into(),
        },
    ] {
        notice.window().dispatch_event(event);
    }
    slint::platform::update_timers_and_animations();
    assert!(!notice.window().is_visible());
    assert!(!window.get_dialog_input_blocked());
    assert!(window.window().is_visible());
    assert!(!ui::has_popup(&window));
    window.hide().unwrap();
}
