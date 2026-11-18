//! Real editor events protect a draft from late settings and cross-shell picks.
#[path = "support/ui.rs"]
mod ui;

use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, install};
use slint::{
    ComponentHandle,
    platform::{Key, WindowEvent},
};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

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
    let mut ui = ui::TestUi::bare();
    let window = ui.window.clone_strong();
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
        system_locations: Vec::new(),
        network_position: 0,
        shell: ShellFlavor::PowerShell,
        locations: vec![],
    });
    midnight_ui::refresh(&window, &presenter.borrow());
    ui.draw();
    window.invoke_focus_command();
    commands.borrow_mut().clear();
    key(&window, "echo ");
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            windows_shell: WindowsShell::Cmd,
            ..Default::default()
        }))));
    midnight_ui::refresh(&window, &presenter.borrow());
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
    let literal_draft = format!("echo folder & echo INJECTED{}", std::path::MAIN_SEPARATOR);
    assert_eq!(window.get_command_text().as_str(), literal_draft);
    let cursor = window.get_command_cursor();
    window.invoke_terminal_changed(1);
    assert_eq!(
        window.get_terminal_index(),
        0,
        "a draft containing shell syntax keeps its original interpreter"
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
                command: literal_draft,
                shell: ShellFlavor::PowerShell,
            },
        }),
    });
    midnight_ui::refresh(&window, &presenter.borrow());
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
