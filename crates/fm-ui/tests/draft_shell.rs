//! Real editor events protect a draft from late settings and cross-shell picks.
use fm_core::*;
use fm_ui::{MainWindow, Presenter, install};
use slint::{
    ComponentHandle,
    platform::{
        Key, Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}
fn key(window: &MainWindow, text: impl Into<slint::SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

#[test]
fn nonempty_editor_and_inserted_paths_keep_their_shell_across_late_load_and_picker_changes() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let presenter = Rc::new(RefCell::new(Presenter::new(
        "folder & echo INJECTED".into(),
        "right".into(),
    )));
    let commands = Rc::new(RefCell::new(vec![]));
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
        shell: ShellFlavor::PowerShell,
        locations: vec![],
    });
    fm_ui::refresh(&window, &presenter.borrow());
    native.set_size(slint::PhysicalSize::new(1340, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1340.0, 820.0),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); 1340 * 820];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    window.invoke_focus_command();
    commands.borrow_mut().clear();
    key(&window, "echo ");
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(UserPreferences {
            windows_shell: WindowsShell::Cmd,
            ..Default::default()
        })));
    fm_ui::refresh(&window, &presenter.borrow());
    assert_eq!(
        window.get_terminal_index(),
        0,
        "late defaults must not reinterpret a manually typed draft"
    );
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    key(&window, "[");
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    let literal_draft = "echo \"folder & echo INJECTED\"";
    assert_eq!(window.get_command_text().as_str(), literal_draft);
    let cursor = window.get_command_cursor();
    window.invoke_terminal_changed(1);
    assert_eq!(
        window.get_terminal_index(),
        0,
        "a PS literal containing & cannot be reinterpreted by cmd"
    );
    assert_eq!(window.get_command_text().as_str(), literal_draft);
    assert_eq!(window.get_command_cursor(), cursor);
    assert!(window.get_command_focused());
    assert!(
        commands.borrow().is_empty(),
        "rejected picks must not save or launch"
    );
    key(&window, Key::Return);
    let first = match &commands.borrow()[0] {
        Command::LaunchTerminal { request, terminal } => {
            assert_eq!(terminal.shell, ShellFlavor::PowerShell);
            assert_eq!(terminal.command, literal_draft);
            assert_eq!(
                terminal.working_directory,
                PathBuf::from("folder & echo INJECTED")
            );
            *request
        }
        _ => panic!("only explicit Enter may launch the bound grammar"),
    };
    presenter.borrow_mut().apply(Event::DesktopActionFinished {
        request: first,
        result: Ok(DesktopActionResult::TerminalStarted {
            process_id: 1,
            history: CommandHistoryEntry {
                command: literal_draft.into(),
                shell: ShellFlavor::PowerShell,
            },
        }),
    });
    fm_ui::refresh(&window, &presenter.borrow());
    assert_eq!(window.get_command_text().as_str(), "");
    window.invoke_terminal_changed(1);
    assert_eq!(
        window.get_terminal_index(),
        1,
        "an empty editor may adopt the new default"
    );
    commands.borrow_mut().clear();
    let cmd_draft = "echo \"文档 & literal\"";
    key(&window, cmd_draft);
    key(&window, Key::Return);
    assert!(
        matches!(&commands.borrow()[0], Command::LaunchTerminal { terminal, .. } if terminal.shell == ShellFlavor::Cmd && terminal.command == cmd_draft)
    );
    window.hide().unwrap();
}
