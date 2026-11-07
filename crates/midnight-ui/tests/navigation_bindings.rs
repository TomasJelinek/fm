//! Exercise native identities through the installed tab/bookmark UI bridge.
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, install};
use slint::{
    ComponentHandle, Model,
    platform::{
        Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::RefCell, rc::Rc};

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

#[test]
fn installed_navigation_bridge_uses_ids_and_focuses_requested_panel() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native))).unwrap();
    let window = MainWindow::new().unwrap();
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let left = std::env::temp_dir().join("left 文档");
    let right = std::env::temp_dir().join("right 👩🏽‍💻");
    let presenter = Rc::new(RefCell::new(Presenter::new(left.clone(), right.clone())));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let sent = commands.clone();
    install(
        &window,
        presenter.clone(),
        Rc::new(move |command| {
            sent.borrow_mut().push(command);
            true
        }),
    );
    midnight_ui::refresh(&window, &presenter.borrow());
    assert_eq!(window.get_left_tabs().row_count(), 1);
    assert_eq!(window.get_right_tabs().row_count(), 1);
    window.invoke_tab_new(1);
    assert_eq!(window.get_active_pane(), 1);
    assert_eq!(window.get_focused_pane(), 1);
    assert_eq!(window.get_right_tabs().row_count(), 2);
    let first = window.get_right_tabs().row_data(0).unwrap().identity;
    window.invoke_tab_select(1, first);
    assert_eq!(window.get_right_tabs().row_count(), 2);

    commands.borrow_mut().clear();
    window.invoke_bookmark_add(0, "文档 $(literal) & Žlutý".into(), 1);
    let bookmark = window.get_bookmarks().row_data(0).unwrap();
    assert_eq!(bookmark.label, "文档 $(literal) & Žlutý");
    let saved = commands.borrow().iter().any(|command| {
        matches!(command,
        Command::SavePreferences { preferences, .. }
            if preferences.bookmarks[0].path == left)
    });
    assert!(saved, "bookmark persistence captures the raw path");
    window.invoke_bookmark_add(0, "Same path again".into(), 0);
    assert_eq!(window.get_bookmarks().row_count(), 1);
    window.invoke_bookmark_assign(bookmark.identity.clone(), 10);
    for toolbar in [window.get_left_locations(), window.get_right_locations()] {
        assert!(
            toolbar
                .iter()
                .all(|row| row.kind != midnight_ui::LocationRole::HotPath),
            "saved paths appear in menus, not as individual toolbar buttons"
        );
    }
    let hot_path = window
        .get_hot_path_choices()
        .iter()
        .find(|row| row.kind == midnight_ui::LocationRole::HotPath)
        .unwrap();
    assert_eq!(hot_path.compact_label, "0");
    assert_eq!(hot_path.label.as_str(), format!("0  {}", left.display()));
    assert!(!hot_path.tooltip.contains("$(literal)"));
    assert!(window.get_navigation_choices().iter().any(|row| {
        row.kind == midnight_ui::LocationRole::HotPath
            && row.identity == hot_path.identity
            && row.label == hot_path.label
    }));
    commands.borrow_mut().clear();
    window.invoke_bookmark_open(1, bookmark.identity.clone());
    assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::Navigate { pane: PaneId::Right, path, .. } if path == &left)));
    assert_eq!(window.get_active_pane(), 1);
    assert_eq!(window.get_focused_pane(), 1);
    assert!(!commands.borrow().iter().any(|command| matches!(
        command,
        Command::OpenEntry { .. } | Command::LaunchTerminal { .. }
    )));

    window.invoke_bookmark_remove(bookmark.identity.clone());
    assert_eq!(window.get_bookmarks().row_count(), 0);
    commands.borrow_mut().clear();
    window.invoke_bookmark_open(1, bookmark.identity);
    assert!(
        commands.borrow().is_empty(),
        "stale identities cannot navigate"
    );
    window.invoke_tab_close_active(1);
    window.invoke_tab_close_active(1);
    assert_eq!(
        window.get_right_tabs().row_count(),
        1,
        "last tab stays open"
    );
    window.hide().unwrap();
}
