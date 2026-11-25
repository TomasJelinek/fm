//! Journey: a session that never touches the mouse. Every step is a key in the window
//! that has the keyboard (the panels, then the Find window), as a keyboard-only user
//! works; no pointer event is sent.
//!
//! 1. At startup the list takes keys: End and Home move the cursor.
//! 2. F7 creates a folder (cursor on it), Enter opens it, Shift+F4 creates a file there,
//!    Backspace returns with the cursor on the folder.
//! 3. Ctrl+= selects `*.txt`: exactly the two text files.
//! 4. F5 and Enter copy them to the other panel; the marks clear. F5 again onto the
//!    copies, now holding other text: Tab reaches the name field and Copy as for one
//!    question, Escape leaves the other for later (the job waits); F10, Down to the
//!    Application category, Tab and Space open Jobs; Up and Down select the waiting job,
//!    Tab and Space press Jobs' Resolve, and Skip answers the question it brings back.
//! 5. Tab to the other panel; F2 renames a copy (the stem preselected, so typing
//!    replaces it); Shift+F8 and Enter delete the other copy. Then in `links/`, F5 of a
//!    link to a folder (a directory junction on Windows): Tab to the dialog's Links box,
//!    Up to "Copy the files links point to", Enter from the box; the folder's tree
//!    arrives, and the link stays a link.
//! 6. Tab back; Ctrl+G opens a typed path; Alt+Left returns.
//! 7. Alt+F7: type a mask, Enter searches, Tab reaches the results, Enter goes to the
//!    file, Escape closes Find.
//! 8. Ctrl+F12 filters to `*.txt` and back to `*` with Enter in the expression field.
//! 9. Ctrl+U swaps the panels and back: they change places, each keeping its folder.
use super::journeys_support::{self as journey, Journey, wait};
use crate::{
    Context,
    app::{self, App, Config},
    files_support as files, input, inspect, panel,
    scenarios::find_support as find,
    tools_support as tools,
};
use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_core::{CopyOperationState, OperationState, PaneId};
use midnight_ui::PanelChoices;
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter},
};
use std::{path::Path, time::Duration};

const LEFT: PaneId = PaneId::Left;
const RIGHT: PaneId = PaneId::Right;
const STEP: Duration = Duration::from_secs(30);
/// What the other panel's copies hold before step 4b copies onto them.
const OLD_ALPHA: &str = "old alpha\n";
const OLD_BETA: &str = "old beta\n";
/// The name step 4b types for alpha.txt's copy under another name.
const KEPT_ALPHA: &str = "alpha-kept.txt";
/// Tab stops a keyboard user goes through to reach a control, at most.
const TABS: usize = 120;
/// Step 5b's link to a folder, in `links/`, and what the folder holds.
const LINK: &str = "to-target";
const LINKED: &str = "linked\n";

pub fn scenario() -> crate::Scenario {
    crate::Scenario {
        name: "journeys_keyboard_only",
        isolation: crate::Isolation::Parallel,
        area: "journeys",
        summary: "A session by keys only: F7, Enter, Shift+F4, Backspace, Ctrl+= mask, F5, conflicts answered by keys (Copy as, Escape to decide later, Jobs by F10, Up and Down to the waiting job, Resolve, Skip), Tab, F2, Shift+F8, F5 of a link with Tab to Links, Up and Enter (the files copied), Ctrl+G, Alt+Left, Alt+F7 with Tab into the results and Enter to go to the file, Ctrl+F12, Ctrl+U; every effect checked on disk and on screen.",
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
        ("links/target/inner.txt", LINKED),
    ] {
        journey::write(&home.join(name), text)?;
    }
    // A link to the folder beside it: relative on Unix, a directory junction on Windows
    // (it needs no privilege).
    let link = home.join("links").join(LINK);
    #[cfg(unix)]
    std::os::unix::fs::symlink("target", &link).map_err(|error| files::io_error(&link, error))?;
    #[cfg(windows)]
    crate::fixtures::junction(&link, &home.join("links").join("target"))?;
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

/// Wait until the panels are drawn `swapped` (Ctrl+U moves the panels, not their
/// folders: `panels-swapped`, the left pane drawn right of the right one) while the left
/// pane still shows `left` and the right one `right`.
fn panels_swapped(app: &mut App, swapped: bool, left: &Path, right: &Path) -> Result<(), String> {
    let (left, right) = (left.to_path_buf(), right.to_path_buf());
    wait(
        app,
        if swapped {
            "the panels to change places"
        } else {
            "the panels to change places back"
        },
        STEP,
        move |app| {
            app.window.get_panels_swapped() == swapped
                && (app.window.get_left_pane_left() > app.window.get_right_pane_left()) == swapped
                && app.location(LEFT).as_ref() == Some(&left)
                && app.location(RIGHT).as_ref() == Some(&right)
        },
    )
    .map_err(|error| {
        format!(
            "{error}; panels-swapped {}, the left pane at x {}, the right pane at x {}",
            app.window.get_panels_swapped(),
            app.window.get_left_pane_left(),
            app.window.get_right_pane_left()
        )
    })
}

/// Ctrl+= in the focused left list, `*.txt`, Enter: exactly the two text files marked.
fn mark_text_files(app: &mut App) -> Result<(), String> {
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
        .map_err(|error| format!("Ctrl+= *.txt: {error}"))
}

/// Tab in `window` until the keyboard focus is in the control `role`/`label`, as a
/// keyboard user reaches it.
fn tab_to(
    app: &mut App,
    window: &tools::Tool,
    role: AccessibleRole,
    label: &str,
) -> Result<(), String> {
    for _ in 0..TABS {
        if journey::focus_inside(window.window(), role, label) {
            return Ok(());
        }
        input::key(window.window(), Key::Tab);
        app.settle();
    }
    Err(format!("{TABS} Tabs never reached the {role:?} {label:?}"))
}

/// Tab to the button `label` of `window` and press Space on it.
fn press_by_keys(app: &mut App, window: &tools::Tool, label: &str) -> Result<(), String> {
    tab_to(app, window, AccessibleRole::Button, label)?;
    input::key(window.window(), " ");
    app.settle();
    Ok(())
}

/// Step 4b: F5 of the two text files onto their copies, which hold other text now. By
/// keys only: alpha.txt's question is answered with Copy as under a typed name;
/// beta.txt's with Escape, which leaves it for later (the job waits); F10 and Tab reach
/// Jobs, Up and Down select the waiting job, Jobs' Resolve brings the question back,
/// Skip answers it. The copy under the new name holds alpha's text, and both copies keep
/// their text.
fn conflicts_by_keys(app: &mut App, journey: &mut Journey, other: &Path) -> Result<(), String> {
    journey::write(&other.join("alpha.txt"), OLD_ALPHA)?;
    journey::write(&other.join("beta.txt"), OLD_BETA)?;
    journey::key_focus(app, LEFT)?;
    mark_text_files(app)?;
    let start = app.copies.len();
    key(app, &[], Key::F5);
    wait(app, "the Copy dialog", STEP, |app| {
        !app.window.get_copy_draft_token().is_empty()
    })?;
    key(app, &[], Key::Return);
    wait(app, "the copy to start", STEP, move |app| {
        app.copies.len() > start
    })?;
    let mut answered = Vec::new();
    let mut deferred = false;
    let mut resolved = false;
    loop {
        wait(app, "the copy to ask, wait or end", STEP, move |app| {
            files::decision_window(app).is_some()
                || files::job(app, start).is_some_and(|job| {
                    job.state.is_terminal() || job.state == CopyOperationState::WaitingForDecision
                })
        })?;
        if let Some(window) = files::decision_window(app) {
            app::layout_tool(&window, 820, 560);
            app.settle();
            let texts = inspect::visible_texts(window.window());
            if texts.iter().any(|text| text.contains("alpha.txt")) {
                tab_to(
                    app,
                    &window,
                    AccessibleRole::TextInput,
                    "Copy under a different name",
                )?;
                input::replace_text(window.window(), KEPT_ALPHA);
                press_by_keys(app, &window, "Copy as")?;
                answered.push("alpha.txt: Copy as");
            } else if texts.iter().any(|text| text.contains("beta.txt")) && !deferred {
                input::key(window.window(), Key::Escape);
                deferred = true;
                answered.push("beta.txt: Escape");
            } else if texts.iter().any(|text| text.contains("beta.txt")) {
                press_by_keys(app, &window, "Skip")?;
                answered.push("beta.txt: Skip");
            } else {
                return Err(format!("an unexpected question: {texts:?}"));
            }
            let watched = window.clone();
            wait(app, "the answer to be taken", STEP, move |_| {
                !watched.window().is_visible()
            })?;
            continue;
        }
        match files::job(app, start).map(|job| job.state) {
            Some(CopyOperationState::WaitingForDecision) if !resolved => {
                resolve_from_jobs(app)?;
                resolved = true;
                wait(app, "Resolve to bring the question back", STEP, |app| {
                    files::decision_window(app).is_some()
                })?;
            }
            Some(state) if state.is_terminal() => break,
            other => return Err(format!("the copy is {other:?} after {answered:?}")),
        }
    }
    let done = files::job(app, start).ok_or("the copy left no report")?;
    journey.check(
        "every question was answered by keys, and Escape left one for later",
        if deferred && resolved && answered.len() == 3 {
            Ok(())
        } else {
            Err(format!(
                "answers {answered:?}, deferred {deferred}, resolved {resolved}"
            ))
        },
    );
    journey.check(
        "the copy ends with one skip, alpha's text under the typed name, both copies kept",
        if done.state == CopyOperationState::Completed && done.skipped == 1 && done.failed == 0 {
            journey::holds(&other.join(KEPT_ALPHA), "alpha\n")
                .and_then(|()| journey::holds(&other.join("alpha.txt"), OLD_ALPHA))
                .and_then(|()| journey::holds(&other.join("beta.txt"), OLD_BETA))
        } else {
            Err(format!(
                "{:?}, {} skipped, {} failed",
                done.state, done.skipped, done.failed
            ))
        },
    );
    close_job_windows(app)?;
    journey.step(format!(
        "F5 onto the copies by keys: {answered:?}; Jobs by F10, Down and Tab, the waiting job \
         selected with Up and Down, Resolve by Tab and Space"
    ));
    Ok(())
}

/// F10, Down to the Application category, Tab to Jobs, Space; the waiting job's row
/// selected with Up and Down; Tab to Jobs' Resolve, Space.
fn resolve_from_jobs(app: &mut App) -> Result<(), String> {
    panel::wait_input(app);
    let before = journey::titled(app, "Jobs").len();
    key(app, &[], Key::F10);
    wait(app, "F10 to open the menu", STEP, |app| {
        inspect::find(app.slint(), AccessibleRole::Combobox, "Menu category").is_some()
    })?;
    // The menu opens with its category picker focused: Down to "Application", whose few
    // actions (Jobs among them) are all in view.
    let category =
        |app: &App| crate::options_support::value(app, AccessibleRole::Combobox, "Menu category");
    for _ in 0..8 {
        if category(app).as_deref() == Some("Application") {
            break;
        }
        key(app, &[], Key::DownArrow);
    }
    if category(app).as_deref() != Some("Application") {
        return Err(format!(
            "Down in the menu's category picker shows {:?}, not Application",
            category(app)
        ));
    }
    let main = app.main.clone();
    press_by_keys(app, &main, "Jobs")?;
    let jobs = journey::new_window(app, "Jobs", before)?;
    let rows = journey::job_rows(&jobs);
    let target = rows
        .iter()
        .position(|(_, texts)| {
            texts
                .iter()
                .any(|text| text.to_lowercase().contains("decision"))
        })
        .ok_or_else(|| format!("Jobs shows no job waiting for a decision: {rows:?}"))?;
    for _ in 0..rows.len() {
        let selected = selected_job(&jobs);
        match selected {
            Some(row) if row == target => break,
            Some(row) if row < target => input::key(jobs.window(), Key::DownArrow),
            _ => input::key(jobs.window(), Key::UpArrow),
        }
        app.settle();
    }
    if selected_job(&jobs) != Some(target) {
        return Err(format!(
            "Up and Down left row {:?} selected, not the waiting job's row {target}",
            selected_job(&jobs)
        ));
    }
    // Jobs offers Resolve for the selected job while it waits for a decision.
    press_by_keys(app, &jobs, "Resolve")
}

/// The row of Jobs that is selected (top to bottom), if any.
fn selected_job(jobs: &tools::Tool) -> Option<usize> {
    let _ = app::render(jobs);
    let mut rows: Vec<(f32, bool)> = inspect::controls(jobs.window(), AccessibleRole::ListItem)
        .into_iter()
        .map(|(_, item)| {
            (
                inspect::rect(&item).1,
                item.accessible_string_property(AccessibleStringProperty::Checked)
                    .is_some_and(|value| value == "true"),
            )
        })
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.iter().position(|(_, checked)| *checked)
}

/// Close Jobs and the job windows that stayed open, with Escape.
fn close_job_windows(app: &mut App) -> Result<(), String> {
    for window in app.tool_windows() {
        let title = tools::title(&window);
        journey::close_window(app, &window, &format!("the {title} window"))?;
    }
    Ok(())
}

/// Step 5b: in `links/`, F5 of [`LINK`], a link to the folder `target` beside it; Tab to
/// the dialog's Links box, Up to "Copy the files links point to" (the dialog proposes
/// "Keep links as links" on this fixed local folder), Enter from the box. The folder's
/// tree arrives in the other panel's folder, not a link; the link itself is unchanged.
fn links_by_keys(
    app: &mut App,
    context: &mut Context,
    journey: &mut Journey,
    home: &Path,
    other: &Path,
) -> Result<(), String> {
    let folder = home.join("links");
    journey::key_focus(app, LEFT)?;
    journey::key_cursor(app, LEFT, "links")?;
    key(app, &[], Key::Return);
    location(app, LEFT, &folder)?;
    journey::key_cursor(app, LEFT, LINK)?;
    let run = files::transfer(
        app,
        context,
        files::TransferSpec {
            links: Some(files::LinkChoice::CopyFiles),
            link_input: files::LinkInput::Keyboard,
            ..files::TransferSpec::copy()
        },
    )?;
    let shown = run.links.as_ref().and_then(files::LinkView::chosen);
    let copy = other.join(LINK);
    journey.check(
        "Tab to Links, Up and Enter copy the files a link points to",
        if shown == Some(files::LinkChoice::CopyFiles)
            && run.progress.state == CopyOperationState::Completed
            && run.progress.failed == 0
        {
            match files::facts(&copy).map(|facts| facts.content) {
                Ok(files::Content::Folder) => journey::holds(&copy.join("inner.txt"), LINKED),
                found => Err(format!("{LINK} arrived as {found:?}")),
            }
        } else {
            Err(format!(
                "Links {shown:?}; {:?}, {} failed",
                run.progress.state, run.progress.failed
            ))
        },
    );
    journey.check(
        "the link itself is unchanged",
        match files::facts(&folder.join(LINK)).map(|facts| facts.content) {
            Ok(files::Content::Link(_)) => Ok(()),
            found => Err(format!("the source is now {found:?}")),
        },
    );
    panel::wait_input(app);
    key(app, &[], Key::Backspace);
    location(app, LEFT, home)?;
    journey.step(format!(
        "links/: F5 of {LINK}, Tab to Links, Up, Enter: Links {shown:?}, the folder's tree \
         copied"
    ));
    Ok(())
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
    mark_text_files(app)?;
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

    // 4b. F5 onto the copies, every question answered by keys.
    conflicts_by_keys(app, journey, other)?;

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
        journey::holds(&other.join("beta-renamed.txt"), OLD_BETA)
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
        "the other panel lists the renamed copy and the copy made under another name",
        journey::wait_shown(app, RIGHT, &[KEPT_ALPHA, "beta-renamed.txt"], STEP),
    );
    journey.step("Tab, F2 beta-renamed, Shift+F8 alpha.txt in the other panel");

    // 5b. F5 of a link, Tab to Links, Up, Enter.
    links_by_keys(app, context, journey, home, other)?;

    // 6. Ctrl+G and Alt+Left.
    journey::key_focus(app, LEFT)?;
    key(app, &[Key::Control], "g");
    wait(app, "Ctrl+G to focus the path editor", STEP, |app| {
        journey::focus_inside(app.slint(), AccessibleRole::TextInput, "Folder path")
    })?;
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
        journey::wait_shown(
            app,
            LEFT,
            &["alpha.txt", "beta.txt", "links", "made", "sub"],
            STEP,
        ),
    );
    filter(app, "*")?;
    journey.check(
        "Ctrl+F12 * lists everything again",
        journey::wait_shown(
            app,
            LEFT,
            &["alpha.txt", "beta.txt", "gamma.log", "links", "made", "sub"],
            STEP,
        ),
    );
    journey.step("Ctrl+F12 *.txt then * with Enter in the expression");

    // 9. Ctrl+U and back: the panels change places, each with its folder (as
    //    files_navigation checks).
    key(app, &[Key::Control], "u");
    journey.check(
        "Ctrl+U swaps the panels",
        panels_swapped(app, true, home, other),
    );
    key(app, &[Key::Control], "u");
    journey.check(
        "Ctrl+U swaps them back",
        panels_swapped(app, false, home, other),
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
