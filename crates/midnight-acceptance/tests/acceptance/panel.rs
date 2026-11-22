//! Panel actions performed with real input: clicks on rows and keys in the focused list.
//!
//! Row positions come from the list's own geometry in every view ([`ListGeometry`]):
//! Detailed lists one row per band, Brief fills columns of `view_rows` rows page by page,
//! Thumbnails fills a grid row by row. Since cd0b3235 a list draws the rows cut at its
//! edges and, after a scroll, glides Detailed and Brief to a whole row on its own:
//! [`wait_rested`] waits for that before a position is read for a later step.
use super::{app::App, input, inspect};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::{
    LogicalPosition,
    platform::{Key, WindowEvent},
};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// A pane's file list as it is laid out now, in window coordinates (logical pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListGeometry {
    /// The list's viewport: rows are drawn, and take clicks, only inside it.
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    /// The scroll offsets: 0 at the start, negative once scrolled.
    pub content_x: f32,
    pub content_y: f32,
    /// The rows' grid; a background gutter and the scroll bar lie to its right.
    pub grid_width: f32,
    /// One band: a row in Detailed and Brief, a grid row in Thumbnails.
    pub row_height: f32,
    /// Cells per band (Brief: columns of a page; Thumbnails: tiles per grid row).
    pub columns: usize,
    /// Bands that fit the viewport (Brief's rows per column).
    pub view_rows: usize,
    /// 0 Detailed, 1 Brief, 2 Thumbnails (a minimal pane has one column in any mode).
    pub mode: i32,
    /// The pane is drawn minimal (a compact window): one column of plain rows.
    pub minimal: bool,
    /// The rows of the list's model, `..` included.
    pub rows: usize,
}

/// A rectangle in window coordinates: x, y, width, height.
pub type Rect = (f32, f32, f32, f32);

impl ListGeometry {
    /// The band (vertical position) and column of the row `index`, as the list's own
    /// `source-row` places rows: Brief fills each page column by column.
    pub fn cell(&self, index: usize) -> (usize, usize) {
        let columns = self.columns.max(1);
        if self.mode == 1 {
            let rows = self.view_rows.max(1);
            let page = columns * rows;
            let within = index % page;
            (index / page * rows + within % rows, within / rows)
        } else {
            (index / columns, index % columns)
        }
    }

    /// How many bands the list holds (its content height in rows).
    pub fn bands(&self) -> usize {
        let columns = self.columns.max(1);
        if self.mode == 1 {
            let rows = self.view_rows.max(1);
            let page = columns * rows;
            let pages = self.rows.div_ceil(page);
            if pages == 0 {
                0
            } else {
                (pages - 1) * rows + rows.min(self.rows - (pages - 1) * page)
            }
        } else {
            self.rows.div_ceil(columns)
        }
    }

    /// The window rectangle of the row `index`'s cell, inside the viewport or not.
    pub fn cell_rect(&self, index: usize) -> Rect {
        let (band, column) = self.cell(index);
        let width = self.grid_width / self.columns.max(1) as f32;
        (
            self.left + self.content_x + column as f32 * width,
            self.top + self.content_y + band as f32 * self.row_height,
            width,
            self.row_height,
        )
    }

    /// The part of `rect` inside the list's viewport, if any.
    pub fn visible(&self, rect: Rect) -> Option<Rect> {
        let (x, y) = (rect.0.max(self.left), rect.1.max(self.top));
        let right = (rect.0 + rect.2).min(self.left + self.width);
        let bottom = (rect.1 + rect.3).min(self.top + self.height);
        (right > x && bottom > y).then_some((x, y, right - x, bottom - y))
    }

    /// The part of the row `index` that the viewport shows, if any.
    pub fn visible_row(&self, index: usize) -> Option<Rect> {
        (index < self.rows)
            .then(|| self.visible(self.cell_rect(index)))
            .flatten()
    }

    /// The lowest scroll offset: the last band ends at the viewport's bottom edge.
    pub fn lowest(&self) -> f32 {
        (self.height - self.bands() as f32 * self.row_height).min(0.0)
    }

    /// Whether the list rests where a settled Detailed or Brief list may: on a row
    /// boundary, or at its very end (its last row ends at the bottom edge).
    pub fn on_row_boundary(&self) -> bool {
        if self.row_height <= 0.0 {
            return true;
        }
        let rows = self.content_y / self.row_height;
        (rows.round() - rows).abs() * self.row_height <= 0.5
            || (self.content_y - self.lowest()).abs() <= 0.5
    }

    /// Whether this view settles on whole rows after a scroll (cd0b3235): Detailed and
    /// Brief do; the tall bands of the Thumbnails grid keep their exact offset.
    pub fn settles_on_rows(&self) -> bool {
        self.mode != 2 || self.minimal
    }

    /// Indices of the rows the viewport shows whole, in index order.
    pub fn whole_rows(&self) -> Vec<usize> {
        (0..self.rows)
            .filter(|index| {
                let full = self.cell_rect(*index);
                self.visible(full)
                    .is_some_and(|shown| shown.3 + 0.5 >= full.3 && shown.2 + 0.5 >= full.2)
            })
            .collect()
    }

    /// A point in the background gutter right of the rows (left of the scroll bar),
    /// halfway down the viewport. A plain click there activates the pane and gives
    /// its list the keyboard; it moves neither the cursor nor a mark (a rubber band
    /// that never moved is dropped on release).
    pub fn gutter(&self) -> LogicalPosition {
        LogicalPosition::new(
            self.left + self.grid_width + 5.0,
            self.top + self.height / 2.0,
        )
    }

    /// The middle of the viewport, where the wheel scrolls the list.
    pub fn center(&self) -> LogicalPosition {
        LogicalPosition::new(
            self.left + self.grid_width / 2.0,
            self.top + self.height / 2.0,
        )
    }
}

/// `pane`'s list geometry as the window reports it now.
pub fn geometry(app: &App, pane: PaneId) -> ListGeometry {
    let window = &app.window;
    let rows = app.rows(pane).len();
    match pane {
        PaneId::Left => ListGeometry {
            left: window.get_left_list_left(),
            top: window.get_left_list_top(),
            width: window.get_left_list_width(),
            height: window.get_left_list_height(),
            content_x: window.get_left_list_content_x(),
            content_y: window.get_left_list_content_y(),
            grid_width: window.get_left_list_grid_width(),
            row_height: window.get_left_view_row_height(),
            columns: window.get_left_view_columns().max(1) as usize,
            view_rows: window.get_left_view_rows().max(1) as usize,
            mode: window.get_left_view_mode(),
            minimal: window.get_compact_window(),
            rows,
        },
        PaneId::Right => ListGeometry {
            left: window.get_right_list_left(),
            top: window.get_right_list_top(),
            width: window.get_right_list_width(),
            height: window.get_right_list_height(),
            content_x: window.get_right_list_content_x(),
            content_y: window.get_right_list_content_y(),
            grid_width: window.get_right_list_grid_width(),
            row_height: window.get_right_view_row_height(),
            columns: window.get_right_view_columns().max(1) as usize,
            view_rows: window.get_right_view_rows().max(1) as usize,
            mode: window.get_right_view_mode(),
            minimal: window.get_compact_window(),
            rows,
        },
    }
}

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

/// Whether `label` is the accessible label of the panel row `name`, in any view: the
/// row's name, followed by its annotations (" · Filesystem compressed", " · Encrypted",
/// " · Folder link", ...) when it has any. A sibling whose own name continues with
/// " · " matches too; `select_row` then reports the cursor on the wrong row.
pub fn is_row_label(label: &str, name: &str) -> bool {
    label
        .strip_prefix(name)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(" · "))
}

/// The row's own accessible item ([`is_row_label`]) inside this pane's list.
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
        .filter(|(label, _)| is_row_label(label, name))
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

/// Window position of the middle of the row `index` in any view: of the part the
/// viewport shows (a row cut at an edge is clicked where it is drawn), or of the whole
/// cell when it is scrolled out of view (a click there would miss it).
pub fn row_position(app: &App, pane: PaneId, index: usize) -> LogicalPosition {
    let geometry = geometry(app, pane);
    let cell = geometry.cell_rect(index);
    let (x, y, width, height) = geometry.visible(cell).unwrap_or(cell);
    LogicalPosition::new(x + width / 2.0, y + height / 2.0)
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

/// Give `pane`'s list the keyboard with a click, as a user does, without moving its
/// cursor or marks: a click on the cursor row where the viewport shows it (at least half
/// of it, or 8 px), otherwise on the background gutter right of the rows, which
/// activates the pane and focuses its list (any view, scrolled or not).
pub fn focus_list(app: &mut App, pane: PaneId) -> Result<(), String> {
    wait_input(app);
    if app.window.get_focused_pane() == pane.index() as i32 {
        return Ok(());
    }
    let list = geometry(app, pane);
    let cursor = usize::try_from(app.cursor(pane))
        .ok()
        .and_then(|row| list.visible_row(row))
        .filter(|(_, _, width, height)| {
            *width >= 8.0 && *height >= (list.row_height / 2.0).min(8.0)
        });
    let position = match cursor {
        Some((x, y, width, height)) => LogicalPosition::new(x + width / 2.0, y + height / 2.0),
        None => list.gutter(),
    };
    click(app, position);
    app.settle();
    if app.window.get_focused_pane() != pane.index() as i32 {
        return Err(format!(
            "clicking the {pane:?} list at {position:?} did not focus it (focused pane {}, \
             list {list:?})",
            app.window.get_focused_pane()
        ));
    }
    Ok(())
}

/// The text of `pane`'s path editor.
pub fn path_draft(app: &App, pane: PaneId) -> String {
    match pane {
        PaneId::Left => app.window.get_left_path_draft(),
        PaneId::Right => app.window.get_right_path_draft(),
    }
    .to_string()
}

/// Type `text` into `pane`'s path editor and press Enter, as a user types a path: the
/// list gets the keyboard with a click, Ctrl+G moves it to the path editor, the editor's
/// text is replaced key by key and must then read `text`. The caller waits for what
/// Enter does (a folder, a file's folder with the cursor on it, a notice).
pub fn type_path(app: &mut App, pane: PaneId, text: &str) -> Result<(), String> {
    focus_list(app, pane)?;
    input::chord(app.slint(), &[Key::Control], "g");
    app.settle();
    if app.window.get_focused_pane() == pane.index() as i32 {
        return Err(format!(
            "Ctrl+G left the keyboard in the {pane:?} list instead of its path editor"
        ));
    }
    input::replace_text(app.slint(), text);
    app.settle();
    let draft = path_draft(app, pane);
    if draft != text {
        return Err(format!(
            "the {pane:?} path editor holds {draft:?} after typing {text:?}"
        ));
    }
    input::key(app.slint(), Key::Return);
    app.settle();
    Ok(())
}

/// Turn the mouse wheel by `delta_y` logical pixels over `position` (negative scrolls
/// down), as Slint delivers a `PointerScrolled` event: a pixel delta, applied by the
/// list's own wheel animation. Not a native wheel notch: the product turns notches
/// (winit line deltas) into its own distance, with the system's lines per notch and
/// reduced motion, only on the native input path; [`notch_at`] takes that path.
/// Nothing is settled: the caller decides which frames to draw meanwhile.
pub fn wheel_at(app: &mut App, position: LogicalPosition, delta_y: f32) {
    input::move_to(app.slint(), position);
    app.slint().dispatch_event(WindowEvent::PointerScrolled {
        position,
        delta_x: 0.0,
        delta_y,
    });
}

/// Turn a mouse wheel by `lines` native notches over `position` (negative scrolls down;
/// fractions as a precision touchpad or a high-resolution wheel reports them), through
/// the panels' acceptance seam (`midnight_ui::acceptance_seams`, decision A1): the winit
/// line delta that the panels' native observer turns into rows, with the lines per notch
/// and reduced motion the panels last read (the seam's settings override stands in for
/// the system). The seam sends the window's frames while the list moves. Nothing is
/// settled; an error when the panels did not take the wheel.
pub fn notch_at(app: &mut App, position: LogicalPosition, lines: f32) -> Result<(), String> {
    use midnight_ui::acceptance_seams::{NativeWheel, wheel};
    match wheel(
        &app.window,
        position,
        NativeWheel::Lines { x: 0.0, y: lines },
    ) {
        Some(true) => Ok(()),
        Some(false) => Err(format!(
            "the panels left a native wheel of {lines} lines at {position:?} to Slint"
        )),
        None => Err("the main window takes no native input: it is not shown".into()),
    }
}

/// Wait until `pane`'s list has come to rest and return its geometry: its offset
/// unchanged for longer than the list takes to start moving on its own. After a user
/// scroll (cd0b3235) Slint eases a wheel delta over 180 ms, the list checks every 100 ms
/// for a whole interval without motion and then glides to the nearest row over 200 ms,
/// so a list still for 500 ms has rested. Detailed and Brief then rest on a row boundary
/// or the list's end; a list placed by the application (a cursor revealed at its bottom
/// edge) and Thumbnails rest at any offset. The limit is a hang guard.
pub fn wait_rested(app: &mut App, pane: PaneId) -> Result<ListGeometry, String> {
    const QUIET: Duration = Duration::from_millis(500);
    const LIMIT: Duration = Duration::from_secs(15);
    let started = Instant::now();
    let mut last = geometry(app, pane);
    let mut still_since = Instant::now();
    loop {
        app.idle(Duration::from_millis(10));
        let now = geometry(app, pane);
        if (now.content_y - last.content_y).abs() > 0.01
            || (now.content_x - last.content_x).abs() > 0.01
            || now.row_height != last.row_height
            || now.height != last.height
            || now.rows != last.rows
        {
            still_since = Instant::now();
        } else if still_since.elapsed() >= QUIET {
            return Ok(now);
        }
        last = now;
        if started.elapsed() >= LIMIT {
            return Err(format!(
                "the {pane:?} list kept moving for {LIMIT:?}: content y {} (rows of {} px, \
                 lowest {}), {} rows, view mode {}",
                now.content_y,
                now.row_height,
                now.lowest(),
                now.rows,
                now.mode
            ));
        }
    }
}

/// One field of a panel's details strip (`DetailRow`): the focused entry's name and
/// facts, a selection summary, or a note such as "Not shown".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detail {
    pub identity: String,
    pub label: String,
    pub value: String,
    pub explanation: String,
}

/// `pane`'s details strip, in order.
pub fn details_of(app: &App, pane: PaneId) -> Vec<Detail> {
    use slint::Model;
    match pane {
        PaneId::Left => app.window.get_left_details(),
        PaneId::Right => app.window.get_right_details(),
    }
    .iter()
    .map(|row| Detail {
        identity: row.identity.to_string(),
        label: row.label.to_string(),
        value: row.value.to_string(),
        explanation: row.explanation.to_string(),
    })
    .collect()
}

/// The free space shown at the right of `pane`'s details strip ("" when none).
pub fn free_space(app: &App, pane: PaneId) -> String {
    match pane {
        PaneId::Left => app.window.get_left_free_space(),
        PaneId::Right => app.window.get_right_free_space(),
    }
    .to_string()
}

/// Put the cursor on `name`: a click when the row is in view, otherwise Home and the
/// arrow keys in the focused list, as the view moves (Detailed and Brief: Down is the
/// next row; Thumbnails: Down is the next grid row, Right the next cell).
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
        let list = geometry(app, pane);
        let (down, right) = if list.mode == 2 {
            (index / list.columns.max(1), index % list.columns.max(1))
        } else {
            (index, 0)
        };
        for _ in 0..down {
            input::key(app.slint(), Key::DownArrow);
        }
        for _ in 0..right {
            input::key(app.slint(), Key::RightArrow);
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
