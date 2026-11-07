use midnight_core::*;
#[path = "support/selection.rs"]
mod selection;
use midnight_ui::{MainWindow, Presenter, Theme, install};
use slint::{
    ComponentHandle, Model,
    platform::{
        Key, Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

struct TestPlatform(Rc<MinimalSoftwareWindow>, Rc<Cell<Duration>>);
#[cfg(windows)]
#[path = "support/windows_keys.rs"]
mod platform_keys;
#[cfg(not(windows))]
#[path = "support/default_keys.rs"]
mod platform_keys;
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
    fn duration_since_start(&self) -> Duration {
        self.1.get()
    }
}

fn key(window: &MainWindow, text: impl Into<slint::SharedString>) {
    platform_keys::key(window, text.into());
}
fn hold(window: &MainWindow, key: Key, pressed: bool) {
    window.window().dispatch_event(if pressed {
        WindowEvent::KeyPressed { text: key.into() }
    } else {
        WindowEvent::KeyReleased { text: key.into() }
    });
}
fn complete(
    presenter: &Rc<RefCell<Presenter>>,
    commands: &RefCell<Vec<Command>>,
    pane: PaneId,
    entries: Vec<Entry>,
) {
    let navigation = commands
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::Navigate {
                pane: actual,
                navigation,
                ..
            }
            | Command::Resort {
                pane: actual,
                navigation,
                ..
            }
            | Command::Refresh {
                pane: actual,
                navigation,
                ..
            } if *actual == pane => Some(*navigation),
            _ => None,
        })
        .unwrap();
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    presenter.borrow_mut().apply(Event::ListingFinished {
        pane,
        navigation,
        entries: entries.into(),
        index,
    });
}

#[test]
fn toolbar_and_header_callbacks_transfer_actual_keyboard_focus() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    let time = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform(native.clone(), time.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let left = PathBuf::from("left 文档'Cafe\u{301} 👩🏽‍💻");
    let right = PathBuf::from("right 文件");
    let presenter = Rc::new(RefCell::new(Presenter::new(left.clone(), right.clone())));
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
    native.set_size(slint::PhysicalSize::new(1340, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1340.0, 820.0),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); 1340 * 820];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    assert_eq!(window.get_focused_pane(), 0);
    assert_eq!(
        commands
            .borrow()
            .iter()
            .filter(|command| matches!(command, Command::LoadPreferences))
            .count(),
        1
    );
    window.invoke_action(1, 3);
    assert_eq!(window.get_focused_pane(), 1);
    assert_eq!(window.get_active_pane(), 1);
    window.invoke_sort(0, 0);
    assert_eq!(window.get_focused_pane(), 0);
    assert_eq!(window.get_active_pane(), 0);
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: "[".into() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: "[".into() });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    assert_eq!(
        window.get_focused_pane(),
        -1,
        "path insertion must focus command editing"
    );

    presenter.borrow_mut().apply(Event::DesktopContext {
        shell: ShellFlavor::Posix,
        locations: vec![NavigationLocation {
            id: "d".into(),
            label: "D:".into(),
            path: "drive-d".into(),
            kind: LocationKind::Volume,
            drive_letter: Some('d'),
        }],
    });
    complete(
        &presenter,
        &commands,
        PaneId::Left,
        vec![
            Entry::new(EntryId(0), "folder 文档".into(), EntryKind::Directory),
            Entry::new(EntryId(1), "file 👩🏽‍💻.txt".into(), EntryKind::File),
        ],
    );
    complete(&presenter, &commands, PaneId::Right, vec![]);
    midnight_ui::refresh(&window, &presenter.borrow());
    commands.borrow_mut().clear();
    assert!(
        window.get_command_focused(),
        "command TextInput must have actual focus before editor shortcuts"
    );

    window.invoke_row_click(0, 1, true, false);
    selection::acknowledge(
        &presenter,
        &commands,
        PaneId::Left,
        &[std::path::Path::new("file 👩🏽‍💻.txt")],
    );
    window.invoke_sort(0, 1);
    assert_eq!(window.get_left_sort(), 1);
    assert!(!window.get_left_descending());
    assert!(matches!(
        commands.borrow().last(),
        Some(Command::Resort {
            sort: SortSpec {
                column: SortColumn::Size,
                descending: false
            },
            ..
        })
    ));
    complete(
        &presenter,
        &commands,
        PaneId::Left,
        vec![
            Entry::new(EntryId(1), "file 👩🏽‍💻.txt".into(), EntryKind::File),
            Entry::new(EntryId(0), "folder 文档".into(), EntryKind::Directory),
        ],
    );
    midnight_ui::refresh(&window, &presenter.borrow());
    assert_eq!(window.get_left_cursor(), 0);
    assert!(window.get_left_rows().row_data(0).unwrap().selected);
    window.invoke_sort(0, 1);
    assert!(window.get_left_descending());
    complete(
        &presenter,
        &commands,
        PaneId::Left,
        vec![
            Entry::new(EntryId(0), "folder 文档".into(), EntryKind::Directory),
            Entry::new(EntryId(1), "file 👩🏽‍💻.txt".into(), EntryKind::File),
        ],
    );
    midnight_ui::refresh(&window, &presenter.borrow());
    assert_eq!(window.get_left_cursor(), 1);
    assert!(window.get_left_rows().row_data(1).unwrap().selected);
    window.invoke_sort(0, 2);
    assert_eq!(window.get_left_sort(), 2);
    assert!(
        !window.get_left_descending(),
        "a new header begins in ascending order"
    );
    complete(
        &presenter,
        &commands,
        PaneId::Left,
        vec![
            Entry::new(EntryId(0), "folder 文档".into(), EntryKind::Directory),
            Entry::new(EntryId(1), "file 👩🏽‍💻.txt".into(), EntryKind::File),
        ],
    );
    midnight_ui::refresh(&window, &presenter.borrow());
    window.invoke_row_click(0, 1, true, false);
    window.invoke_row_click(0, 0, false, false);
    commands.borrow_mut().clear();

    hold(&window, Key::Control, true);
    key(&window, "[");
    hold(&window, Key::Control, false);
    assert_eq!(
        window.get_command_text().as_str(),
        "\"left 文档'Cafe\u{301} 👩🏽‍💻\""
    );
    assert!(window.get_command_focused());
    assert!(
        commands.borrow().is_empty(),
        "inserting a path must not run anything"
    );
    window.invoke_set_command("echo OLD tail".into(), 8);
    hold(&window, Key::Shift, true);
    for _ in 0..3 {
        key(&window, Key::LeftArrow);
    }
    hold(&window, Key::Shift, false);
    hold(&window, Key::Control, true);
    key(&window, "]");
    hold(&window, Key::Control, false);
    assert_eq!(
        window.get_command_text().as_str(),
        "echo \"right 文件\" tail"
    );
    assert_eq!(window.get_command_cursor(), 19);
    let before = window.get_command_text();
    presenter.borrow_mut().apply(Event::IconLoaded {
        key: IconKey::folder(),
        pixels: Arc::new(IconPixels::new(1, 1, vec![30, 60, 90, 255]).unwrap()),
    });
    midnight_ui::refresh(&window, &presenter.borrow());
    assert_eq!(window.get_command_text(), before);
    assert_eq!(window.get_command_cursor(), 19);
    assert!(window.get_command_focused());

    window.invoke_set_command("printf \"%s\" \"\"".into(), 13);
    hold(&window, Key::Control, true);
    key(&window, "[");
    hold(&window, Key::Control, false);
    assert_eq!(
        window.get_command_text().as_str(),
        "printf \"%s\" \"\"",
        "path insertion inside quotes must be rejected"
    );
    key(&window, Key::Escape);
    window.invoke_set_command(before, 19);

    window.invoke_activate(1);
    assert_eq!(window.get_active_pane(), 1);
    assert!(window.get_command_focused());
    hold(&window, Key::Shift, true);
    key(&window, "D");
    hold(&window, Key::Shift, false);
    assert!(
        commands.borrow().is_empty(),
        "drive shortcuts must not steal editor text"
    );
    assert_eq!(
        window.get_command_text().as_str(),
        "echo \"right 文件\"D tail"
    );
    key(&window, Key::Return);
    key(&window, Key::Return);
    assert_eq!(commands.borrow().len(), 1);
    let request = match &commands.borrow()[0] {
        Command::LaunchTerminal { request, terminal } => {
            assert_eq!(terminal.working_directory, right);
            assert_eq!(terminal.command, "echo \"right 文件\"D tail");
            assert_eq!(terminal.shell, ShellFlavor::Posix);
            assert_eq!(terminal.mode, TerminalMode::Standard);
            *request
        }
        _ => panic!("Enter must explicitly launch the typed command"),
    };
    assert_eq!(window.get_command_text().as_str(), "");
    presenter.borrow_mut().apply(Event::DesktopActionFinished {
        request,
        result: Ok(DesktopActionResult::TerminalStarted {
            process_id: 1,
            history: CommandHistoryEntry {
                command: "echo \"right 文件\"D tail".into(),
                shell: ShellFlavor::Posix,
            },
        }),
    });
    midnight_ui::refresh(&window, &presenter.borrow());
    commands.borrow_mut().clear();
    let elevated_draft = "printf '%s' '文档 👩🏽‍💻'";
    window.invoke_set_command(elevated_draft.into(), elevated_draft.len() as i32);
    window.invoke_terminal_changed(1);
    assert_eq!(window.get_command_text().as_str(), elevated_draft);
    assert!(
        commands.borrow().is_empty(),
        "choosing sudo must not persist or launch anything"
    );
    key(&window, Key::Return);
    let elevated_request = match &commands.borrow()[0] {
        Command::LaunchTerminal { request, terminal } => {
            assert_eq!(terminal.shell, ShellFlavor::Posix);
            assert_eq!(terminal.mode, TerminalMode::Elevated);
            assert_eq!(terminal.command, elevated_draft);
            assert_eq!(terminal.working_directory, right);
            *request
        }
        _ => panic!("sudo selection must wait for explicit editor Enter"),
    };
    presenter.borrow_mut().apply(Event::DesktopActionFinished {
        request: elevated_request,
        result: Ok(DesktopActionResult::TerminalStarted {
            process_id: 2,
            history: CommandHistoryEntry {
                command: elevated_draft.into(),
                shell: ShellFlavor::Posix,
            },
        }),
    });
    midnight_ui::refresh(&window, &presenter.borrow());
    window.invoke_terminal_changed(0);
    window.invoke_activate(0);
    window.invoke_focus_files();
    // Establish the focused-item Properties case after outstanding selection
    // intents, just as the backend's ordered Clear result would do.
    window.invoke_bulk_selection(0, 1);
    selection::acknowledge(&presenter, &commands, PaneId::Left, &[]);
    midnight_ui::refresh(&window, &presenter.borrow());
    commands.borrow_mut().clear();
    window.invoke_row_click(0, 0, false, false);
    hold(&window, Key::Alt, true);
    key(&window, Key::Return);
    hold(&window, Key::Alt, false);
    assert_eq!(
        commands.borrow().len(),
        1,
        "Alt+Enter must not fall through to navigation"
    );
    assert!(
        matches!(&commands.borrow()[0], Command::ShowSelectionProperties { fallback, .. }
            if fallback.as_ref() == Some(&PathBuf::from("folder 文档")))
    );
    commands.borrow_mut().clear();
    hold(&window, Key::Control, true);
    hold(&window, Key::Alt, true);
    key(&window, Key::Return);
    hold(&window, Key::Alt, false);
    hold(&window, Key::Control, false);
    assert_eq!(commands.borrow().len(), 1);
    assert!(matches!(
        commands.borrow()[0],
        Command::ShowSelectionProperties { .. }
    ));
    commands.borrow_mut().clear();
    // F5 now captures a copy draft and deliberately contains file shortcuts.
    key(&window, Key::F5);
    assert!(!window.get_copy_draft_token().is_empty());
    assert_eq!(window.get_focused_pane(), -1);
    assert!(
        commands.borrow().is_empty(),
        "opening a copy draft does not queue a copy"
    );
    key(&window, Key::Escape);
    assert!(window.get_copy_draft_token().is_empty());
    assert_eq!(window.get_focused_pane(), 0);
    // F6 similarly waits for explicit recovery confirmation and releases focus on Escape.
    key(&window, Key::F6);
    assert!(!window.get_copy_draft_token().is_empty());
    assert!(window.get_copy_draft_move());
    assert_eq!(window.get_focused_pane(), -1);
    assert!(
        commands.borrow().is_empty(),
        "opening a Move draft does not queue a transfer"
    );
    key(&window, Key::Escape);
    assert!(window.get_copy_draft_token().is_empty());
    assert_eq!(window.get_focused_pane(), 0);
    hold(&window, Key::Control, true);
    hold(&window, Key::Shift, true);
    key(&window, "S");
    hold(&window, Key::Shift, false);
    hold(&window, Key::Control, false);
    assert!(matches!(
        commands.borrow()[0],
        Command::ComputeFolderSizes {
            pane: PaneId::Left,
            ..
        }
    ));
    commands.borrow_mut().clear();
    hold(&window, Key::Shift, true);
    key(&window, "D");
    hold(&window, Key::Shift, false);
    assert!(
        matches!(&commands.borrow()[0], Command::Navigate { pane: PaneId::Left, path, .. } if path == &PathBuf::from("drive-d"))
    );

    complete(
        &presenter,
        &commands,
        PaneId::Left,
        (0..100)
            .map(|i| Entry::new(EntryId(i), format!("row-{i}").into(), EntryKind::Directory))
            .collect(),
    );
    midnight_ui::refresh(&window, &presenter.borrow());
    native.set_size(slint::PhysicalSize::new(1340, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1340.0, 820.0),
    });
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(200.0, 350.0),
            delta_x: 0.0,
            delta_y: -260.0,
        });
    time.set(Duration::from_secs(1));
    slint::platform::update_timers_and_animations();
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    presenter.borrow_mut().apply(Event::IconLoaded {
        key: IconKey::drive(),
        pixels: Arc::new(IconPixels::new(1, 1, vec![90, 60, 30, 255]).unwrap()),
    });
    midnight_ui::refresh(&window, &presenter.borrow());
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    let typed = "watch typing 👩🏽‍💻";
    window.invoke_set_command(typed.into(), typed.len() as i32);
    let committed = presenter.borrow().visible_directory(PaneId::Left).unwrap();
    let sent = commands.clone();
    assert!(presenter.borrow_mut().refresh_if_changed(
        PaneId::Left,
        committed.navigation,
        &|command| {
            sent.borrow_mut().push(command);
            true
        }
    ));
    midnight_ui::refresh(&window, &presenter.borrow());
    assert_eq!(window.get_left_rows().row_count(), 100);
    assert_eq!(window.get_command_text().as_str(), typed);
    assert!(window.get_command_focused());
    let mut refreshed = vec![Entry::new(
        EntryId(999),
        "new-first-row".into(),
        EntryKind::Directory,
    )];
    refreshed.extend((0..100).map(|i| {
        Entry::new(
            EntryId(1000 + i),
            format!("row-{i}").into(),
            EntryKind::Directory,
        )
    }));
    complete(&presenter, &commands, PaneId::Left, refreshed);
    midnight_ui::refresh(&window, &presenter.borrow());
    assert_eq!(
        window.get_left_cursor(),
        1,
        "the cursor must follow its old raw path"
    );
    assert_eq!(window.get_command_text().as_str(), typed);
    assert_eq!(window.get_command_cursor(), typed.len() as i32);
    assert!(window.get_command_focused());
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    let theme = window.global::<Theme>();
    let list_top = window.get_left_list_top();
    for pressed in [true, false] {
        let position = slint::LogicalPosition::new(100.0, list_top + theme.get_row_height() / 2.0);
        let button = slint::platform::PointerEventButton::Left;
        window.window().dispatch_event(if pressed {
            WindowEvent::PointerPressed { position, button }
        } else {
            WindowEvent::PointerReleased { position, button }
        });
    }
    assert!(
        window.get_left_cursor() >= 8,
        "wheel must move virtualized rows before the next click; cursor={}",
        window.get_left_cursor()
    );
    window.invoke_preferences_changed(2, 0, 0);
    window.invoke_preferences_changed(2, 1, 1);
    assert_eq!(window.get_theme_index(), 2);
    assert_eq!(window.get_density_index(), 1);
    let theme = window.global::<Theme>();
    assert_eq!(theme.get_row_height(), 40.0);
    assert!(theme.get_control_height() >= 40.0);
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    assert!(
        pixels[0].r > 200 && pixels[0].g > 200 && pixels[0].b > 200,
        "Light must change the rendered window surface"
    );
    assert!(
        theme.get_text().red() < 100,
        "Light text must contrast with its surface"
    );
    window.invoke_focus_files();
    key(&window, Key::Home);
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    let top = window.get_left_list_top();
    for pressed in [true, false] {
        let position = slint::LogicalPosition::new(100.0, top + 1.5 * theme.get_row_height());
        let button = slint::platform::PointerEventButton::Left;
        window.window().dispatch_event(if pressed {
            WindowEvent::PointerPressed { position, button }
        } else {
            WindowEvent::PointerReleased { position, button }
        });
    }
    assert_eq!(
        window.get_left_cursor(),
        1,
        "touch hit testing must use the 40px row geometry"
    );
    key(&window, Key::End);
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    for pressed in [true, false] {
        let position = slint::LogicalPosition::new(100.0, top + theme.get_row_height() / 2.0);
        let button = slint::platform::PointerEventButton::Left;
        window.window().dispatch_event(if pressed {
            WindowEvent::PointerPressed { position, button }
        } else {
            WindowEvent::PointerReleased { position, button }
        });
    }
    assert!(
        window.get_left_cursor() >= 85,
        "End reveal must use the touch row height, cursor={}",
        window.get_left_cursor()
    );
    window.invoke_preferences_changed(1, 1, 0);
    window.invoke_preferences_changed(1, 0, 1);
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    assert!(
        pixels[0].r < 50 && pixels[0].r == pixels[0].g && pixels[0].g == pixels[0].b,
        "Dark must render neutral surfaces"
    );
    assert_eq!(theme.get_row_height(), 22.0);
    window.invoke_preferences_changed(0, 0, 0);
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    assert!(
        pixels[0].b > pixels[0].r,
        "Blue must restore the default blue surface"
    );
    presenter.borrow_mut().apply(Event::DesktopContext {
        shell: ShellFlavor::PowerShell,
        locations: vec![],
    });
    window.invoke_set_command("".into(), 0);
    window.invoke_command_edited(false);
    midnight_ui::refresh(&window, &presenter.borrow());
    let windows_draft = "echo 文档 👩🏽‍💻 'unchanged draft'";
    for (index, shell) in [(0, ShellFlavor::PowerShell), (1, ShellFlavor::Cmd)] {
        commands.borrow_mut().clear();
        window.invoke_set_command("".into(), 0);
        window.invoke_command_edited(false);
        window.invoke_terminal_changed(index);
        window.invoke_set_command(windows_draft.into(), windows_draft.len() as i32);
        window.invoke_command_edited(true);
        assert_eq!(window.get_command_text().as_str(), windows_draft);
        assert!(window.get_command_focused());
        assert!(
            commands
                .borrow()
                .iter()
                .all(|command| matches!(command, Command::SavePreferences { .. })),
            "terminal selection may save preference, never launch a command"
        );
        key(&window, Key::Return);
        let request = commands
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::LaunchTerminal { request, terminal } => {
                    assert_eq!(terminal.shell, shell);
                    assert_eq!(terminal.mode, TerminalMode::Standard);
                    assert_eq!(terminal.command, windows_draft);
                    assert_eq!(terminal.working_directory, PathBuf::from("drive-d"));
                    Some(*request)
                }
                _ => None,
            })
            .expect("editor Enter must launch the selected Windows shell");
        presenter.borrow_mut().apply(Event::DesktopActionFinished {
            request,
            result: Ok(DesktopActionResult::TerminalStarted {
                process_id: 3,
                history: CommandHistoryEntry {
                    command: windows_draft.into(),
                    shell,
                },
            }),
        });
        midnight_ui::refresh(&window, &presenter.borrow());
    }
    commands.borrow_mut().clear();
    window.invoke_set_command("echo ".into(), 5);
    hold(&window, Key::Control, true);
    key(&window, "]");
    hold(&window, Key::Control, false);
    assert_eq!(window.get_command_text().as_str(), "echo \"right 文件\"");
    window.invoke_terminal_changed(0);
    assert_eq!(
        window.get_terminal_index(),
        1,
        "a nonempty cmd draft must keep its shell"
    );
    assert_eq!(
        window.get_command_text().as_str(),
        "echo \"right 文件\"",
        "switching shell must keep existing inserted tokens unchanged"
    );
    assert!(
        commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::LaunchTerminal { .. }))
    );
    window.hide().unwrap();
}
