//! Exercise the installed command bridge, not just shortcut callback routing.
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, install};
use slint::{
    ComponentHandle,
    platform::{
        Key, Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn control(window: &MainWindow, text: impl Into<slint::SharedString>) {
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
}

#[test]
fn control_enter_appends_literal_basename_and_control_p_uses_active_panel() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let left = PathBuf::from("left 文档");
    let right = PathBuf::from("right 👩🏽‍💻");
    let name = "Žlutý ' $(data) & 文档.txt";
    let presenter = Rc::new(RefCell::new(Presenter::new(left.clone(), right.clone())));
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
    presenter.borrow_mut().apply(Event::DesktopContext {
        shell: ShellFlavor::Posix,
        locations: vec![],
    });
    let entries: Arc<[Entry]> =
        vec![Entry::new(EntryId(1), left.join(name), EntryKind::File)].into();
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    presenter.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        entries,
        index,
    });
    midnight_ui::refresh(&window, &presenter.borrow());
    native.set_size(slint::PhysicalSize::new(1340, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1340.0, 820.0),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); 1340 * 820];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    window.invoke_row_click(0, 0, false, false);
    commands.borrow_mut().clear();
    window.invoke_set_command("echo prefix".into(), 0);
    control(&window, Key::Return);
    assert_eq!(
        window.get_command_text().as_str(),
        format!(
            "echo prefix {}",
            quote_shell_path(Path::new(name), ShellFlavor::Posix).unwrap()
        )
    );
    assert!(window.get_command_focused());
    assert!(
        commands.borrow().is_empty(),
        "insertion never opens a file or launches a terminal"
    );

    for (pane, path) in [(0, &left), (1, &right)] {
        window.invoke_activate(pane);
        window.invoke_set_command("echo ".into(), 5);
        control(&window, "p");
        assert_eq!(
            window.get_command_text().as_str(),
            format!(
                "echo {}",
                quote_shell_path(path, ShellFlavor::Posix).unwrap()
            )
        );
        assert!(window.get_command_focused());
        assert!(commands.borrow().is_empty());
    }
    window.hide().unwrap();
}
