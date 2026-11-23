//! The output window of a Run directly command: opened by its session, its lines as they
//! arrive (standard error marked), Stop, Copy all, Clear, Find, Close (stopping a program
//! that still runs), "Close when finished successfully", and its layout at 8, 12 and
//! 32 px fonts in both densities, a light and a dark theme.
#[path = "support/window_polish.rs"]
mod polish;
use crate::support::modeless;
use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::{CommandOutputRow, CommandOutputWindow, MainWindow, Presenter};
use modeless::{Windows, accessible_item, click, draw};
use slint::{
    ComponentHandle, ModelRc, VecModel,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::Arc,
    time::{Duration, SystemTime},
};

type Natives = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;

struct App {
    natives: Natives,
    /// The requests whose windows opened, in the order of `natives` after the main one.
    opened: RefCell<Vec<u64>>,
    main: MainWindow,
    state: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
}

fn command(close_on_success: bool) -> UserCommand {
    UserCommand {
        id: UserCommandId::new("git-clean").unwrap(),
        name: "git clean".into(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Program {
            executable: std::env::current_exe().unwrap(),
            arguments: vec!["clean".into(), "-fxd".into()],
        },
        output: UserCommandOutput {
            mode: UserCommandOutputMode::Window,
            close_on_success,
        },
    }
}

fn app(close_on_success: bool) -> App {
    let natives = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(natives.clone()))).unwrap();
    let main = MainWindow::new().unwrap();
    let root = std::env::temp_dir();
    let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
    state
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(
            AcceptedPreferences {
                preferences: UserPreferences {
                    user_commands: Arc::new(vec![command(close_on_success)]),
                    ..Default::default()
                },
                comparators: FileComparatorSnapshot::default(),
            },
        ))));
    let commands = Rc::new(RefCell::new(Vec::new()));
    midnight_ui::install(
        &main,
        state.clone(),
        Rc::new({
            let commands = commands.clone();
            move |command| {
                commands.borrow_mut().push(command);
                true
            }
        }),
    );
    main.show().unwrap();
    midnight_ui::refresh(&main, &state.borrow());
    App {
        natives,
        opened: RefCell::default(),
        main,
        state,
        commands,
    }
}

impl App {
    fn notify(&self, session: &Arc<UserCommandOutputSession>, opened: bool) {
        let before = self.natives.borrow().len();
        self.state
            .borrow_mut()
            .apply(Event::UserCommandOutput(Box::new(UserCommandOutputEvent {
                session: session.clone(),
                opened,
            })));
        midnight_ui::refresh(&self.main, &self.state.borrow());
        slint::platform::update_timers_and_animations();
        if self.natives.borrow().len() > before {
            self.opened.borrow_mut().push(session.request.0);
        }
    }
    fn output(&self) -> Rc<MinimalSoftwareWindow> {
        self.natives.borrow().last().unwrap().clone()
    }
    /// Whether the output window of request `id` is shown: the presenter keeps its
    /// session exactly while it is.
    fn is_open(&self, id: u64) -> bool {
        let opened = self.opened.borrow();
        let Some(index) = opened.iter().position(|request| *request == id) else {
            return false;
        };
        self.natives.borrow()[index + 1].window().is_visible()
    }
}

fn session(request: u64) -> Arc<UserCommandOutputSession> {
    let session = UserCommandOutputSession::new(
        UserCommandRequestId(request),
        UserCommandId::new("git-clean").unwrap(),
        "git clean".into(),
        1,
    );
    session.begin_run(OutputRun {
        index: 0,
        command_line: r#""C:\Program Files\Git\bin\git.exe" clean -fxd"#.into(),
        directory: r"C:\work\文档".into(),
        started: SystemTime::now(),
        process_id: 42,
        exit: None,
        duration: None,
        stopped: false,
    });
    session
}

fn item_exists(native: &MinimalSoftwareWindow, role: AccessibleRole, label: &str) -> bool {
    accessible_item(native, role, label).is_some()
}

#[test]
fn an_output_window_shows_the_lines_as_they_arrive_and_stop_ends_the_run() {
    let app = app(false);
    let running = session(5);
    app.notify(&running, true);
    assert_eq!(app.natives.borrow().len(), 2, "the output window opened");
    let native = app.output();
    running.append(
        OutputStream::Stdout,
        vec!["Removing build/".into(), "Removing target/".into()],
        Some("Counting objects:  50%".into()),
    );
    running.append(
        OutputStream::Stderr,
        vec!["warning: could not remove locked".into()],
        None,
    );
    app.notify(&running, false);
    draw(&native, 900, 600, "output-running");
    for label in [
        "Removing build/",
        "Removing target/",
        "Counting objects:  50%",
        "Error: warning: could not remove locked",
    ] {
        assert!(
            item_exists(&native, AccessibleRole::ListItem, label),
            "{label}"
        );
    }
    assert!(item_exists(
        &native,
        AccessibleRole::Text,
        "Status: Running…"
    ));
    // Escape never stops a running program.
    native.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    native.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
    assert!(!running.stop_requested());
    assert!(app.is_open(5));
    click(&native, "Stop");
    assert!(running.stop_requested());
    assert!(
        app.commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::CancelUserCommand(UserCommandRequestId(5))))
    );
    running.end_run(0, Some(ProgramExit::Code(1)), Duration::from_millis(2500));
    running.finish(true, None);
    app.notify(&running, false);
    draw(&native, 900, 600, "output-stopped");
    assert!(item_exists(
        &native,
        AccessibleRole::Text,
        "Status: Stopped · Exit code 1 · 2.5 s"
    ));
    // Copy all goes to the clipboard worker; Clear empties the list.
    let before = app.commands.borrow().len();
    click(&native, "Copy all");
    assert!(
        app.commands.borrow().len() > before,
        "the text went to the clipboard"
    );
    click(&native, "Clear");
    draw(&native, 900, 600, "output-cleared");
    assert!(!item_exists(
        &native,
        AccessibleRole::ListItem,
        "Removing build/"
    ));
    // A finished window closes with Escape.
    native.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    assert!(!app.is_open(5));
}

#[test]
fn closing_a_running_window_stops_its_program() {
    let app = app(false);
    let running = session(7);
    app.notify(&running, true);
    let native = app.output();
    draw(&native, 900, 600, "output-before-close");
    click(&native, "Close");
    assert!(running.stop_requested());
    assert!(!app.is_open(7));
    // Its late news opens nothing.
    running.finish(true, None);
    let count = app.natives.borrow().len();
    app.notify(&running, false);
    assert_eq!(app.natives.borrow().len(), count);
}

#[test]
fn find_moves_between_matching_lines() {
    let app = app(false);
    let shown = session(3);
    shown.append(
        OutputStream::Stdout,
        (0..40)
            .map(|index| {
                if index % 10 == 3 {
                    format!("Removing item {index}")
                } else {
                    format!("keeping item {index}")
                }
            })
            .collect(),
        None,
    );
    app.notify(&shown, true);
    let native = app.output();
    let window = native.window();
    draw(&native, 900, 600, "output-find");
    window.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    window.dispatch_event(WindowEvent::KeyPressed { text: "f".into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: "f".into() });
    window.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    draw(&native, 900, 600, "output-find-open");
    for character in "removing".chars() {
        window.dispatch_event(WindowEvent::KeyPressed {
            text: character.to_string().into(),
        });
        window.dispatch_event(WindowEvent::KeyReleased {
            text: character.to_string().into(),
        });
    }
    window.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Return.into(),
    });
    window.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Return.into(),
    });
    draw(&native, 900, 600, "output-find-first");
    assert!(item_exists(&native, AccessibleRole::Text, "Line 5 of 41"));
    click(&native, "Next");
    assert!(item_exists(&native, AccessibleRole::Text, "Line 15 of 41"));
    click(&native, "Previous");
    click(&native, "Previous");
    assert!(
        item_exists(&native, AccessibleRole::Text, "Line 35 of 41"),
        "Previous wraps around"
    );
}

#[test]
fn close_when_finished_successfully_closes_after_a_zero_exit_only() {
    let app = app(true);
    let failing = session(1);
    app.notify(&failing, true);
    failing.end_run(0, Some(ProgramExit::Code(2)), Duration::from_millis(300));
    failing.finish(false, None);
    app.notify(&failing, false);
    assert!(app.is_open(1), "a failure stays open");
    let passing = session(2);
    app.notify(&passing, true);
    let native = app.output();
    draw(&native, 900, 600, "output-close-on-success");
    assert!(item_exists(
        &native,
        AccessibleRole::Checkbox,
        "Close when finished successfully"
    ));
    passing.end_run(0, Some(ProgramExit::Code(0)), Duration::from_millis(300));
    passing.finish(false, None);
    app.notify(&passing, false);
    slint::platform::update_timers_and_animations();
    assert!(!app.is_open(2), "closed by itself");
    // Unticking it in the window is remembered with the command.
    let window = app.natives.borrow()[1].clone();
    draw(&window, 900, 600, "output-close-on-success-failed");
    let check = accessible_item(
        &window,
        AccessibleRole::Checkbox,
        "Close when finished successfully",
    )
    .expect("the check box");
    modeless::click_item(&window, &check, "Close when finished successfully");
    assert!(app.commands.borrow().iter().any(|command| matches!(
        command,
        Command::SavePreferences { preferences, .. }
            if !preferences.user_commands[0].output.close_on_success
    )));
}

fn sample_rows() -> ModelRc<CommandOutputRow> {
    let mut rows = vec![CommandOutputRow {
        text: r#"▶ "C:\Program Files\Git\bin\git.exe" clean -fxd"#.into(),
        kind: 2,
        partial: false,
    }];
    for index in 0..30 {
        rows.push(CommandOutputRow {
            text: format!("Removing target/debug/build/文档-{index:02}/output").into(),
            kind: 0,
            partial: false,
        });
    }
    rows.push(CommandOutputRow {
        text: format!(
            "warning: {}",
            "a very long line that scrolls sideways ".repeat(6)
        )
        .into(),
        kind: 1,
        partial: false,
    });
    rows.push(CommandOutputRow {
        text: "Exit code 1 · 2.4 s".into(),
        kind: 2,
        partial: false,
    });
    ModelRc::new(VecModel::from(rows))
}

#[test]
fn the_output_window_fits_small_and_large_fonts_in_both_densities_and_themes() {
    let natives = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(natives.clone()))).unwrap();
    let window = CommandOutputWindow::new().unwrap();
    window.set_heading("git clean".into());
    if cfg!(windows) {
        window.set_mono_family("Consolas".into());
    }
    window.set_rows(sample_rows());
    // As the window counts them: the longest line in characters, CJK twice.
    window.set_columns(9 + 39 * 6);
    window.set_command_line(r#""C:\Program Files\Git\bin\git.exe" clean -fxd"#.into());
    window.set_directory(r"C:\work\文档".into());
    window.set_started("2026-11-23 15:04:05".into());
    window.set_status("Failed · Exit code 1 · 2.4 s".into());
    window.set_failed(true);
    window.set_close_on_success_available(true);
    window.set_note("Copied 33 lines.".into());
    window.show().unwrap();
    let native = natives.borrow()[0].clone();
    // Light (2) and dark (1) themes; 8, 12 and 32 px; desktop (0) and touch (1).
    for (theme, density, font) in [(2, 0, 8), (2, 0, 12), (1, 1, 12), (1, 0, 32), (2, 1, 32)] {
        window.invoke_apply_preferences(theme, density, "".into(), font, font);
        for running in [false, true] {
            window.set_running(running);
            window.set_failed(!running);
            for (width, height) in [(320, 200), (760, 520), (1340, 820)] {
                let name = format!(
                    "output-t{theme}-d{density}-f{font}-{}",
                    if running { "running" } else { "failed" }
                );
                draw(&native, width, height, &name);
                let findings = polish::layout_findings(native.window());
                assert!(
                    findings.is_empty(),
                    "{name} at {width}x{height}: {findings:#?}"
                );
                // The actions stay reachable at every size.
                for label in ["Stop", "Copy all", "Clear", "Find", "Close"] {
                    let button = accessible_item(&native, AccessibleRole::Button, label)
                        .unwrap_or_else(|| panic!("{label} in {name}"));
                    let (x, y, w, h) = polish::bounds(&button);
                    assert!(
                        x >= 0.0
                            && y >= 0.0
                            && x + w <= width as f32 + 0.5
                            && y + h <= height as f32 + 0.5,
                        "{label} outside the {width}x{height} window in {name}: {:?}",
                        (x, y, w, h)
                    );
                }
            }
        }
    }
    window.hide().unwrap();
}
