//! Real message-window layout, clipboard dispatch and focus behavior.
#[allow(dead_code)]
#[path = "support/modeless.rs"]
mod modeless;
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{item_tree::ItemRc, items::AccessibleRole, window::WindowInner};
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, install, refresh};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, io, path::PathBuf, rc::Rc, sync::Arc};

type NativeWindows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;

fn native_main() -> (MainWindow, NativeWindows) {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    window.show().unwrap();
    (window, windows)
}

fn escape(window: &MinimalSoftwareWindow) {
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
}

#[test]
fn panel_error_copy_preserves_the_complete_diagnostic_and_uses_the_backend() {
    let mut ui = ui::TestUi::new();
    let root = PathBuf::from(std::path::MAIN_SEPARATOR_STR).join("errors");
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let capture = commands.clone();
    install(
        &ui.window,
        presenter.clone(),
        Rc::new(move |command| {
            capture.borrow_mut().push(command);
            true
        }),
    );
    let error = format!(
        "Archive unavailable\nPath\nD:\\資料\\{}\\example.zip",
        "long-".repeat(100)
    );
    let navigation = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::Navigate {
                pane: PaneId::Left,
                navigation,
                ..
            } => Some(*navigation),
            _ => None,
        })
        .unwrap();
    presenter.borrow_mut().apply(Event::ListingFailed {
        pane: PaneId::Left,
        navigation,
        error: FsError::from_io(
            "archive navigation",
            std::path::Path::new(""),
            io::Error::other(error.clone()),
        ),
    });
    refresh(&ui.window, &presenter.borrow());
    let displayed = ui.window.get_left_error().to_string();
    assert!(displayed.contains(&error));
    for (width, height) in [(1340, 820), (320, 200)] {
        ui.resize(width, height);
        ui.draw();
        commands.borrow_mut().clear();
        ui::click_root(&ui.window, "Copy error details", AccessibleRole::Button);
        assert!(commands.borrow().iter().any(|command| matches!(command,
            Command::CopyText { text, .. } if text == &displayed
        )));
        assert!(commands.borrow().iter().all(|command| !matches!(
            command,
            Command::PrepareOperation(_) | Command::LaunchTerminal { .. }
        )));
    }
}

#[test]
fn failed_action_is_readable_copyable_and_escape_closes_without_losing_panel() {
    let (window, windows) = native_main();
    let root = PathBuf::from(std::path::MAIN_SEPARATOR_STR).join("errors");
    let path = root
        .join("資料")
        .join("long-name-".repeat(40))
        .join("žluťoučký 🦀.txt");
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let sink = commands.clone();
    install(
        &window,
        presenter.clone(),
        Rc::new(move |command| {
            sink.borrow_mut().push(command);
            true
        }),
    );
    let navigation = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::Navigate {
                pane: PaneId::Left,
                navigation,
                ..
            } => Some(*navigation),
            _ => None,
        })
        .unwrap();
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
    presenter.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    refresh(&window, &presenter.borrow());
    window.invoke_open(0, 1);
    let request = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::OpenEntry { request, .. } => Some(*request),
            _ => None,
        })
        .unwrap();
    presenter.borrow_mut().apply(Event::DesktopActionFinished {
        request,
        result: Err(FsError::from_io(
            "open file",
            &path,
            io::Error::other("Access denied\nThe application could not open this file."),
        )),
    });
    refresh(&window, &presenter.borrow());
    slint::platform::update_timers_and_animations();
    assert_eq!(windows.borrow().len(), 2);
    let notice = windows.borrow()[1].clone();
    let body = window.get_message_body().to_string();
    assert!(
        body.contains(&format!("\nPath\n{}\n", path.display())),
        "{body}"
    );
    assert!(window.get_message_error());
    for (scheme, density, width, height) in [(2, 0, 1340, 820), (1, 1, 1040, 650), (1, 1, 320, 200)]
    {
        window.invoke_preferences_changed(scheme, density, 0);
        slint::platform::update_timers_and_animations();
        modeless::draw(
            &notice,
            width,
            height,
            &format!("error-message-{scheme}-{density}"),
        );
        assert!(modeless::accessible_item(&notice, AccessibleRole::Text, "Error").is_some());
        let inner = WindowInner::from_pub(notice.window());
        let texts = ui::text_values(ItemRc::new_root(inner.component()));
        assert!(texts.iter().any(|text| text.as_str() == body));
        for label in ["Copy details", "Close"] {
            let item = modeless::accessible_item(&notice, AccessibleRole::Button, label).unwrap();
            let geometry = item.geometry();
            let position = item.map_to_native_window(geometry.origin);
            assert!(position.y >= 0.0 && position.y + geometry.height() <= height as f32);
            assert!(position.x >= 0.0 && position.x + geometry.width() <= width as f32);
        }
    }
    modeless::click(&notice, "Copy details");
    assert!(
        commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::CopyText { text, .. }
        if text == &format!("Desktop action failed\n\n{body}")))
    );
    assert!(notice.window().is_visible());
    assert!(!ui::has_popup(&window));
    escape(&notice);
    assert!(!notice.window().is_visible());
    assert!(window.window().is_visible());
    assert!(!window.get_dialog_input_blocked());
}

#[test]
fn information_dialog_has_no_error_glyph() {
    let (window, windows) = native_main();
    let path = PathBuf::from("notice-fixture");
    install(
        &window,
        Rc::new(RefCell::new(Presenter::new(path.clone(), path))),
        Rc::new(|_| true),
    );
    window.set_message_title("Drive information".into());
    window.set_message_body("Path\nC:\\\n\nAvailable: 100 GB".into());
    window.set_message_error(false);
    window.set_message_generation(1);
    slint::platform::update_timers_and_animations();
    let notice = windows.borrow()[1].clone();
    modeless::draw(&notice, 640, 420, "information-notice");
    let inner = WindowInner::from_pub(notice.window());
    let texts = ui::text_values(ItemRc::new_root(inner.component()));
    assert!(!texts.iter().any(|text| text.as_str() == "!"));
    assert!(modeless::accessible_item(&notice, AccessibleRole::Text, "Error").is_none());
    escape(&notice);
    assert!(!notice.window().is_visible());
    assert!(window.window().is_visible());
}

#[test]
fn panel_error_has_a_readable_multiline_body_at_the_minimum_window_size() {
    let mut ui = ui::TestUi::new();
    ui.window.set_theme_index(1);
    ui.window.invoke_apply_preferences(1, 0);
    let message = "Access denied\n\nPath\nC:\\資料\\protected\\folder\n\nOperation: list directory";
    ui.window.set_left_error(message.into());
    slint::platform::update_timers_and_animations();
    ui.resize(1040, 480);
    ui::root_item(&ui.window, "Show error details", AccessibleRole::Button);
    assert!(ui.window.get_left_list_height() >= 22.0);
    render::snapshot(&ui, "error-panel-minimum");
    ui.window.hide().unwrap();
}
