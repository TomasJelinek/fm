//! Journey: a session that never touches the mouse. Every step is a key in the window
//! that has the keyboard (the panels, then the Find window), as a keyboard-only user
//! works; no pointer event is sent.
//!
//! 1. At startup the list takes keys: End and Home move the cursor.
//! 2. F7 creates a folder (cursor on it), Enter opens it, Shift+F4 creates a file there,
//!    Backspace returns with the cursor on the folder.
//! 3. Ctrl+= selects `*.txt`: exactly the two text files.
//! 4. F5 and Enter copy them to the other panel; the marks clear.
//! 5. Tab to the other panel; F2 renames a copy (the stem preselected, so typing
//!    replaces it); Shift+F8 and Enter delete the other copy.
//! 6. Tab back; Ctrl+G opens a typed path; Alt+Left returns.
//! 7. Alt+F7: type a mask, Enter searches, Tab reaches the results, Enter goes to the
//!    file, Escape closes Find.
//! 8. Ctrl+F12 filters to `*.txt` and back to `*` with Enter in the expression field.
//! 9. Ctrl+U swaps the panels and back.
use super::journeys_support::{self as journey, Journey, wait};
use crate::{
    Context,
    app::{self, App, Config},
    files_support as files, input, panel,
    scenarios::find_support as find,
    tools_support as tools,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{OperationState, PaneId};
use midnight_ui::PanelChoices;
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter},
};
use std::{path::Path, time::Duration};

const LEFT: PaneId = PaneId::Left;
const RIGHT: PaneId = PaneId::Right;
const STEP: Duration = Duration::from_secs(30);

pub fn scenario() -> crate::Scenario {
    crate::Scenario {
        name: "journeys_keyboard_only",
        area: "journeys",
        summary: "A session by keys only: F7, Enter, Shift+F4, Backspace, Ctrl+= mask, F5, Tab, F2, Shift+F8, Ctrl+G, Alt+Left, Alt+F7 with Tab into the results and Enter to go to the file, Ctrl+F12, Ctrl+U; every effect checked on disk and on screen.",
        run: keyboard_only,
        timeout: Duration::from_secs(8 * 60),
    }
}

pub fn keyboard_only(context: &mut Context) -> Result<(), String> {
    let home = files::scratch(context, "kb")?;
    let other = files::scratch(context, "kb-out")?;
    for (name, text) in [
        ("alpha.txt", "alpha\n"),
        ("beta.txt", "beta\n"),
        ("gamma.log", "gamma\n"),
        ("sub/inner.txt", "inner\n"),
    ] {
        journey::write(&home.join(name), text)?;
    }
    let mut app = App::start(Config::new(&home, &other));
    let mut journey = Journey::default();
    let ended = steps(&mut app, context, &mut journey, &home, &other);
    find::reset(&mut app);
    files::dismiss_all(&mut app);
    let ended = ended.and(app.shutdown());
    journey.finish(context, ended)
}

fn key(app: &mut App, modifiers: &[Key], text: impl Into<slint::SharedString>) {
    input::chord(app.slint(), modifiers, text);
    app.settle();
}

/// Wait for the mutation reported after `start` to complete.
fn completed(app: &mut App, start: usize, what: &str) -> Result<(), String> {
    wait(app, what, STEP, move |app| {
        app.operations[start..]
            .iter()
            .any(|progress| progress.state.is_terminal())
    })?;
    let progress = files::wait_operation(app, start);
    if progress.state == OperationState::Completed {
        Ok(())
    } else {
        Err(format!(
            "{what} ended {:?}: {:?}",
            progress.state, progress.errors
        ))
    }
}

fn cursor_on(app: &mut App, pane: PaneId, name: &str) -> Result<(), String> {
    let wanted = name.to_owned();
    wait(
        app,
        &format!("the {pane:?} cursor on {name}"),
        STEP,
        move |app| files::cursor_name(app, pane).as_deref() == Some(wanted.as_str()),
    )
    .map_err(|error| {
        format!(
            "{error}; the cursor is on {:?}",
            files::cursor_name(app, pane)
        )
    })
}

fn location(app: &mut App, pane: PaneId, folder: &Path) -> Result<(), String> {
    let wanted = folder.to_path_buf();
    wait(
        app,
        &format!("the {pane:?} panel to show {}", folder.display()),
        STEP,
        move |app| app.location(pane).as_ref() == Some(&wanted) && !app.loading(pane),
    )
}

/// Ctrl+F12, the expression replaced by typing, Enter.
fn filter(app: &mut App, text: &str) -> Result<(), String> {
    panel::wait_input(app);
    key(app, &[Key::Control], Key::F12);
    wait(app, "the Panel filter dialog", STEP, |app| {
        app.window.get_panel_filter_view().open
    })?;
    input::replace_text(app.slint(), text);
    key(app, &[], Key::Return);
    wait(app, "Enter to apply the filter", STEP, |app| {
        !app.window.get_panel_filter_view().open
    })
}

fn steps(
    app: &mut App,
    context: &mut Context,
    journey: &mut Journey,
    home: &Path,
    other: &Path,
) -> Result<(), String> {
    // 1. The list takes keys at startup.
    let ends = |app: &mut App| {
        key(app, &[], Key::End);
        let end = app.cursor(LEFT);
        key(app, &[], Key::Home);
        let start = app.cursor(LEFT);
        let rows = app.names(LEFT).len() as i32;
        let focused = app.window.get_focused_pane();
        if focused == 0 && end == rows - 1 && start == 0 {
            Ok(())
        } else {
            Err(format!(
                "focused pane {focused}, End put the cursor on row {end} of {rows}, Home on \
                 {start}"
            ))
        }
    };
    let ready = ends(app);
    if !journey.check("at startup the left list takes keys", ready) {
        // A keyboard user would press Tab to reach the list.
        journey::key_focus(app, LEFT)?;
        ends(app).map_err(|error| format!("even after Tab: {error}"))?;
    }
    journey.step("at startup End and Home move the left panel's cursor");

    // 2. F7, Enter, Shift+F4, Backspace.
    let start = files::named_operation(app, &[], Key::F7, "made")?;
    completed(app, start, "F7 made")?;
    journey.check(
        "F7 creates the folder",
        if home.join("made").is_dir() {
            Ok(())
        } else {
            Err("made/ is not on disk".into())
        },
    );
    cursor_on(app, LEFT, "made")?;
    key(app, &[], Key::Return);
    location(app, LEFT, &home.join("made"))?;
    let start = files::named_operation(app, &[Key::Shift], Key::F4, "note.txt")?;
    completed(app, start, "Shift+F4 note.txt")?;
    journey.check(
        "Shift+F4 creates the empty file inside the opened folder",
        journey::holds(&home.join("made/note.txt"), ""),
    );
    journey.check(
        "the new file has the cursor",
        cursor_on(app, LEFT, "note.txt"),
    );
    panel::wait_input(app);
    key(app, &[], Key::Backspace);
    location(app, LEFT, home)?;
    journey.check(
        "Backspace returns with the cursor on the folder",
        cursor_on(app, LEFT, "made"),
    );
    journey.step("F7 made, Enter, Shift+F4 note.txt, Backspace: on disk, cursor followed");

    // 3. Ctrl+= *.txt.
    panel::wait_input(app);
    key(app, &[Key::Control], "=");
    let mask = |app: &App| {
        !app.window
            .global::<PanelChoices>()
            .get_mask_token()
            .is_empty()
    };
    if wait(
        app,
        "Ctrl+= to open Select files",
        Duration::from_secs(3),
        mask,
    )
    .is_err()
    {
        // The native key hook delivers Ctrl+= as the physical Equal key.
        app.window
            .invoke_physical_shortcut("Equal".into(), true, false, false, false);
        wait(app, "Ctrl+= (physical) to open Select files", STEP, mask)?;
    }
    input::replace_text(app.slint(), "*.txt");
    key(app, &[], Key::Return);
    wait(app, "Enter to select by mask", STEP, move |app| !mask(app))?;
    journey::same(&journey::marked(app, LEFT), &["alpha.txt", "beta.txt"])
        .map_err(|error| format!("Ctrl+= *.txt: {error}"))?;
    journey.step("Ctrl+= *.txt Enter marks exactly the two text files");

    // 4. F5, Enter.
    let start = files::start_transfer(app, context, files::TransferSpec::copy())?;
    let done = files::wait_job(app, start);
    journey.check(
        "F5, Enter copies the marked files",
        if done.state == midnight_core::CopyOperationState::Completed && done.failed == 0 {
            Ok(())
        } else {
            Err(format!("{:?}, {} failed", done.state, done.failed))
        },
    );
    for (name, text) in [("alpha.txt", "alpha\n"), ("beta.txt", "beta\n")] {
        journey.check(
            &format!("{name} copied"),
            journey::holds(&other.join(name), text),
        );
    }
    journey.check(
        "the other panel lists the copies",
        journey::wait_shown(app, RIGHT, &["alpha.txt", "beta.txt"], STEP),
    );
    journey.check(
        "the completed copies are no longer marked",
        journey::same(&journey::marked(app, LEFT), &[]),
    );
    journey.step("F5, Enter copied both to the other panel; marks cleared");

    // 5. Tab, F2, Shift+F8.
    journey::key_focus(app, RIGHT)?;
    journey::key_cursor(app, RIGHT, "beta.txt")?;
    let start = app.operations.len();
    key(app, &[], Key::F2);
    wait(app, "the Rename dialog", STEP, |app| {
        !app.window.get_operation_token().is_empty()
    })?;
    input::type_text(app.slint(), "beta-renamed");
    key(app, &[], Key::Return);
    completed(app, start, "F2 beta-renamed")?;
    journey.check(
        "F2 with the stem preselected renames to the typed stem",
        journey::holds(&other.join("beta-renamed.txt"), "beta\n")
            .and_then(|()| journey::absent(&other.join("beta.txt"))),
    );
    journey.check(
        "the renamed file has the cursor",
        cursor_on(app, RIGHT, "beta-renamed.txt"),
    );
    journey::key_cursor(app, RIGHT, "alpha.txt")?;
    let start = app.operations.len();
    key(app, &[Key::Shift], Key::F8);
    wait(app, "the permanent delete confirmation", STEP, |app| {
        !app.window.get_operation_token().is_empty() && app.window.get_operation_dangerous()
    })?;
    key(app, &[], Key::Return);
    completed(app, start, "Shift+F8 alpha.txt")?;
    journey.check(
        "Shift+F8, Enter deletes the file",
        journey::absent(&other.join("alpha.txt")),
    );
    journey.check(
        "the other panel lists only the renamed copy",
        journey::wait_shown(app, RIGHT, &["beta-renamed.txt"], STEP),
    );
    journey.step("Tab, F2 beta-renamed, Shift+F8 alpha.txt in the other panel");

    // 6. Ctrl+G and Alt+Left.
    journey::key_focus(app, LEFT)?;
    key(app, &[Key::Control], "g");
    app.idle(Duration::from_millis(100));
    input::replace_text(app.slint(), &home.join("sub").display().to_string());
    key(app, &[], Key::Return);
    location(app, LEFT, &home.join("sub"))?;
    journey::wait_shown(app, LEFT, &["inner.txt"], STEP)?;
    panel::wait_input(app);
    key(app, &[Key::Alt], Key::LeftArrow);
    location(app, LEFT, home)?;
    journey.step("Ctrl+G to a typed path, Alt+Left back");

    // 7. Find by keys.
    let before = app.tool_windows().len();
    key(app, &[Key::Alt], Key::F7);
    wait(app, "the Find window", STEP, move |app| {
        app.tool_windows().len() > before
    })?;
    let window = app.tool_windows().pop().ok_or("no Find window")?;
    app::layout_tool(&window, find::SIZE.0, find::SIZE.1);
    app.settle();
    input::replace_text(window.window(), "*.log");
    input::key(window.window(), Key::Return);
    let watched = window.clone();
    wait(app, "Enter in the masks to search", STEP, move |_| {
        find::terminal(&find::status(&watched))
    })?;
    let rows: Vec<String> = find::visible_rows(&window)
        .into_values()
        .map(|row| row.name)
        .collect();
    journey.check(
        "Enter in the masks searches *.log",
        if rows == ["gamma.log"] {
            Ok(())
        } else {
            Err(format!("results {rows:?} ({})", find::status(&window)))
        },
    );
    let mut tabs = 0;
    while !journey::focus_inside(window.window(), AccessibleRole::List, "Find results") && tabs < 40
    {
        input::key(window.window(), Key::Tab);
        app.settle();
        tabs += 1;
    }
    let reached = journey::focus_inside(window.window(), AccessibleRole::List, "Find results");
    journey.check(
        "Tab reaches the results list",
        if reached {
            Ok(())
        } else {
            Err(format!(
                "40 Tabs did not reach it; focus on {:?}",
                focused_label(&window)
            ))
        },
    );
    if reached {
        input::key(window.window(), Key::Home);
        input::key(window.window(), Key::Return);
        let went = location(app, LEFT, home).and_then(|()| cursor_on(app, LEFT, "gamma.log"));
        journey.check("Enter on the result goes to the file", went);
    }
    input::key(window.window(), Key::Escape);
    let watched = window.clone();
    journey.check(
        "Escape closes Find",
        wait(app, "Escape to close Find", STEP, move |_| {
            !watched.window().is_visible()
        }),
    );
    app.slint()
        .dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(true));
    journey.step(format!(
        "Alt+F7 *.log Enter, {tabs} Tab(s) to the results, Enter went to gamma.log, Escape"
    ));

    // 8. Ctrl+F12 by keys.
    panel::wait_input(app);
    filter(app, "*.txt")?;
    journey.check(
        "Ctrl+F12 *.txt lists the folders and the text files",
        journey::wait_shown(app, LEFT, &["alpha.txt", "beta.txt", "made", "sub"], STEP),
    );
    filter(app, "*")?;
    journey.check(
        "Ctrl+F12 * lists everything again",
        journey::wait_shown(
            app,
            LEFT,
            &["alpha.txt", "beta.txt", "gamma.log", "made", "sub"],
            STEP,
        ),
    );
    journey.step("Ctrl+F12 *.txt then * with Enter in the expression");

    // 9. Ctrl+U and back.
    key(app, &[Key::Control], "u");
    journey.check(
        "Ctrl+U swaps the panels",
        location(app, LEFT, other).and_then(|()| location(app, RIGHT, home)),
    );
    key(app, &[Key::Control], "u");
    journey.check(
        "Ctrl+U swaps them back",
        location(app, LEFT, home).and_then(|()| location(app, RIGHT, other)),
    );
    journey.check(
        "no pointer event was sent",
        if app.last_click.is_none() {
            Ok(())
        } else {
            Err("a row was clicked".into())
        },
    );
    journey.check(
        "nothing is left open",
        if files::main_idle(app) && tools::shown_notice(app).is_none() {
            Ok(())
        } else {
            Err(format!(
                "windows {:?}, notice {:?}",
                tools::tool_titles(app),
                tools::shown_notice(app)
            ))
        },
    );
    app.screenshot(context, "done", "After the keyboard-only session");
    journey.step("Ctrl+U swapped the panels and back; nothing left open");
    Ok(())
}

/// The accessible label (or role) of the focused item of a window, for messages.
fn focused_label(window: &tools::Tool) -> String {
    journey::focused(window.window()).map_or_else(
        || "nothing".to_owned(),
        |item| {
            crate::inspect::label(&item).unwrap_or_else(|| format!("{:?}", item.accessible_role()))
        },
    )
}
