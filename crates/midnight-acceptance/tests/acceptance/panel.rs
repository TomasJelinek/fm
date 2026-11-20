//! Panel actions performed with real input: clicks on rows and keys in the focused list.
use super::{app::App, input, inspect};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::{LogicalPosition, platform::Key};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

/// Panel input is held while a finished job's result is being shown; real keys and
/// clicks are swallowed meanwhile, so wait it out first.
#[track_caller]
pub fn wait_input(app: &mut App) {
    app.until("panel input to be accepted again", |app| {
        !app.window.get_mutation_input_blocked() && !app.window.get_embedded_modal_open()
    });
}

/// Click a list position. A second click on the same spot within the double-click
/// interval would open the row, so wait that interval out first.
pub fn click(app: &mut App, position: LogicalPosition) {
    if let Some((when, last)) = app.last_click
        && when.elapsed() < Duration::from_millis(650)
        && (last.x - position.x).abs() < 8.0
        && (last.y - position.y).abs() < 8.0
    {
        app.idle(Duration::from_millis(650).saturating_sub(when.elapsed()));
    }
    input::click_at(app.slint(), position);
    app.last_click = Some((std::time::Instant::now(), position));
}

/// The row's own accessible item (labelled with its name) inside this pane's list.
fn row_item(app: &App, pane: PaneId, name: &str) -> Option<LogicalPosition> {
    let (left, width) = match pane {
        PaneId::Left => (
            app.window.get_left_list_left(),
            app.window.get_left_list_width(),
        ),
        PaneId::Right => (
            app.window.get_right_list_left(),
            app.window.get_right_list_width(),
        ),
    };
    let top = match pane {
        PaneId::Left => app.window.get_left_list_top(),
        PaneId::Right => app.window.get_right_list_top(),
    };
    let bottom = top
        + match pane {
            PaneId::Left => app.window.get_left_list_height(),
            PaneId::Right => app.window.get_right_list_height(),
        };
    inspect::controls(app.slint(), AccessibleRole::ListItem)
        .into_iter()
        .filter(|(label, _)| label == name)
        .map(|(_, item)| inspect::rect(&item))
        .find(|(x, y, w, h)| {
            let center = (x + w / 2.0, y + h / 2.0);
            center.0 >= left
                && center.0 < left + width
                && center.1 >= top + 1.0
                && center.1 < bottom - 1.0
        })
        // The middle of the row: clear of the tree expander at its left edge.
        .map(|(x, y, w, h)| LogicalPosition::new(x + w / 2.0, y + h / 2.0))
}

/// The middle of the row `name` in `pane`, when it is shown.
pub fn row_center(app: &App, pane: PaneId, name: &str) -> Option<LogicalPosition> {
    row_item(app, pane, name)
}

/// A point in `pane`'s list below its last row (the panel's own folder as a drop
/// target), or its middle when rows fill it.
pub fn list_spot(app: &App, pane: PaneId) -> LogicalPosition {
    let window = &app.window;
    let (left, top, width, height) = match pane {
        PaneId::Left => (
            window.get_left_list_left(),
            window.get_left_list_top(),
            window.get_left_list_width(),
            window.get_left_list_height(),
        ),
        PaneId::Right => (
            window.get_right_list_left(),
            window.get_right_list_top(),
            window.get_right_list_width(),
            window.get_right_list_height(),
        ),
    };
    LogicalPosition::new(left + width / 2.0, top + height - 12.0)
}

/// Drag the row `name` of `pane` with the mouse to `to` while holding `modifiers`, as a
/// user drags a row to the other panel: press, wait out the list's press delay, move in
/// small mostly horizontal steps while the application runs (a list turns a fast
/// vertical move into scrolling), wait until the drop target's feedback settles, then
/// release. Returns the feedback state seen at the release (3 Copy, 4 Move,
/// 5 not allowed) and its label.
pub fn drag_row(
    app: &mut App,
    pane: PaneId,
    name: &str,
    to: LogicalPosition,
    modifiers: &[Key],
) -> Result<(i32, String), String> {
    use slint::platform::{PointerEventButton, WindowEvent};
    wait_input(app);
    let from = row_item(app, pane, name)
        .ok_or_else(|| format!("no {name:?} row is shown in the {pane:?} panel to drag"))?;
    let button = PointerEventButton::Left;
    input::move_to(app.slint(), from);
    app.slint().dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button,
    });
    app.idle(Duration::from_millis(150));
    for modifier in modifiers {
        input::press(app.slint(), *modifier);
    }
    const STEPS: u32 = 24;
    for step in 1..=STEPS {
        let t = step as f32 / STEPS as f32;
        let position =
            LogicalPosition::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
        input::move_to(app.slint(), position);
        app.idle(Duration::from_millis(16));
    }
    // The engine prepares the drag and checks the destination asynchronously.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !matches!(app.window.get_drag_feedback_state(), 3..=5)
        && std::time::Instant::now() < deadline
    {
        input::move_to(app.slint(), to);
        app.idle(Duration::from_millis(30));
    }
    let state = app.window.get_drag_feedback_state();
    let label = app.window.get_drag_label().to_string();
    app.slint().dispatch_event(WindowEvent::PointerReleased {
        position: to,
        button,
    });
    for modifier in modifiers.iter().rev() {
        input::release(app.slint(), *modifier);
    }
    app.settle();
    if !matches!(state, 3..=5) {
        return Err(format!(
            "the drag of {name:?} never offered a drop (feedback state {state}, label {label:?}, \
             drag active {})",
            app.window.get_drag_active()
        ));
    }
    Ok((state, label))
}

/// Window position of the middle of `index`'s name cell in a detailed panel list.
pub fn row_position(app: &App, pane: PaneId, index: usize) -> LogicalPosition {
    let window = &app.window;
    let (left, top, content, height) = match pane {
        PaneId::Left => (
            window.get_left_list_left(),
            window.get_left_list_top(),
            window.get_left_list_content_y(),
            window.get_left_view_row_height(),
        ),
        PaneId::Right => (
            window.get_right_list_left(),
            window.get_right_list_top(),
            window.get_right_list_content_y(),
            window.get_right_view_row_height(),
        ),
    };
    LogicalPosition::new(left + 140.0, top + content + (index as f32 + 0.5) * height)
}

fn visible_rows(app: &App, pane: PaneId) -> (usize, usize) {
    let window = &app.window;
    let (start, end) = match pane {
        PaneId::Left => (
            window.get_left_visible_row_start(),
            window.get_left_visible_row_end(),
        ),
        PaneId::Right => (
            window.get_right_visible_row_start(),
            window.get_right_visible_row_end(),
        ),
    };
    (start.max(0) as usize, end.max(0) as usize)
}

pub fn index_of(app: &App, pane: PaneId, name: &str) -> Result<usize, String> {
    let names = app.names(pane);
    names.iter().position(|row| row == name).ok_or_else(|| {
        format!(
            "the {pane:?} panel shows no {name:?}; it shows {} rows: {:?}",
            names.len(),
            sample(&names)
        )
    })
}

fn sample(names: &[String]) -> Vec<&String> {
    names.iter().take(40).collect()
}

/// Click the cursor row of the pane (or its first visible row): activates the pane
/// and focuses its list without moving the cursor elsewhere.
pub fn focus_list(app: &mut App, pane: PaneId) -> Result<(), String> {
    wait_input(app);
    if app.window.get_focused_pane() == pane.index() as i32 {
        return Ok(());
    }
    let (start, end) = visible_rows(app, pane);
    let cursor = usize::try_from(app.cursor(pane))
        .ok()
        .filter(|row| *row >= start && *row + 1 < end);
    let position = row_position(app, pane, cursor.unwrap_or(start));
    click(app, position);
    app.settle();
    if app.window.get_focused_pane() != pane.index() as i32 {
        return Err(format!(
            "clicking the {pane:?} list did not focus it (focused pane {})",
            app.window.get_focused_pane()
        ));
    }
    Ok(())
}

/// Put the cursor on `name`: a click when the row is in view, otherwise Home and
/// Down keys in the focused list.
pub fn select_row(app: &mut App, pane: PaneId, name: &str) -> Result<(), String> {
    wait_input(app);
    let index = index_of(app, pane, name)?;
    let already =
        app.cursor(pane) == index as i32 && app.window.get_focused_pane() == pane.index() as i32;
    if already {
        return Ok(());
    }
    if let Some(position) = row_item(app, pane, name) {
        click(app, position);
    } else {
        focus_list(app, pane)?;
        input::key(app.slint(), Key::Home);
        for _ in 0..index {
            input::key(app.slint(), Key::DownArrow);
        }
    }
    app.settle();
    let cursor = app.cursor(pane);
    let names = app.names(pane);
    let current = usize::try_from(cursor).ok().and_then(|row| names.get(row));
    if current.is_none_or(|row| row != name) {
        return Err(format!(
            "the {pane:?} cursor is on {current:?} (row {cursor}) instead of {name:?} (row {index})"
        ));
    }
    if app.window.get_focused_pane() != pane.index() as i32 {
        return Err(format!(
            "the {pane:?} list lost focus after selecting {name:?}"
        ));
    }
    Ok(())
}

/// Wait until the pane shows `path` with its listing complete.
#[track_caller]
pub fn wait_location(app: &mut App, pane: PaneId, path: &Path) {
    let wanted = path.to_path_buf();
    app.until(
        &format!("the {pane:?} panel to show {}", path.display()),
        move |app| app.location(pane).as_ref() == Some(&wanted) && !app.loading(pane),
    );
}

/// Enter the folder `name` with Enter on its row.
pub fn enter(app: &mut App, pane: PaneId, name: &str) -> Result<PathBuf, String> {
    let parent = app.location(pane).ok_or("the panel shows no folder")?;
    select_row(app, pane, name)?;
    input::key(app.slint(), Key::Return);
    let target = parent.join(name);
    wait_location(app, pane, &target);
    Ok(target)
}

/// Go to the parent with Backspace; the cursor returns to the folder we came from.
pub fn leave(app: &mut App, pane: PaneId) -> Result<PathBuf, String> {
    let current = app.location(pane).ok_or("the panel shows no folder")?;
    let parent = current.parent().ok_or("already at the root")?.to_path_buf();
    input::key(app.slint(), Key::Backspace);
    wait_location(app, pane, &parent);
    let came_from = current
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    let names = app.names(pane);
    let cursor = usize::try_from(app.cursor(pane))
        .ok()
        .and_then(|row| names.get(row));
    if cursor != came_from.as_ref() {
        return Err(format!(
            "after Backspace the cursor is on {cursor:?}, not the folder we left ({came_from:?})"
        ));
    }
    Ok(parent)
}

/// Names on disk, as the panel should list them (hidden entries included).
pub fn disk_names(path: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
