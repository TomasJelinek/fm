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
//! 8. On the Slint clock, which the journey holds so that the time a synthetic gesture
//!    takes does not count: a drop 250 ms after the press is cancelled (nothing copied
//!    or moved, the panel's status says why), and the same drop 450 ms after the press
//!    moves the folder.
//! 9. In a folder of 60 rows, a wheel of one and a half rows leaves a row cut at the
//!    list's top, drawn in every frame (no blank band), until the list rests on a whole
//!    row; a double click opens the first fully visible row.
use super::{
    journeys_support::{self as journey, Journey, wait},
    panels_support as panels,
};
use crate::{
    Context,
    app::{self, App, Config},
    files_support::{self as files, Choice},
    input, inspect, panel, viewers_stage,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{CopyOperationState, OperationState, PaneId};
use slint::{
    LogicalPosition, Model,
    platform::{PointerEventButton, WindowAdapter, WindowEvent},
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};

const LEFT: PaneId = PaneId::Left;
const RIGHT: PaneId = PaneId::Right;
const STEP: Duration = Duration::from_secs(30);
pub const TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Drag feedback states (`MainWindow.drag-feedback-state`).
const MOVE: i32 = 4;
/// The panel's status line after a drop sooner than 0.4 s (`quick_drag_note`).
const QUICK_NOTE: &str = "Drag cancelled: released within 0.4 s";
/// The folders of step 9's list.
const ROWS: usize = 60;
const LONG: &str = "one, the longest of the three files\n";
/// The Copy window's outcome line for the original kept by Replace · keep original.
const KEPT: &str = "1 file was kept in a recovery folder; Copy details shows where.";

pub fn scenario() -> crate::Scenario {
    crate::Scenario {
        name: "journeys_pointer_only",
        area: "journeys",
        summary: "A session by mouse only: double clicks into and out of a folder, a rubber band, the footer's F5 and the dialog's Copy with Replace, a drag onto a folder row (Move), header sorting, Jobs and its Close, F8 to a private Trash by clicks; on the held Slint clock a drop 250 ms after the press cancelled with its note and one 450 ms after it moving the folder; a wheel of 1.5 rows drawn without a blank band and resting on a whole row; a double click on the first fully visible row.",
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
    for row in 0..ROWS {
        let folder = home.join("rows").join(format!("r{row:02}"));
        std::fs::create_dir_all(&folder).map_err(|error| files::io_error(&folder, error))?;
    }
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

    // 8. Drops timed on the held clock: 250 ms after the press, then 450 ms.
    let start = app.copies.len();
    let to = panel::list_spot(app, RIGHT);
    let from = row(app, LEFT, "folder")?;
    let quick = timed_drop(app, from, to, Duration::from_millis(250))?;
    // Whatever a drop would start has begun once its events are applied.
    app.pump(Duration::from_millis(300));
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
    journey.check(
        "a drop 250 ms after the press does nothing, and the panel says why",
        if !draft
            && started == 0
            && quick.noted
            && home.join("folder").is_dir()
            && !other.join("folder").exists()
        {
            Ok(())
        } else {
            Err(format!(
                "feedback {}: dialog {draft}, {started} job(s), note {}, folder/ moved {}",
                quick.offered,
                quick.noted,
                other.join("folder").exists()
            ))
        },
    );
    let start = app.copies.len();
    let from = row(app, LEFT, "folder")?;
    let late = timed_drop(app, from, to, Duration::from_millis(450))?;
    let moved = wait(
        app,
        "the 450 ms drop to start a move or ask",
        STEP,
        move |app| app.copies.len() > start || !app.window.get_copy_draft_token().is_empty(),
    )
    .and_then(|()| {
        if !app.window.get_copy_draft_token().is_empty() {
            click(app, "Move")?;
        }
        wait(app, "the 450 ms drop's move to end", STEP, move |app| {
            files::job(app, start).is_some_and(|job| job.state.is_terminal())
        })
    })
    .and_then(|()| journey::holds(&other.join("folder/x.txt"), "x\n"))
    .and_then(|()| journey::absent(&home.join("folder")));
    journey.check(
        "the same drop 450 ms after the press moves the folder",
        moved.map_err(|error| format!("feedback {}: {error}", late.offered)),
    );
    close_finished(app, "Move")?;
    journey.step(format!(
        "on the held clock a drop 250 ms after the press (feedback {}) did nothing and noted \
         {QUICK_NOTE:?}; 450 ms after it (feedback {}) it moved folder/",
        quick.offered, late.offered
    ));

    // 9. A wheel of one and a half rows, its rest on a whole row, a double click.
    wheel_and_rest(app, journey, home)?;
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

/// What a drop timed on the held clock showed.
struct TimedDrop {
    /// The drop feedback before the release (3 Copy, 4 Move, 5 not allowed).
    offered: i32,
    /// Whether the left panel's status line then said the drag was released too soon.
    noted: bool,
}

/// Press at `from`, move to `to` and release there `held_for` after the press on the
/// Slint clock, which is held meanwhile: the drag rule counts the clock's time only, so
/// the real time synthetic events take does not count, while the Engine prepares the
/// drag in real time. The clock runs again afterwards.
fn timed_drop(
    app: &mut App,
    from: LogicalPosition,
    to: LogicalPosition,
    held_for: Duration,
) -> Result<TimedDrop, String> {
    panel::wait_input(app);
    let button = PointerEventButton::Left;
    app.hold_clock();
    input::move_to(app.slint(), from);
    app.slint().dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button,
    });
    // The list's press delay, then small, mostly horizontal moves, a frame each.
    let mut elapsed = Duration::from_millis(150);
    app.advance(elapsed);
    const MOVES: u32 = 8;
    for step in 1..=MOVES {
        let t = step as f32 / MOVES as f32;
        input::move_to(
            app.slint(),
            LogicalPosition::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t),
        );
        app.advance(app::FRAME);
        elapsed += app::FRAME;
    }
    // The Engine answers in real time; the bridge publishes the target on its poll,
    // which runs on the clock: let both happen, a millisecond of the clock at a time,
    // keeping a frame before the release.
    let deadline = std::time::Instant::now() + STEP;
    while !matches!(app.window.get_drag_feedback_state(), 3..=5)
        && std::time::Instant::now() < deadline
    {
        input::move_to(app.slint(), to);
        app.pump(app::FRAME);
        if elapsed + app::FRAME < held_for {
            app.advance(Duration::from_millis(1));
            elapsed += Duration::from_millis(1);
        }
    }
    app.advance(held_for.saturating_sub(elapsed));
    let offered = app.window.get_drag_feedback_state();
    app.slint().dispatch_event(WindowEvent::PointerReleased {
        position: to,
        button,
    });
    app.advance(Duration::from_millis(50));
    let noted = app
        .window
        .get_left_details()
        .iter()
        .any(|field| field.value == QUICK_NOTE);
    app.resume_clock();
    Ok(TimedDrop { offered, noted })
}

/// Whether a row of the left list is shown across the list's top edge (a row cut by the
/// edge counts): a list that leaves the band above its first whole row undrawn has none.
fn top_edge_row(app: &App) -> bool {
    let window = &app.window;
    let (left, right) = (
        window.get_left_list_left(),
        window.get_left_list_left() + window.get_left_list_width(),
    );
    let edge = window.get_left_list_top() + 0.5;
    inspect::controls(app.slint(), AccessibleRole::ListItem)
        .iter()
        .map(|(_, item)| inspect::rect(item))
        .any(|(x, y, w, h)| x + w / 2.0 >= left && x + w / 2.0 < right && y <= edge && y + h > edge)
}

/// The distinct colours of the left list's top band in a frame rendered now: the rows
/// of pixels from the list's top edge down half a row, across the list but its
/// scrollbar. A band no row is drawn in is one plain colour.
fn top_band_colours(app: &App) -> usize {
    let (width, height, pixels) = app::render(&app.main);
    let scale = app.slint().scale_factor();
    let window = &app.window;
    let left = ((window.get_left_list_left() + 2.0) * scale) as usize;
    let right = ((window.get_left_list_left() + window.get_left_list_width() - 18.0) * scale)
        .max(0.0) as usize;
    let top = (window.get_left_list_top() * scale) as usize;
    let bottom =
        ((window.get_left_list_top() + window.get_left_view_row_height() / 2.0) * scale) as usize;
    let mut colours = BTreeSet::new();
    for y in top..bottom.min(height) {
        for x in left..right.min(width) {
            let pixel = pixels[y * width + x];
            colours.insert((pixel.r, pixel.g, pixel.b));
        }
    }
    colours.len()
}

/// Step 9: in `rows/`, a wheel of one and a half rows on the held clock; every frame
/// until the list rests is drawn to its top edge; the rest is on a whole row; a double
/// click opens the first fully visible row.
fn wheel_and_rest(app: &mut App, journey: &mut Journey, home: &Path) -> Result<(), String> {
    let position = row(app, LEFT, "rows")?;
    panels::double_click(app, position);
    let folder = home.join("rows");
    wait(app, "a double click to open rows/", STEP, |app| {
        app.location(LEFT).as_deref() == Some(folder.as_path()) && !app.loading(LEFT)
    })?;
    let height = app.window.get_left_view_row_height();
    let window = &app.window;
    let middle = LogicalPosition::new(
        window.get_left_list_left() + window.get_left_list_width() / 2.0,
        window.get_left_list_top() + window.get_left_list_height() / 2.0,
    );
    app.hold_clock();
    input::move_to(app.slint(), middle);
    app.slint().dispatch_event(WindowEvent::PointerScrolled {
        position: middle,
        delta_x: 0.0,
        delta_y: -1.5 * height,
    });
    app.settle();
    // Slint eases a wheel's delta in over the clock; the list then holds still for the
    // rest check (100 ms) before it glides to a whole row (200 ms).
    let offset = |app: &App| -app.window.get_left_list_content_y();
    let part = |y: f32| y.rem_euclid(height);
    let wheeled = 1.5 * height;
    let mut cut = None;
    let mut seen = Vec::new();
    let mut blank = Vec::new();
    let mut frames = 0;
    let mut still = 0;
    let mut last = offset(app);
    // Two seconds of the clock at most; the list rests within half of that.
    for _ in 0..200 {
        frames += 1;
        let now = offset(app);
        if cut.is_none() && (now - wheeled).abs() <= 1.0 {
            cut = Some(now);
        }
        if seen.last() != Some(&now) {
            seen.push(now);
        }
        if !top_edge_row(app) {
            blank.push(format!("{now:.1} px: no row at the list's top edge"));
        } else if now >= height && part(now) <= 0.4 * height {
            // A named folder row fills the band: its text must be in the pixels.
            let colours = top_band_colours(app);
            if colours <= 2 {
                blank.push(format!("{now:.1} px: the top band is {colours} colour(s)"));
            }
        }
        app.advance(app::FRAME);
        let next = offset(app);
        if (next - last).abs() < 0.01 {
            still += 1;
            if still >= 30 {
                break;
            }
        } else {
            still = 0;
        }
        last = next;
    }
    app.resume_clock();
    let rest = offset(app);
    journey.check(
        "a wheel of 1.5 rows takes the list 1.5 rows down, a row cut at its top",
        match cut {
            Some(cut) if (1.0..height - 1.0).contains(&part(cut)) => Ok(()),
            _ => Err(format!(
                "{height} px rows; the offsets were {:?}",
                seen.iter().take(40).collect::<Vec<_>>()
            )),
        },
    );
    let cut = cut.unwrap_or_default();
    journey.check(
        "every frame until the rest draws the list to its top edge",
        if blank.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "{} of {frames} frames blank: {blank:?}",
                blank.len()
            ))
        },
    );
    journey.check(
        "the list rests on a whole row",
        if still >= 30 && !(0.5..=height - 0.5).contains(&part(rest)) && rest > 0.0 {
            Ok(())
        } else {
            Err(format!(
                "offset {rest:.2} px with {height} px rows (still for {still} frames)"
            ))
        },
    );
    // Rows are listed `..` first; the first one whose top is in view.
    let first = ((rest - 0.5) / height).ceil().max(0.0) as usize;
    let name = app
        .names(LEFT)
        .get(first)
        .cloned()
        .ok_or_else(|| format!("no row {first} in rows/"))?;
    let position = row(app, LEFT, &name)?;
    panels::double_click(app, position);
    let opened = folder.join(&name);
    journey.check(
        "a double click opens the first fully visible row",
        wait(
            app,
            &format!("a double click to open {name}"),
            STEP,
            |app| app.location(LEFT).as_deref() == Some(opened.as_path()) && !app.loading(LEFT),
        ),
    );
    journey.step(format!(
        "rows/: a wheel of 1.5 rows cut a row at {cut:.1} px, {frames} frames drawn to the \
         top, rest at {rest:.1} px ({height} px rows), a double click opened {name}"
    ));
    Ok(())
}
