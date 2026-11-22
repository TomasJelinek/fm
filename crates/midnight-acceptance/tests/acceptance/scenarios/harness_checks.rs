//! Checks of the harness itself, so that the scenarios built on it can trust it:
//!
//! 0. `App::start` returns with both panels showing their folders, never before a
//!    panel started its first listing.
//! 1. The Slint clock: held, it stands still while real time passes; a timer started on
//!    it fires exactly when the clock is advanced to its timeout, not before; resumed,
//!    it runs on from where it stood.
//! 2. `App::pump` settles frame by frame (a 20 ms timer fires again and again), where
//!    one `idle` jumps once.
//! 3. A tool window that closed is left alone: `layout_tool` and `render` do nothing,
//!    `screenshot_window` notes it, and nothing panics.
//! 4. The Copy dialog's link question is counted (`App::links_inspected`).
//! 5. The gated volumes' variables: unset, `SKIP:` with the variable's name; an unknown
//!    name is an error; set, a fresh folder there that goes with its guard.
//! 6. Production's startup deadline: a settings read slower than 10 s shows the defaults
//!    with "Settings are still loading; defaults are shown.", and the read applies when
//!    it arrives.
//! 7. Pruning keeps runs that are still going and other folders, and only the newest
//!    finished runs.
//! 8. No terminal ever starts: without the desktop stand-in the command line reports
//!    "Terminal unavailable" and nothing is launched; with it (`Config::desktop`) the
//!    command line's terminal is recorded, not started; a Custom Command in terminal
//!    mode goes to the recording launcher of `Config::user_commands`, never the
//!    platform's terminal.
//! 9. No browser ever starts: a web link a viewer followed (`App::open_url`, as
//!    `Command::OpenUrl`) is refused with the reason without the desktop stand-in and
//!    launches nothing; with it the address is recorded (`Launch::OpenUrl`) and the
//!    viewer would say it opened.
use crate::{
    Context,
    app::{self, App, Config},
    files_support::{self as files, Pending},
    fixtures, input,
    launches::{Launch, Launches, UserCommands},
    report,
};
use midnight_core::{
    Cancellation, PaneId, PreferenceTheme, TerminalMode, TerminalRequest, TerminalService,
    UserCommandProcess, UserCommandProcessInvocation, UserCommandService,
};
use slint::platform::{Key, WindowAdapter};
use std::{
    cell::Cell,
    fs,
    path::Path,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

pub const TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// The status line production shows while the deadline revealed the defaults.
const SETTINGS_PENDING: &str = "Settings are still loading; defaults are shown.";
/// What the command line submits in the terminal checks.
const COMMAND: &str = "echo midnight-acceptance";
/// A web link a viewer followed; no browser ever gets it.
const LINK: &str = "https://example.com/midnight-acceptance";

pub fn scenario() -> crate::Scenario {
    crate::Scenario {
        name: "harness_checks",
        area: "harness",
        summary: "The harness itself: a held Slint clock stands still and fires a timer exactly when advanced to it; pump settles frame by frame; a closed tool window is left alone; the Copy dialog's link question is counted; no terminal ever starts (refused without the desktop stand-in, recorded with it and for a terminal-mode Custom Command); no browser ever starts (a followed web link refused without the stand-in, recorded with it); gated volumes report SKIP when unset; a settings read slower than the 10 s startup deadline shows the defaults first; pruning keeps runs still going.",
        run: checks,
        timeout: TIMEOUT,
    }
}

fn checks(context: &mut Context) -> Result<(), String> {
    let mut failures = Vec::new();
    let pending = Pending::default();
    let root = context.fixture.root.clone();
    let other = files::scratch(context, "other")?;
    {
        let mut app = App::start(Config::new(&root, &other));
        for (what, result) in [
            ("the start", start_folders(&app, &root, &other)),
            ("the held clock", clock(&mut app)),
            ("pump against idle", frames(&mut app)),
            ("a closed tool window", closed_window(&mut app, context)),
            ("the Copy dialog's link question", links_counted(&mut app)),
            ("the command line without a desktop", no_terminal(&mut app)),
            ("a followed link without a desktop", no_url(&mut app)),
        ] {
            if let Err(error) = result {
                failures.push(format!("{what}: {error}"));
            }
        }
        files::dismiss_all(&mut app);
        if let Err(error) = app.shutdown() {
            failures.push(format!("shutdown: {error}"));
        }
    }
    if let Err(error) = desktop_terminal(&root, &other) {
        failures.push(format!(
            "the command line with the desktop stand-in: {error}"
        ));
    }
    if let Err(error) = desktop_url(&root, &other) {
        failures.push(format!(
            "a followed link with the desktop stand-in: {error}"
        ));
    }
    if let Err(error) = user_command_terminal(context, &other) {
        failures.push(format!("a Custom Command in terminal mode: {error}"));
    }
    if let Err(error) = gated(context) {
        failures.push(format!("gated volumes: {error}"));
    }
    if let Err(error) = late_settings(&root) {
        failures.push(format!("a slow settings read: {error}"));
    }
    if let Err(error) = pruning(context) {
        failures.push(format!("pruning: {error}"));
    }
    files::report_with(context, &failures, &pending)
}

/// The held clock's checks; the clock runs again afterwards, whatever they found.
fn clock(app: &mut App) -> Result<(), String> {
    let result = held_clock(app);
    app.resume_clock();
    result
}

fn held_clock(app: &mut App) -> Result<(), String> {
    app.hold_clock();
    let held = app.clock.now();
    app.pump(Duration::from_millis(60));
    if app.clock.now() != held {
        return Err(format!(
            "it moved from {held:?} to {:?} while held",
            app.clock.now()
        ));
    }
    let fired = Rc::new(Cell::new(0));
    let counter = fired.clone();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::SingleShot,
        Duration::from_millis(400),
        move || counter.set(counter.get() + 1),
    );
    app.pump(Duration::from_millis(30));
    app.advance(Duration::from_millis(390));
    if fired.get() != 0 {
        return Err("a 400 ms timer fired after 390 ms of the held clock".into());
    }
    app.advance(Duration::from_millis(10));
    if fired.get() != 1 {
        return Err(format!(
            "a 400 ms timer fired {} time(s) when the clock reached 400 ms",
            fired.get()
        ));
    }
    let stood = app.clock.now();
    if stood != held + Duration::from_millis(400) {
        return Err(format!(
            "advancing 400 ms moved it from {held:?} to {stood:?}"
        ));
    }
    app.resume_clock();
    let resumed = app.clock.now();
    app.pump(Duration::from_millis(30));
    if resumed < stood || app.clock.now() <= resumed {
        return Err(format!(
            "resumed at {stood:?} it read {resumed:?}, then {:?}",
            app.clock.now()
        ));
    }
    Ok(())
}

fn frames(app: &mut App) -> Result<(), String> {
    let ticks = Rc::new(Cell::new(0));
    let counter = ticks.clone();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(20),
        move || counter.set(counter.get() + 1),
    );
    app.settle();
    ticks.set(0);
    app.pump(Duration::from_millis(300));
    let pumped = ticks.get();
    ticks.set(0);
    app.idle(Duration::from_millis(300));
    let idled = ticks.get();
    timer.stop();
    // Pump settles about every 10 ms, so the 20 ms timer fires again and again: at least
    // three times even where one settle takes far longer than a frame (a debug build under
    // WSL fired it four times). Idle settles once, so the timer fires once or twice.
    if pumped < 3 || idled > 2 || pumped <= idled {
        return Err(format!(
            "a 20 ms timer fired {pumped} time(s) in 300 ms of pump and {idled} in 300 ms of \
             idle"
        ));
    }
    Ok(())
}

fn closed_window(app: &mut App, context: &mut Context) -> Result<(), String> {
    let before = app.tool_windows().len();
    input::key(app.slint(), Key::F1);
    app.until_for("F1 help to open", Duration::from_secs(30), move |app| {
        app.tool_windows().len() > before
    });
    let window = app.tool_windows().pop().ok_or("F1 opened no tool window")?;
    if !app::layout_tool(&window, 900, 640) {
        return Err("layout_tool refused the open help window".into());
    }
    input::key(window.window(), Key::Escape);
    let watched = window.clone();
    app.until_for(
        "Escape to close the help",
        Duration::from_secs(30),
        move |_| !watched.window().is_visible(),
    );
    if app::shown(&window) {
        return Err("the closed help window still counts as shown".into());
    }
    if app::layout_tool(&window, 900, 640) {
        return Err("layout_tool laid out a closed window".into());
    }
    let (width, height, pixels) = app::render(&window);
    if width != 0 || height != 0 || !pixels.is_empty() {
        return Err(format!("render drew a closed window ({width} × {height})"));
    }
    let notes = context.outcome.notes.len();
    let shots = context.outcome.shots.len();
    app.screenshot_window(context, &window, "closed-help", "The closed help window");
    let noted = context.outcome.notes[notes..]
        .iter()
        .any(|note| note.contains("not taken: the window had closed"));
    if !noted || context.outcome.shots.len() != shots {
        return Err(format!(
            "screenshot_window of a closed window: noted {noted}, shots {} → {}",
            shots,
            context.outcome.shots.len()
        ));
    }
    Ok(())
}

fn links_counted(app: &mut App) -> Result<(), String> {
    let before = app.links_inspected.len();
    crate::panel::select_row(app, PaneId::Left, "docs")?;
    input::key(app.slint(), Key::F5);
    app.until_for("the Copy dialog", Duration::from_secs(30), |app| {
        !app.window.get_copy_draft_token().is_empty()
    });
    app.until_for(
        "the dialog's link question to be answered",
        Duration::from_secs(30),
        move |app| app.links_inspected.len() > before,
    );
    let answer = app.links_inspected[before].clone();
    let wanted = app.location(PaneId::Right);
    let result = answer
        .result
        .as_ref()
        .map(|_| ())
        .map_err(|error| format!("the question failed: {error}"));
    input::key(app.slint(), Key::Escape);
    app.until_for(
        "Escape to close the Copy dialog",
        Duration::from_secs(30),
        |app| app.window.get_copy_draft_token().is_empty(),
    );
    result?;
    if wanted.as_deref() != Some(answer.destination.as_path()) {
        return Err(format!(
            "it was asked for {}, not the other panel's {wanted:?}",
            answer.destination.display()
        ));
    }
    Ok(())
}

/// Click the command line, type [`COMMAND`] and press Enter.
fn submit_command(app: &mut App) -> Result<(), String> {
    crate::panel::wait_input(app);
    let window = &app.window;
    let position = slint::LogicalPosition::new(
        window.get_command_input_left() + window.get_command_input_width() / 2.0,
        window.get_command_input_top() + window.get_command_input_height() / 2.0,
    );
    input::click_at(app.slint(), position);
    app.until_for(
        "the command editor to take focus",
        Duration::from_secs(30),
        |app| app.window.get_command_focused(),
    );
    input::type_text(app.slint(), COMMAND);
    app.until_for("the command draft", Duration::from_secs(30), |app| {
        app.window.get_command_text().as_str() == COMMAND
    });
    input::key(app.slint(), Key::Return);
    Ok(())
}

/// Without a desktop service the command line reports "Terminal unavailable", and
/// nothing is launched.
fn no_terminal(app: &mut App) -> Result<(), String> {
    submit_command(app)?;
    app.until_for(
        "the command line's answer",
        Duration::from_secs(30),
        |app| files::shown_notice(app).is_some(),
    );
    let notice = files::take_notice(app);
    let launched = app.launches.all();
    if notice.as_ref().map(|(title, _)| title.as_str()) != Some("Terminal unavailable")
        || !launched.is_empty()
    {
        return Err(format!("notice {notice:?}, launches {launched:?}"));
    }
    Ok(())
}

/// Without the desktop stand-in the Engine has no desktop service: a followed web link is
/// refused with the reason, and nothing is launched.
fn no_url(app: &mut App) -> Result<(), String> {
    let before = app.launches.all().len();
    let answer = app.open_url(LINK)?;
    let launched = app.launches.all();
    match &answer {
        Err(status)
            if status.starts_with(&format!("Could not open {LINK}: "))
                && launched.len() == before =>
        {
            Ok(())
        }
        _ => Err(format!(
            "the viewer would say {answer:?}; launches {launched:?}"
        )),
    }
}

/// With the desktop stand-in a followed web link is recorded as the address the system's
/// handler would get, and taken as a handler takes it (the viewer would say it opened);
/// no browser starts.
fn desktop_url(root: &Path, other: &Path) -> Result<(), String> {
    let mut app = App::start(Config {
        desktop: true,
        ..Config::new(root, other)
    });
    let result = app.open_url(LINK).and_then(|answer| {
        let launched = app.launches.all();
        match (&answer, launched.as_slice()) {
            (Ok(status), [Launch::OpenUrl(url)])
                if url == LINK && *status == format!("Opened in your browser: {LINK}") =>
            {
                Ok(())
            }
            _ => Err(format!(
                "the viewer would say {answer:?}; launches {launched:?}"
            )),
        }
    });
    files::dismiss_all(&mut app);
    result.and(app.shutdown())
}

/// With the desktop stand-in the command line's terminal is recorded with the command,
/// the panel's folder and the standard mode, and not started.
fn desktop_terminal(root: &Path, other: &Path) -> Result<(), String> {
    let mut app = App::start(Config {
        desktop: true,
        ..Config::new(root, other)
    });
    let result = submit_command(&mut app).and_then(|()| {
        let folder = app.location(PaneId::Left);
        app.until_for(
            "the command line's terminal to be recorded",
            Duration::from_secs(30),
            |app| !app.launches.all().is_empty(),
        );
        // A notice, if any, says what the product made of the recorded launch.
        let notice = files::shown_notice(&app);
        match app.launches.all().as_slice() {
            [Launch::Terminal { request, .. }]
                if request.command == COMMAND
                    && Some(&request.working_directory) == folder.as_ref()
                    && request.mode == TerminalMode::Standard
                    && notice.is_none() =>
            {
                Ok(())
            }
            other => Err(format!(
                "launches {other:?}, notice {notice:?}, the left panel in {folder:?}"
            )),
        }
    });
    files::dismiss_all(&mut app);
    result.and(app.shutdown())
}

/// A Custom Command in terminal mode, handed to the launcher `Config::user_commands`
/// installs as the Engine hands it over: recorded with its environment, never started;
/// the Engine gets a process that has already ended successfully. When the platform's
/// own check refuses the command first (no terminal program on this host), nothing may
/// be recorded, and the launch itself is checked on its own.
fn user_command_terminal(context: &mut Context, folder: &Path) -> Result<(), String> {
    let launches = Arc::new(Launches::default());
    let launcher = UserCommands::new(
        Arc::new(midnight_platform::LocalFileSystem::new(None)),
        launches.clone(),
    );
    let shell = launcher.shell_flavor();
    let script = "exit 7";
    let environment = vec![(
        "ME_ACTIVE_PATH".to_owned(),
        folder.as_os_str().to_os_string(),
    )];
    let process = UserCommandProcess {
        working_directory: folder.to_path_buf(),
        invocation: UserCommandProcessInvocation::Terminal {
            shell,
            script: script.to_owned(),
            environment: environment.clone(),
        },
    };
    let cancel = Cancellation::default();
    let mut child = match launcher.launch_user_command(&process, &cancel) {
        Ok(child) => child,
        Err(refused) => {
            if !launches.all().is_empty() {
                return Err(format!(
                    "refused ({refused}) yet recorded {:?}",
                    launches.all()
                ));
            }
            context.note(format!(
                "harness_checks: the platform refused a terminal Custom Command before its \
                 launch ({refused}); the recording launch was checked on its own"
            ));
            let request = TerminalRequest {
                command: script.to_owned(),
                working_directory: folder.to_path_buf(),
                shell,
                mode: TerminalMode::Standard,
            };
            launcher
                .launch_terminal_with_environment(&request, &environment, &cancel)
                .map_err(|error| format!("the recording launch failed: {error}"))?
        }
    };
    let ended = child.try_wait().map_err(|error| error.to_string())?;
    match launches.all().as_slice() {
        [
            Launch::Terminal {
                request,
                environment: recorded,
            },
        ] if request.command == script
            && request.working_directory == folder
            && *recorded == environment
            && child.id() == 0
            && ended.is_some_and(|status| status.success()) =>
        {
            Ok(())
        }
        other => Err(format!(
            "launches {other:?}, process {} ended {ended:?}",
            child.id()
        )),
    }
}

fn gated(context: &mut Context) -> Result<(), String> {
    match fixtures::gated_dir("MIDNIGHT_ACCEPTANCE_NOT_A_VOLUME") {
        Err(error) if !error.starts_with("SKIP:") => {}
        other => return Err(format!("an unknown variable gave {other:?}")),
    }
    for variable in fixtures::GATED {
        match std::env::var_os(variable).filter(|value| !value.is_empty()) {
            None => match fixtures::gated(context, variable) {
                Err(error) if error == format!("SKIP: {variable} not set") => {}
                Err(error) => return Err(format!("{variable} unset gave {error:?}")),
                Ok(folder) => {
                    return Err(format!("{variable} unset gave {}", folder.display()));
                }
            },
            Some(value) => {
                let named = Path::new(&value).to_path_buf();
                let folder = fixtures::gated(context, variable)?;
                let made = folder.to_path_buf();
                fs::write(folder.join("probe.txt"), b"gated")
                    .map_err(|error| format!("{}: {error}", made.display()))?;
                if made.parent() != Some(named.as_path()) {
                    return Err(format!(
                        "{variable}'s folder {} is not in {}",
                        made.display(),
                        named.display()
                    ));
                }
                drop(folder);
                if made.exists() {
                    return Err(format!("{} stayed after its guard", made.display()));
                }
                context.note(format!(
                    "{variable} is set: its scenario folder came and went"
                ));
            }
        }
    }
    Ok(())
}

fn late_settings(root: &Path) -> Result<(), String> {
    let started = Instant::now();
    let mut app = App::start(Config {
        theme: PreferenceTheme::Dark,
        settings_delay: Some(app::SETTINGS_DEADLINE + Duration::from_secs(3)),
        ..Config::new(root, root)
    });
    let shown = (
        app.settings_late,
        app.settings_reads,
        app.window.get_theme_index(),
        app.window.get_desktop_status().to_string(),
    );
    let early = shown.0 && shown.1 == 0 && shown.2 == PreferenceTheme::Blue as i32;
    let result = if !early || shown.3 != SETTINGS_PENDING {
        Err(format!(
            "after {:.1} s the start showed late {}, reads {}, theme {}, status {:?}",
            started.elapsed().as_secs_f64(),
            shown.0,
            shown.1,
            shown.2,
            shown.3
        ))
    } else {
        app.until_for("the late read to apply", Duration::from_secs(30), |app| {
            app.settings_reads == 1 && app.window.get_theme_index() == PreferenceTheme::Dark as i32
        });
        if app.window.get_desktop_status() == SETTINGS_PENDING {
            Err("the read applied, and the status still says the settings are loading".into())
        } else {
            Ok(())
        }
    };
    result.and(app.shutdown())
}

fn pruning(context: &mut Context) -> Result<(), String> {
    let root = context.work.join("runs");
    fs::create_dir(&root).map_err(|error| format!("{}: {error}", root.display()))?;
    let make = |name: &str, marker: bool, summary: bool| -> Result<(), String> {
        let folder = root.join(name);
        fs::create_dir(&folder).map_err(|error| format!("{}: {error}", folder.display()))?;
        if marker {
            fs::write(folder.join(".acceptance-run"), b"").map_err(|error| error.to_string())?;
        }
        if summary {
            fs::write(folder.join("summary.json"), b"{}").map_err(|error| error.to_string())?;
        }
        Ok(())
    };
    // Six finished runs, an unfinished one from long ago, one still going (now), a
    // folder named like a run without the marker, and another folder.
    for day in 1..=6 {
        make(&format!("2020010{day}-000000"), true, true)?;
    }
    make("20190101-000000", true, false)?;
    let going = report::run_id();
    make(&format!("{going}-9"), true, false)?;
    make("20180101-000000", false, true)?;
    make("notes", false, false)?;
    let current = report::create_run(&root)?;
    report::prune_old_runs(&current, false);
    let mut left: Vec<String> = fs::read_dir(&root)
        .map_err(|error| format!("{}: {error}", root.display()))?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    let current_name = current
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut wanted: Vec<String> = [
        "20180101-000000",
        "20200103-000000",
        "20200104-000000",
        "20200105-000000",
        "20200106-000000",
        "notes",
    ]
    .iter()
    .map(|name| (*name).to_owned())
    .chain([format!("{going}-9"), current_name])
    .collect();
    wanted.sort();
    if left == wanted {
        Ok(())
    } else {
        Err(format!("left {left:?}, expected {wanted:?}"))
    }
}

/// `App::start` returns once both panels show the folders it was given: a panel that has
/// not started its first listing is not loading either, which once let a staged child
/// start with no folder at all.
fn start_folders(app: &App, left: &Path, right: &Path) -> Result<(), String> {
    let shown = [app.location(PaneId::Left), app.location(PaneId::Right)];
    let same = |shown: &Option<std::path::PathBuf>, wanted: &Path| {
        shown.as_ref().is_some_and(|path| {
            std::fs::canonicalize(path).ok() == std::fs::canonicalize(wanted).ok()
        })
    };
    if same(&shown[0], left) && same(&shown[1], right) {
        Ok(())
    } else {
        Err(format!(
            "the panels show {shown:?} when App::start returned"
        ))
    }
}
