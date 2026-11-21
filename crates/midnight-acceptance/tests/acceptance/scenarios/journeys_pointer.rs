//! Journey: a session by mouse only. Every step is a click, a double click, a drag or
//! a rubber band; no key event is sent. It runs in a private child process
//! (`viewers_stage::run_private`) whose Trash is a private folder.
//!
//! 1. A double click opens a folder; a double click on `..` returns with the cursor on
//!    it.
//! 2. A rubber band from the gutter marks two files.
//! 3. The footer's F5 cell and the dialog's Copy button copy them; the name conflict is
//!    answered with Replace · keep original. The kept original is a warning, so the
//!    job's window stays with that outcome; its Close button closes it.
//! 4. A file dragged onto a folder row of the other panel is moved into that folder
//!    (one volume: Move), confirmed with the dialog's Move button if one opens.
//! 5. The Size header sorts by size and, clicked again, reverses; the Name header
//!    restores the order.
//! 6. The toolbar's Jobs button lists both jobs (the copy completed with warnings, the
//!    move completed); its Close button closes it.
//! 7. A click on a row, the footer's F8 cell and the confirmation's button move a file
//!    to the (private) Trash. Windows has no private Recycle Bin: there the
//!    confirmation is cancelled and the file must stay.
//! 8. A drop within 400 ms of the press is cancelled: nothing is copied or moved
//!    (pending until announced).
use super::{
    journeys_support::{self as journey, Journey, wait},
    panels_support as panels,
};
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as files, Choice},
    input, inspect, panel, viewers_stage,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{CopyOperationState, OperationState, PaneId};
use slint::{
    LogicalPosition,
    platform::{PointerEventButton, WindowAdapter, WindowEvent},
};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const LEFT: PaneId = PaneId::Left;
const RIGHT: PaneId = PaneId::Right;
const STEP: Duration = Duration::from_secs(30);
pub const TIMEOUT: Duration = Duration::from_secs(10 * 60);
const QUICK_DROP: &str = "a drop within 400 ms of the press is cancelled";
/// Drag feedback states (`MainWindow.drag-feedback-state`).
const MOVE: i32 = 4;
const LONG: &str = "one, the longest of the three files\n";
/// The Copy window's outcome line for the original kept by Replace · keep original.
const KEPT: &str = "1 file was kept in a recovery folder; Details shows where.";

pub fn scenario() -> crate::Scenario {
    crate::Scenario {
        name: "journeys_pointer_only",
        area: "journeys",
        summary: "A session by mouse only: double clicks into and out of a folder, a rubber band, the footer's F5 and the dialog's Copy with Replace, a drag onto a folder row (Move), header sorting, Jobs and its Close, F8 to a private Trash by clicks, and a drop within 400 ms cancelled (pending).",
        run: pointer_only,
        timeout: TIMEOUT,
    }
}

pub fn pointer_only(context: &mut Context) -> Result<(), String> {
    viewers_stage::run_private(context, TIMEOUT, run)
}

fn run(context: &mut Context) -> Result<(), String> {
    let staged = context.run_dir.parent().and_then(Path::parent);
    // Windows keeps one Recycle Bin per volume and account, which no variable makes
    // private: there the Trash confirmation is cancelled instead (step 7).
    let data = if cfg!(unix) {
        Some(
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .filter(|data| staged.is_some_and(|staged| data.starts_with(staged)))
                .ok_or(
                    "this journey runs only in its private process, with a private XDG_DATA_HOME",
                )?,
        )
    } else {
        None
    };
    let home = files::scratch(context, "pt")?;
    let other = files::scratch(context, "pt-out")?;
    for (name, text) in [
        ("a-one.txt", LONG),
        ("b-two.txt", "two new\n"),
        ("c-three.txt", "three\n"),
        ("folder/x.txt", "x\n"),
    ] {
        journey::write(&home.join(name), text)?;
    }
    journey::write(&other.join("b-two.txt"), "two old\n")?;
    std::fs::create_dir(other.join("into")).map_err(|error| files::io_error(&other, error))?;
    let mut app = App::start(Config::new(&home, &other));
    let mut journey = Journey::default();
    let ended = steps(
        &mut app,
        context,
        &mut journey,
        &home,
        &other,
        data.map(|data| data.join("Trash")).as_deref(),
    );
    files::dismiss_all(&mut app);
    let ended = ended.and(app.shutdown());
    journey.finish(context, ended)
}

fn row(app: &App, pane: PaneId, name: &str) -> Result<LogicalPosition, String> {
    panel::row_center(app, pane, name)
        .ok_or_else(|| format!("the {pane:?} panel shows no {name:?} row"))
}

/// Click the visible button `label` of the main window (popups first).
fn click(app: &mut App, label: &str) -> Result<(), String> {
    input::click_button(app.slint(), label)?;
    app.settle();
    Ok(())
}

/// The files of the left panel in display order (folders and `..` left out).
fn file_order(app: &App) -> Vec<String> {
    app.rows(LEFT)
        .iter()
        .filter(|row| !row.parent_entry && !row.directory && row.tree_depth == 0)
        .map(|row| row.name.to_string())
        .collect()
}

/// Click the left panel's header `label`.
/// Close every finished job window titled `title` that stayed open with its outcome, by
/// its Close button (this session presses no key); how many there were.
fn close_finished(app: &mut App, title: &str) -> Result<usize, String> {
    let open: Vec<_> = journey::titled(app, title)
        .into_iter()
        .filter(|window| window.window().is_visible())
        .collect();
    for window in &open {
        // A tool window is sized before it is used: an unsized one has no area.
        crate::app::layout_tool(window, 760, 560);
        app.settle();
        crate::tools_support::click_last(window.window(), "Close")?;
        let watched = window.clone();
        wait(
            app,
            &format!("the {title} window to close"),
            STEP,
            move |_| !watched.window().is_visible(),
        )?;
    }
    Ok(open.len())
}

fn header(app: &mut App, label: &str) -> Result<(), String> {
    let edge = app.window.get_left_pane_left() + app.window.get_left_pane_width();
    let item = inspect::find_all(app.slint(), AccessibleRole::Button, label)
        .into_iter()
        .find(|item| inspect::center(item).x < edge)
        .ok_or_else(|| format!("no {label:?} header in the left panel"))?;
    input::click_at(app.slint(), inspect::center(&item));
    app.settle();
    Ok(())
}

fn steps(
    app: &mut App,
    context: &mut Context,
    journey: &mut Journey,
    home: &Path,
    other: &Path,
    trash: Option<&Path>,
) -> Result<(), String> {
    // 1. Double clicks.
    let position = row(app, LEFT, "folder")?;
    panels::double_click(app, position);
    let inside = home.join("folder");
    wait(app, "a double click to open folder/", STEP, |app| {
        app.location(LEFT).as_deref() == Some(inside.as_path()) && !app.loading(LEFT)
    })?;
    let position = row(app, LEFT, "..")?;
    panels::double_click(app, position);
    wait(app, "a double click on .. to return", STEP, |app| {
        app.location(LEFT).as_deref() == Some(home) && !app.loading(LEFT)
    })?;
    journey.check(
        "the cursor returns to the folder left",
        wait(app, "the cursor on folder", STEP, |app| {
            files::cursor_name(app, LEFT).as_deref() == Some("folder")
        }),
    );
    journey.step("a double click opened folder/, a double click on .. returned to it");

    // 2. A rubber band.
    let gutter = app.window.get_left_list_left() + app.window.get_left_list_grid_width() + 4.0;
    let (first, second) = (row(app, LEFT, "a-one.txt")?, row(app, LEFT, "b-two.txt")?);
    panels::rubber_band(
        app,
        LogicalPosition::new(gutter, first.y),
        LogicalPosition::new(first.x, second.y),
        &[],
    );
    journey::same(&journey::marked(app, LEFT), &["a-one.txt", "b-two.txt"])
        .map_err(|error| format!("the rubber band from the gutter: {error}"))?;
    journey.step("a rubber band from the gutter marked a-one.txt and b-two.txt");

    // 3. Footer F5, Copy, Replace.
    let start = app.copies.len();
    journey::click_footer(app, "F5")?;
    wait(app, "the Copy dialog", STEP, |app| {
        !app.window.get_copy_draft_token().is_empty()
    })?;
    app.screenshot(
        context,
        "copy-dialog",
        "The Copy dialog opened from the footer",
    );
    click(app, "Copy")?;
    wait(app, "the copy to start", STEP, move |app| {
        app.copies.len() > start
    })?;
    let (progress, questions) =
        journey::answer_job(app, start, Duration::from_secs(60), |texts| {
            texts
                .iter()
                .any(|text| text.contains("b-two.txt"))
                .then_some(Choice::Replace)
        })?;
    journey.check(
        "the copy completes after Replace",
        if progress.state == CopyOperationState::Completed
            && progress.failed == 0
            && questions.len() == 1
        {
            Ok(())
        } else {
            Err(format!(
                "{:?}, {} failed, {} question(s)",
                progress.state,
                progress.failed,
                questions.len()
            ))
        },
    );
    journey.check(
        "a-one.txt copied",
        journey::holds(&other.join("a-one.txt"), LONG),
    );
    journey.check(
        "b-two.txt replaced",
        journey::holds(&other.join("b-two.txt"), "two new\n"),
    );
    // The kept original is a warning: the job's window stays and says so.
    let copies: Vec<_> = journey::titled(app, "Copy")
        .into_iter()
        .filter(|window| window.window().is_visible())
        .collect();
    for window in &copies {
        crate::app::layout_tool(window, 760, 560);
    }
    let said = wait(app, "the Copy window's outcome", STEP, |app| {
        journey::titled(app, "Copy")
            .iter()
            .any(|window| journey::window_text(window).contains(KEPT))
    });
    journey.check(
        "the copy's window stays and says the original was kept",
        said.map_err(|error| {
            format!(
                "{error}; Copy windows say {:?}",
                copies.iter().map(journey::window_text).collect::<Vec<_>>()
            )
        }),
    );
    close_finished(app, "Copy")?;
    journey.step(
        "footer F5, Copy, Replace · keep original: both files in the other panel; the job's \
         window kept the outcome until its Close button",
    );

    // 4. A drag onto a folder row.
    let start = app.copies.len();
    let onto = row(app, RIGHT, "into")?;
    let (state, label) = panel::drag_row(app, LEFT, "c-three.txt", onto, &[])?;
    journey.check(
        "a plain drag on one volume offers Move",
        if state == MOVE {
            Ok(())
        } else {
            Err(format!("feedback {state} ({label:?})"))
        },
    );
    wait(app, "the drop to start a move or ask", STEP, move |app| {
        app.copies.len() > start || !app.window.get_copy_draft_token().is_empty()
    })?;
    if !app.window.get_copy_draft_token().is_empty() {
        click(app, "Move")?;
    }
    wait(app, "the move to end", STEP, move |app| {
        files::job(app, start).is_some_and(|job| job.state.is_terminal())
    })?;
    journey.check(
        "the dropped file moved into the folder",
        journey::holds(&other.join("into/c-three.txt"), "three\n")
            .and_then(|()| journey::absent(&home.join("c-three.txt"))),
    );
    journey.check(
        "the moved file leaves the panel",
        wait(app, "c-three.txt to leave", STEP, |app| {
            !journey::shown(app, LEFT).contains("c-three.txt")
        }),
    );
    // A Move with nothing to report closes its window; one that stayed with its outcome
    // is shown, then closed by its Close button (Jobs below says why it stayed).
    if let Some(window) = journey::titled(app, "Move")
        .into_iter()
        .find(|window| window.window().is_visible())
    {
        crate::app::layout_tool(&window, 760, 560);
        app.settle();
        app.screenshot_window(
            context,
            &window,
            "move-outcome",
            "The drag's Move window stayed",
        );
    }
    let stayed = close_finished(app, "Move")?;
    journey.step(format!(
        "drag of c-three.txt onto into/: {label:?}, moved inside; {stayed} Move window(s) \
         stayed with the outcome"
    ));

    // 5. Header sorting.
    header(app, "Sort by Size")?;
    let ascending = file_order(app);
    header(app, "Sort by Size")?;
    let descending = file_order(app);
    header(app, "Sort by Name")?;
    let by_name = file_order(app);
    journey.check(
        "the Size header sorts by size and reverses",
        if ascending == ["b-two.txt", "a-one.txt"] && descending == ["a-one.txt", "b-two.txt"] {
            Ok(())
        } else {
            Err(format!(
                "ascending {ascending:?}, descending {descending:?}"
            ))
        },
    );
    journey.check(
        "the Name header restores the name order",
        if by_name == ["a-one.txt", "b-two.txt"] {
            Ok(())
        } else {
            Err(format!("{by_name:?}"))
        },
    );
    journey.step("Size header twice and Name header: orders by size, reversed, by name");

    // 6. Jobs.
    let jobs = journey::open_jobs(app)?;
    let rows = journey::job_rows(&jobs);
    journey.check(
        "Jobs lists the copy as completed with warnings (an original kept) and the move as \
         completed",
        if journey::job_row(
            &rows,
            "Copy",
            &["Completed with warnings", "2 files · 0 skipped · 0 errors"],
        )
        .is_some()
            && journey::job_row(&rows, "Move", &["Completed"]).is_some()
        {
            Ok(())
        } else {
            Err(format!("Jobs lists {rows:?}"))
        },
    );
    crate::app::layout_tool(&jobs, 760, 560);
    app.settle();
    app.screenshot_window(context, &jobs, "jobs", "Jobs: the copy and the move");
    crate::tools_support::click_last(jobs.window(), "Close")?;
    let watched = jobs.clone();
    journey.check(
        "the Jobs window's Close button closes it",
        wait(app, "Jobs to close", STEP, move |_| {
            !watched.window().is_visible()
        }),
    );
    journey.step("the toolbar's Jobs lists the copy and the move; Close closes it");

    // 7. F8 by clicks.
    let position = row(app, LEFT, "b-two.txt")?;
    panel::click(app, position);
    app.settle();
    journey::same(&journey::marked(app, LEFT), &[])
        .map_err(|error| format!("before F8 nothing should be marked: {error}"))?;
    let start = app.operations.len();
    journey::click_footer(app, "F8")?;
    wait(app, "the Trash confirmation", STEP, |app| {
        !app.window.get_operation_token().is_empty()
    })?;
    if app.window.get_operation_dangerous() {
        return Err("the footer's F8 asked for a permanent delete".into());
    }
    if let Some(trash) = trash {
        click(app, "Move to Trash")?;
        wait(app, "the move to the Trash", STEP, move |app| {
            app.operations[start..]
                .iter()
                .any(|progress| progress.state.is_terminal())
        })?;
        let done = app.operations[start..]
            .iter()
            .find(|progress| progress.state.is_terminal())
            .cloned();
        journey.check(
            "F8 by clicks completes",
            match done {
                Some(progress) if progress.state == OperationState::Completed => Ok(()),
                other => Err(format!("{:?}", other.map(|progress| progress.state))),
            },
        );
        // A trashed entry is named "<label>-midnight-<unique>", with its original path
        // in its .trashinfo: find it by that path, as `files_trash` does.
        journey.check(
            "the file is in the Trash and gone from its folder",
            super::files_trash::entry_for(trash, &home.join("b-two.txt"))
                .and_then(|entry| journey::holds(&entry, "two new\n"))
                .and_then(|()| journey::absent(&home.join("b-two.txt"))),
        );
        journey.step("a click on b-two.txt, footer F8, Move to Trash: the file is in the Trash");
    } else {
        click(app, "Cancel")?;
        wait(app, "the Trash confirmation to close", STEP, |app| {
            app.window.get_operation_token().is_empty()
        })?;
        journey.check(
            "a cancelled Trash confirmation keeps the file",
            journey::holds(&home.join("b-two.txt"), "two new\n"),
        );
        journey.step(
            "a click on b-two.txt, footer F8: the Trash confirmation, cancelled (Windows has \
             no private Recycle Bin)",
        );
    }

    // 8. A drop within 400 ms of the press.
    let start = app.copies.len();
    let from = row(app, LEFT, "folder")?;
    let to = panel::list_spot(app, RIGHT);
    let pressed = Instant::now();
    let button = PointerEventButton::Left;
    input::move_to(app.slint(), from);
    app.slint().dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button,
    });
    for step in 1..=8 {
        let t = step as f32 / 8.0;
        input::move_to(
            app.slint(),
            LogicalPosition::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t),
        );
        app.idle(Duration::from_millis(8));
    }
    while !matches!(app.window.get_drag_feedback_state(), 3..=5)
        && pressed.elapsed() < Duration::from_millis(300)
    {
        input::move_to(app.slint(), to);
        app.idle(Duration::from_millis(10));
    }
    let offered = app.window.get_drag_feedback_state();
    app.slint().dispatch_event(WindowEvent::PointerReleased {
        position: to,
        button,
    });
    let released = pressed.elapsed();
    app.idle(Duration::from_millis(1500));
    let draft = !app.window.get_copy_draft_token().is_empty();
    let started = app.copies.len() - start;
    if draft {
        click(app, "Cancel")?;
    }
    if started > 0 {
        let _ = wait(app, "the quick drop's job to end", STEP, move |app| {
            files::job(app, start).is_some_and(|job| job.state.is_terminal())
        });
        close_finished(app, "Move")?;
        close_finished(app, "Copy")?;
    }
    journey.pending(
        "a drop released within 400 ms of the press does nothing",
        QUICK_DROP,
        if !draft && started == 0 && home.join("folder").is_dir() && !other.join("folder").exists()
        {
            Ok(())
        } else {
            Err(format!(
                "released after {} ms (feedback {offered}): dialog {draft}, {started} job(s), \
                 folder/ moved {}",
                released.as_millis(),
                other.join("folder").exists()
            ))
        },
    );
    journey.step(format!(
        "a drop {} ms after the press (feedback {offered}): dialog {draft}, {started} job(s)",
        released.as_millis()
    ));
    journey.check(
        "nothing is left open",
        if files::main_idle(app) && files::shown_notice(app).is_none() {
            Ok(())
        } else {
            Err(format!(
                "windows {}, notice {:?}",
                app.tool_windows().len(),
                files::shown_notice(app)
            ))
        },
    );
    app.screenshot(context, "done", "After the mouse-only session");
    Ok(())
}
