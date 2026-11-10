//! Actual Ctrl+L/Ctrl+F1 request, detail display, copy and dismissed-result lifecycle.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;
use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{ComponentHandle, platform::Key};
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
    let mut ui = ui::TestUi::new();
    let root = std::env::temp_dir().join("資料_".repeat(150));
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let captured = commands.clone();
    install(
        &ui.window,
        presenter.clone(),
        Rc::new(move |command| {
            captured.borrow_mut().push(command);
            true
        }),
    );
    ui.window.invoke_focus_files();
    ui::chord(&ui.window, &[Key::Control], "l");
    ui.draw();
    let request = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::QueryVolumeInformation(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(request.path, root);
    assert!(ui.window.get_message_body().contains("Reading native"));
    assert!(ui::has_popup(&ui.window));
    ui::click_popup(&ui.window, "Close", AccessibleRole::Button);
    ui.draw();
    assert!(!ui::has_popup(&ui.window));
    presenter.borrow_mut().apply(result(&request));
    refresh(&ui.window, &presenter.borrow());
    ui.draw();
    assert!(
        !ui::has_popup(&ui.window),
        "a dismissed volume query must never reopen its popup"
    );

    ui.window.invoke_focus_files();
    ui::chord(&ui.window, &[Key::Control], Key::F1);
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
    refresh(&ui.window, &presenter.borrow());
    ui.draw();
    assert!(ui::has_popup(&ui.window));
    let body = ui.window.get_message_body();
    assert!(body.contains("MeasuredFS"));
    assert!(body.contains("資料 disk 🦀"));
    assert!(body.contains("Used: 5000000000 bytes"));
    assert!(body.contains("Storage: Unavailable"));
    assert!(!ui.window.get_message_error());
    render::snapshot(&ui, "volume-information");
    ui::click_popup(&ui.window, "Copy details", AccessibleRole::Button);
    assert!(commands.borrow().iter().any(
        |command| matches!(command, Command::CopyText { text, .. } if text.contains(body.as_str()))
    ));
    ui.resize(1040, 600);
    ui.draw();
    let close = ui::popup_item(&ui.window, "Close", AccessibleRole::Button);
    let geometry = close.geometry();
    let origin = close.map_to_native_window(geometry.origin);
    assert!(origin.y >= 0.0 && origin.y + geometry.height() <= 600.0);
    render::snapshot(&ui, "volume-information-small");
    ui::key(&ui.window, Key::Escape);
    ui.draw();
    assert!(!ui::has_popup(&ui.window));
    ui.window.hide().unwrap();
}
