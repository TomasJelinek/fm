//! The viewer's controls, used as a person would: the status bar (the page's bytes out of
//! the file's exact length), Find (Ctrl+F, Enter,
//! F3, Shift+F3) with case sensitivity (the find bar's Aa), Wrap (Ctrl+W and its check
//! box), the Font and presentation (Aa) row, switching between the text and binary
//! modes, Alt+F3 plain text, scrolling to the end and back with keys and the mouse
//! wheel, image zoom (Ctrl+1, Ctrl+Plus, Ctrl+Minus, Ctrl+Equal, the buttons, Ctrl+wheel,
//! Fit), PDF layout, the next and previous
//! file (Space, Backspace), closing with Escape and with the window's own close button
//! (the platform's close request).
//!
//! Space and Backspace load the file they open as it is at that moment: a file changed
//! after the viewer opened shows its new content. In a folder whose order holds more
//! than 8 MiB of paths, they cycle within the contiguous window of the order that the
//! viewer keeps around the opened file (README, "Viewers"): from the first file
//! Backspace shows the window's last file, not the folder's, and from the last file
//! Space shows the window's first file.
use crate::{
    Context,
    app::{App, Config},
    input, panel, viewers_stage,
    viewers_support::{self as support, Failures, Viewing, clip, generated, probe},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::{
    LogicalPosition, Model,
    platform::{Key, WindowEvent},
};
use std::time::{Duration, Instant};

pub const TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// The large log can take long to open (see the latency notes of `viewers_files`).
const OPEN_LIMIT: Duration = Duration::from_secs(120);
const STEP: Duration = Duration::from_secs(10);
/// Steps that may read the whole large log.
const LARGE_STEP: Duration = Duration::from_secs(120);
const NO_MATCH: &str = "No further match";

pub fn controls(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

fn run(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    // Written before the start, so the right panel lists them at once.
    let order = prepare_order(context)?;
    generated::write(
        &context.work.join("reload"),
        "a.txt",
        "alpha first version\n",
    )?;
    generated::write(
        &context.work.join("reload"),
        "b.txt",
        "bravo first version\n",
    )?;
    let mut config = Config::new(&root, &context.work);
    config.plugins = Some(context.work.join("plugins"));
    let mut app = App::start(config);
    let failures = Failures::default();
    panel::enter(&mut app, PaneId::Left, "text")?;

    open(&mut app, "huge.log")
        .and_then(|viewing| {
            failures.check("status bar", status_bar(&mut app, context, &viewing));
            failures.check("find", find(&mut app, context, &viewing));
            failures.check(
                "keyboard scrolling",
                keyboard_scroll(&mut app, context, &viewing),
            );
            failures.check("wheel scrolling", wheel_scroll(&mut app, &viewing));
            viewing.close(&mut app)
        })
        .unwrap_or_else(|error| failures.check("text/huge.log", Err(error)));

    open(&mut app, "long-line.txt")
        .and_then(|viewing| {
            failures.check("wrap", wrap(&mut app, context, &viewing));
            failures.check(
                "presentation row",
                presentation(&mut app, context, &viewing),
            );
            viewing.close(&mut app)
        })
        .unwrap_or_else(|error| failures.check("text/long-line.txt", Err(error)));

    open(&mut app, "utf8.txt")
        .and_then(|viewing| {
            failures.check("text and binary modes", modes(&mut app, context, &viewing));
            viewing.close(&mut app)
        })
        .unwrap_or_else(|error| failures.check("text/utf8.txt", Err(error)));

    open(&mut app, "plain.txt")
        .and_then(|viewing| {
            failures.check("next and previous file", cycle(&mut app, context, &viewing));
            // The window's own close button: the platform's close request.
            close_requested(&mut app, viewing)
        })
        .unwrap_or_else(|error| failures.check("text/plain.txt", Err(error)));
    panel::leave(&mut app, PaneId::Left)?;

    failures.check(
        "Space and Backspace load a file changed after the viewer opened",
        reload(&mut app, context),
    );
    failures.check(
        "Space and Backspace in a folder over 8 MiB of paths",
        large_order(&mut app, context, order),
    );

    panel::enter(&mut app, PaneId::Left, "docs")?;
    failures.check("Alt+F3 plain text", plain_text(&mut app, context));
    match open(&mut app, "report.pdf") {
        Ok(viewing) => {
            failures.check("PDF layout", pdf(&mut app, context, &viewing));
            failures.check("close PDF", viewing.close(&mut app));
        }
        Err(error) => failures.check("docs/report.pdf", Err(error)),
    }
    panel::leave(&mut app, PaneId::Left)?;

    panel::enter(&mut app, PaneId::Left, "images")?;
    match open(&mut app, "photo.png") {
        Ok(viewing) => {
            failures.check("image zoom", zoom(&mut app, context, &viewing));
            failures.check("close image", viewing.close(&mut app));
        }
        Err(error) => failures.check("images/photo.png", Err(error)),
    }
    panel::leave(&mut app, PaneId::Left)?;
    app.shutdown()?;
    failures.finish(context)
}

fn open(app: &mut App, name: &str) -> Result<Viewing, String> {
    open_in(app, PaneId::Left, name)
}

fn open_in(app: &mut App, pane: PaneId, name: &str) -> Result<Viewing, String> {
    panel::select_row(app, pane, name)?;
    let viewing = support::view(app, false, OPEN_LIMIT)?;
    viewing
        .result
        .clone()
        .map_err(|error| format!("{name}: {error}"))?;
    Ok(viewing)
}

/// The window's own close button (its title bar's ×): the platform's close request
/// closes the viewer; the main window gets its activation back.
fn close_requested(app: &mut App, viewing: Viewing) -> Result<(), String> {
    viewing.slint().dispatch_event(WindowEvent::CloseRequested);
    let deadline = Instant::now() + STEP;
    while support::software_window(app, viewing.slint()).is_some() {
        if Instant::now() >= deadline {
            return Err("the window's close request did not close the viewer".into());
        }
        app.idle(Duration::from_millis(10));
    }
    app.slint()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    app.settle();
    Ok(())
}

/// Wait until the viewer shows `name` with text containing `text`.
fn shows(
    app: &mut App,
    viewing: &Viewing,
    what: &str,
    name: &str,
    text: &str,
) -> Result<(), String> {
    viewing.wait(app, what, STEP, |window| {
        window.get_caption().contains(name) && window.get_accessible_text().contains(text)
    })
}

/// Space and Backspace load the file they open as it is at that moment (README): b.txt
/// changed after a.txt was opened shows its new text after Space, and a.txt changed
/// meanwhile shows its new text after Backspace.
fn reload(app: &mut App, context: &mut Context) -> Result<(), String> {
    let folder = context.work.join("reload");
    panel::enter(app, PaneId::Right, "reload")?;
    let viewing = open_in(app, PaneId::Right, "a.txt")?;
    let result = (|| {
        shows(app, &viewing, "a.txt", "a.txt", "alpha first version")?;
        generated::write(&folder, "b.txt", "bravo second, a longer version\n")?;
        viewing.click_content(app);
        input::key(viewing.slint(), Key::Space);
        shows(
            app,
            &viewing,
            "Space to show the changed b.txt",
            "b.txt",
            "bravo second, a longer version",
        )?;
        generated::write(&folder, "a.txt", "alpha second, a longer version\n")?;
        input::key(viewing.slint(), Key::Backspace);
        shows(
            app,
            &viewing,
            "Backspace to show the changed a.txt",
            "a.txt",
            "alpha second, a longer version",
        )?;
        app.screenshot_window(
            context,
            &viewing.software,
            "reloaded-file",
            "Backspace after a.txt changed: its new content",
        );
        Ok(())
    })();
    viewing.close(app)?;
    panel::leave(app, PaneId::Right)?;
    result
}

/// The order budget of a viewer window (`midnight-viewer-engine` `capture::ORDER_BUDGET`)
/// and what each native entry costs besides its path's bytes (`NATIVE_ENTRY`).
const ORDER_BUDGET: usize = 8 * 1024 * 1024;
const NATIVE_ENTRY: usize = 64;
/// Long names (with `.txt`), so that the folder needs fewer files to exceed the budget.
const LONG_NAME: usize = 120;

fn long_name(index: usize) -> String {
    let stem = format!("order-{index:06}-");
    format!("{stem}{}.txt", "x".repeat(LONG_NAME - stem.len() - 4))
}

/// The contiguous window around `focus` that the viewer keeps (`capture::window`): grown
/// alternately after and before the focus while the costs fit the budget.
fn order_window(costs: &[usize], focus: usize) -> std::ops::Range<usize> {
    let mut range = focus..focus + 1;
    let mut used = costs[focus];
    let mut admit = |index: usize| {
        let fits = used + costs[index] <= ORDER_BUDGET;
        if fits {
            used += costs[index];
        }
        fits
    };
    let (mut after, mut before) = (true, true);
    while after || before {
        after = after && range.end < costs.len() && admit(range.end);
        if after {
            range.end += 1;
        }
        before = before && range.start > 0 && admit(range.start - 1);
        if before {
            range.start -= 1;
        }
    }
    range
}

/// The text of the order's file `index`: unique, so a view of one cannot pass for another.
fn order_text(index: usize) -> String {
    format!("order file {index:06}.")
}

/// `work/order`: enough files with long names that their paths exceed the order budget
/// by half; returns how many.
fn prepare_order(context: &mut Context) -> Result<usize, String> {
    let folder = context.work.join("order");
    std::fs::create_dir_all(&folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    let cost = folder.join(long_name(0)).as_os_str().len() + NATIVE_ENTRY;
    let count = ORDER_BUDGET * 3 / 2 / cost + 1;
    let started = Instant::now();
    for index in 0..count {
        let path = folder.join(long_name(index));
        std::fs::write(&path, format!("{}\n", order_text(index)))
            .map_err(|error| format!("{}: {error}", path.display()))?;
    }
    context.note(format!(
        "work/order: {count} files written in {:.1} s",
        started.elapsed().as_secs_f64()
    ));
    Ok(count)
}

/// A folder whose paths exceed the 8 MiB order budget by half: from its first file,
/// Backspace shows the last file of the window the viewer keeps (not the folder's last
/// file) and Space returns; from its last file, Space shows the window's first file and
/// Backspace returns (README: navigation cycles within a contiguous window around the
/// opened file, which reaches the start or end of the folder there).
fn large_order(app: &mut App, context: &mut Context, count: usize) -> Result<(), String> {
    panel::enter(app, PaneId::Right, "order")?;
    let mut costs: Vec<usize> = Vec::new();
    let result = (|| {
        let names = app.names(PaneId::Right);
        let files = names
            .iter()
            .filter(|name| name.starts_with("order-"))
            .count();
        if files != count {
            return Err(format!("the panel lists {files} of the {count} files"));
        }
        let viewing = open_edge(app, PaneId::Right, true, &long_name(0))?;
        let checked = (|| {
            shows(
                app,
                &viewing,
                "the first file",
                &long_name(0),
                &order_text(0),
            )?;
            // The viewer's caption is the native path it holds ("<path> — Midnight
            // Explorer"): every entry costs that path's folder, its name and the slot.
            let caption = viewing.window.get_caption().to_string();
            let path = caption
                .strip_suffix(" — Midnight Explorer")
                .ok_or_else(|| format!("the caption {caption:?} names no path"))?;
            let folder = path.len() - long_name(0).len();
            costs = (0..count)
                .map(|index| folder + long_name(index).len() + NATIVE_ENTRY)
                .collect();
            viewing.click_content(app);
            input::key(viewing.slint(), Key::Backspace);
            // From the first file the window reaches the folder's start.
            let last = order_window(&costs, 0).end - 1;
            shows(
                app,
                &viewing,
                "Backspace on the first file to show the window's last file",
                &long_name(last),
                &order_text(last),
            )?;
            input::key(viewing.slint(), Key::Space);
            shows(
                app,
                &viewing,
                "Space to return to the first file",
                &long_name(0),
                &order_text(0),
            )
        })();
        viewing.close(app)?;
        checked?;
        let first = order_window(&costs, 0);
        let total: usize = costs.iter().sum();
        // From the last file: the window reaches the folder's end.
        let end = order_window(&costs, count - 1);
        let viewing = open_edge(app, PaneId::Right, false, &long_name(count - 1))?;
        let checked = (|| {
            viewing.click_content(app);
            input::key(viewing.slint(), Key::Space);
            shows(
                app,
                &viewing,
                "Space on the last file to show the window's first file",
                &long_name(end.start),
                &order_text(end.start),
            )?;
            app.screenshot_window(
                context,
                &viewing.software,
                "order-window-first",
                &format!(
                    "Space on the last of {count} files: the first file of the viewer's window \
                     ({})",
                    end.start
                ),
            );
            input::key(viewing.slint(), Key::Backspace);
            shows(
                app,
                &viewing,
                "Backspace to return to the last file",
                &long_name(count - 1),
                &order_text(count - 1),
            )
        })();
        viewing.close(app)?;
        checked?;
        context.note(format!(
            "Order over the budget: {count} files with {LONG_NAME}-character names, {:.1} MiB \
             of order; the window from the first file holds {}, from the last file {} (files \
             {}..{})",
            total as f64 / 1_048_576.0,
            first.len(),
            end.len(),
            end.start,
            end.end
        ));
        if first.end >= count || end.start == 0 {
            return Err(format!(
                "the {count} files fit one window ({first:?}, {end:?}): the order did not \
                 exceed the budget"
            ));
        }
        Ok(())
    })();
    panel::leave(app, PaneId::Right)?;
    result
}

/// Put the cursor on the panel's first (Home, `first`) or last (End) row, check it is
/// `name`, and open it with F3.
fn open_edge(app: &mut App, pane: PaneId, first: bool, name: &str) -> Result<Viewing, String> {
    panel::focus_list(app, pane)?;
    input::key(app.slint(), if first { Key::Home } else { Key::End });
    app.settle();
    let names = app.names(pane);
    let mut cursor = usize::try_from(app.cursor(pane)).unwrap_or(usize::MAX);
    // Home stops on the parent row ("..") above the first file.
    if first && names.get(cursor).is_some_and(|row| row != name) {
        input::key(app.slint(), Key::DownArrow);
        app.settle();
        cursor = usize::try_from(app.cursor(pane)).unwrap_or(usize::MAX);
    }
    if names.get(cursor).map(String::as_str) != Some(name) {
        return Err(format!(
            "the cursor is on {:?}, not {name:?}",
            names.get(cursor)
        ));
    }
    let viewing = support::view(app, false, OPEN_LIMIT)?;
    viewing
        .result
        .clone()
        .map_err(|error| format!("{name}: {error}"))?;
    Ok(viewing)
}

fn status_bar(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    viewing.wait(app, "a status", STEP, |window| {
        let status = window.get_status();
        !status.is_empty() && !status.starts_with("Opening")
    })?;
    let status = viewing.status();
    context.note(format!("huge.log status bar: {status:?}"));
    app.screenshot_window(
        context,
        &viewing.software,
        "status-bar",
        &format!("Status bar of a large log: {status}"),
    );
    if status.to_lowercase().contains("error") {
        return Err(format!("the status reports an error: {status:?}"));
    }
    // The page's bytes: from the file's start, out of the file's exact length.
    let length = std::fs::metadata(context.fixture.path("text/huge.log"))
        .map_err(|error| error.to_string())?
        .len();
    match probe::page_bytes(&status) {
        Some((0, end, total)) if end > 0 && total == length => Ok(()),
        other => Err(format!(
            "the status names bytes {other:?} (start, end, length) of the {length}-byte log: \
             {status:?}"
        )),
    }
}

/// Type a query into the open find bar (replacing it) and press Enter.
fn search(app: &mut App, viewing: &Viewing, query: &str) {
    input::replace_text(viewing.slint(), query);
    input::key(viewing.slint(), Key::Return);
    app.settle();
}

fn find(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    let window = &viewing.window;
    input::chord(viewing.slint(), &[Key::Control], "f");
    viewing.wait(app, "the find bar", STEP, |window| {
        window.get_search_visible()
    })?;
    // A line far below the first screen: finding it scrolls there.
    search(app, viewing, "line=700 ");
    viewing.wait(app, "the first match in view", STEP, |window| {
        window.get_accessible_text().contains("line=700 ")
    })?;
    if viewing.status().contains(NO_MATCH) {
        return Err(format!("found nothing: {:?}", viewing.status()));
    }
    app.screenshot_window(
        context,
        &viewing.software,
        "find-match",
        "Ctrl+F, \"line=700 \", Enter: the match is scrolled into view and selected",
    );
    // F3 looks for the next match; there is none.
    input::key(viewing.slint(), Key::F3);
    viewing.wait(app, "no further match after F3", STEP, |window| {
        window.get_status().contains(NO_MATCH)
    })?;
    // Shift+F3 searches backwards: a new query for a line above the match.
    input::replace_text(viewing.slint(), "line=650 ");
    input::chord(viewing.slint(), &[Key::Shift], Key::F3);
    viewing.wait(app, "Shift+F3 to find an earlier line", STEP, |window| {
        window.get_accessible_text().contains("line=650 ")
            && !window.get_status().contains(NO_MATCH)
    })?;
    input::chord(viewing.slint(), &[Key::Shift], Key::F3);
    viewing.wait(app, "Shift+F3 to find no earlier match", STEP, |window| {
        window.get_status().contains(NO_MATCH)
    })?;
    // Case sensitivity: "info" only matches "INFO" when case is ignored.
    if window.get_case_sensitive() {
        return Err("case-sensitive search is on by default".into());
    }
    input::replace_text(viewing.slint(), "info line=40 ");
    input::chord(viewing.slint(), &[Key::Shift], Key::F3);
    viewing.wait(app, "a case-insensitive match", STEP, |window| {
        window.get_accessible_text().contains("INFO line=40 ")
            && !window.get_status().contains(NO_MATCH)
    })?;
    input::click(viewing.slint(), AccessibleRole::Checkbox, "Case sensitive")?;
    viewing.wait(app, "Aa to turn on", STEP, |window| {
        window.get_case_sensitive()
    })?;
    // The check box took focus: Ctrl+F returns to the query.
    input::chord(viewing.slint(), &[Key::Control], "f");
    app.settle();
    input::replace_text(viewing.slint(), "info line=41 ");
    input::key(viewing.slint(), Key::F3);
    viewing.wait(app, "no case-sensitive match", STEP, |window| {
        window.get_query() == "info line=41 " && window.get_status().contains(NO_MATCH)
    })?;
    app.screenshot_window(
        context,
        &viewing.software,
        "find-case-sensitive",
        "Find bar with Aa (case sensitive) on: \"info\" is not found",
    );
    input::click(viewing.slint(), AccessibleRole::Checkbox, "Case sensitive")?;
    viewing.wait(app, "Aa to turn off", STEP, |window| {
        !window.get_case_sensitive()
    })?;
    // Back to the content (a click on it), so scrolling keys go to it.
    viewing.click_content(app);
    Ok(())
}

fn last_line(context: &Context) -> Result<String, String> {
    let text = std::fs::read_to_string(context.fixture.path("text/huge.log"))
        .map_err(|error| error.to_string())?;
    let line = text.lines().last().ok_or("an empty log")?;
    let number = line
        .split("line=")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .ok_or("no line number")?;
    Ok(format!("line={number} "))
}

fn keyboard_scroll(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    let window = &viewing.window;
    let last = last_line(context)?;
    // Scrolling keys belong to the content: click it first.
    viewing.click_content(app);
    input::chord(viewing.slint(), &[Key::Control], Key::Home);
    viewing.wait(app, "the beginning", STEP, |window| {
        window.get_accessible_text().contains("line=0 ")
    })?;
    let top = window.get_position();
    for _ in 0..3 {
        input::key(viewing.slint(), Key::PageDown);
    }
    viewing.wait(app, "PageDown to move down", STEP, |window| {
        window.get_position() > top && !window.get_accessible_text().contains("line=0 ")
    })?;
    input::chord(viewing.slint(), &[Key::Control], Key::End);
    let started = std::time::Instant::now();
    viewing.wait(app, "the last line", LARGE_STEP, |window| {
        window.get_accessible_text().contains(last.as_str())
    })?;
    context.note(format!(
        "latency\tCtrl+End\ttext/huge.log\t{}\tlast line shown",
        started.elapsed().as_millis()
    ));
    app.screenshot_window(
        context,
        &viewing.software,
        "end-of-log",
        "Ctrl+End: the last line of the large log",
    );
    input::chord(viewing.slint(), &[Key::Control], Key::Home);
    viewing.wait(app, "Ctrl+Home back to the beginning", STEP, |window| {
        window.get_accessible_text().contains("line=0 ")
    })?;
    context.note(format!(
        "Keyboard scrolling: PageDown moved from {top:.4}; Ctrl+End showed {last:?}; Ctrl+Home returned."
    ));
    Ok(())
}

fn wheel(viewing: &Viewing, delta_y: f32) {
    let position = LogicalPosition::new(
        viewing.window.get_content_left() + viewing.window.get_content_width() / 2.0,
        viewing.window.get_content_top() + viewing.window.get_content_height() / 2.0,
    );
    viewing
        .slint()
        .dispatch_event(WindowEvent::PointerMoved { position });
    viewing
        .slint()
        .dispatch_event(WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y,
        });
}

/// The wheel scrolls the large log down and back to its first view. Followed by the shown
/// position and the pixels: the content's accessible text is the whole prepared page,
/// which still holds the first lines after a scroll within it.
fn wheel_scroll(app: &mut App, viewing: &Viewing) -> Result<(), String> {
    let top = probe::still_canvas(app, viewing, STEP)?;
    let start = viewing.window.get_displayed_position();
    for _ in 0..8 {
        wheel(viewing, -120.0);
        app.idle(Duration::from_millis(20));
    }
    viewing.wait(app, "the wheel to scroll down", STEP, |window| {
        window.get_position() > start && window.get_displayed_position() > start
    })?;
    let down = probe::still_canvas(app, viewing, STEP)?;
    if down.fingerprint() == top.fingerprint() {
        return Err("the wheel moved the position but the shown lines stayed".into());
    }
    // Wheel back up, a step at a time, until the first view shows again.
    for _ in 0..200 {
        if viewing.window.get_displayed_position() <= start {
            break;
        }
        wheel(viewing, 120.0);
        app.idle(Duration::from_millis(15));
    }
    viewing.wait(app, "the wheel to scroll back to the top", STEP, |window| {
        window.get_displayed_position() <= start && window.get_position() <= start
    })?;
    let back = probe::still_canvas(app, viewing, STEP)?;
    match probe::vertical_shift(&top, &back) {
        Some((0, share)) if share >= probe::SHIFT_CONFIDENCE => Ok(()),
        other => Err(format!(
            "back at the top, the view is not the first view: {other:?} (shift, share of rows)"
        )),
    }
}

fn wrap(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    let window = &viewing.window;
    if !window.get_wrap_available() {
        return Err("Wrap is unavailable for a long text line".into());
    }
    let initial = window.get_wrap_enabled();
    // Wrap off: the line needs a horizontal scroll bar.
    if initial {
        input::chord(viewing.slint(), &[Key::Control], "w");
        viewing.wait(app, "Ctrl+W to turn wrap off", STEP, |window| {
            !window.get_wrap_enabled()
        })?;
    }
    viewing.wait(app, "a horizontal extent below 1", STEP, |window| {
        window.get_horizontal_extent() < 1.0
    })?;
    app.screenshot_window(
        context,
        &viewing.software,
        "wrap-off",
        "Wrap off: one 200,000-character line with a horizontal scroll bar",
    );
    input::click(viewing.slint(), AccessibleRole::Checkbox, "Word wrap")?;
    viewing.wait(app, "the Wrap box to turn wrap on", STEP, |window| {
        window.get_wrap_enabled() && window.get_horizontal_extent() >= 0.999
    })?;
    app.screenshot_window(
        context,
        &viewing.software,
        "wrap-on",
        "Wrap on (check box): the line wraps, no horizontal scroll bar",
    );
    input::chord(viewing.slint(), &[Key::Control], "w");
    viewing.wait(app, "Ctrl+W to toggle wrap again", STEP, |window| {
        !window.get_wrap_enabled()
    })?;
    if initial {
        input::chord(viewing.slint(), &[Key::Control], "w");
        viewing.wait(app, "the initial wrap setting", STEP, |window| {
            window.get_wrap_enabled()
        })?;
    }
    Ok(())
}

fn presentation(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    let shown = viewing.window.get_typography_visible();
    input::click(
        viewing.slint(),
        AccessibleRole::Button,
        "Font and presentation",
    )?;
    viewing.wait(app, "the font row to toggle", STEP, |window| {
        window.get_typography_visible() != shown
    })?;
    if !shown {
        app.screenshot_window(
            context,
            &viewing.software,
            "presentation-row",
            "Aa (Font and presentation): font family and size row",
        );
        let size = viewing.window.get_viewer_font_size();
        input::click(
            viewing.slint(),
            AccessibleRole::Button,
            "Font and presentation",
        )?;
        viewing.wait(app, "the font row to hide", STEP, |window| {
            !window.get_typography_visible()
        })?;
        context.note(format!("Viewer font size in the presentation row: {size}"));
    }
    Ok(())
}

fn modes(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    let window = &viewing.window;
    viewing.wait(app, "the UTF-8 text", STEP, |window| {
        window.get_accessible_text().contains("Příliš")
    })?;
    let modes: Vec<String> = window
        .get_modes()
        .iter()
        .map(|mode| mode.to_string())
        .collect();
    context.note(format!("Viewer modes for utf8.txt: {modes:?}"));
    let Some(binary) = modes
        .iter()
        .find(|mode| {
            let lower = mode.to_lowercase();
            lower.contains("binary") || lower.contains("hex")
        })
        .cloned()
    else {
        return Err(format!("no binary or hex mode among {modes:?}"));
    };
    let text_mode = modes[usize::try_from(window.get_mode_index()).unwrap_or(0)].clone();
    input::click(viewing.slint(), AccessibleRole::Combobox, "Viewer mode")?;
    app.settle();
    input::click(viewing.slint(), AccessibleRole::Button, &binary)?;
    // "Př" in UTF-8 is 50 C5 99.
    viewing.wait(app, "the binary view", STEP, |window| {
        let text: String = window
            .get_accessible_text()
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>()
            .to_uppercase();
        text.contains("50C599")
    })?;
    app.screenshot_window(
        context,
        &viewing.software,
        "binary-mode",
        &format!("utf8.txt in the {binary} mode (chosen in the mode picker)"),
    );
    input::click(viewing.slint(), AccessibleRole::Combobox, "Viewer mode")?;
    app.settle();
    input::click(viewing.slint(), AccessibleRole::Button, &text_mode)?;
    viewing.wait(app, "the text view again", STEP, |window| {
        window.get_accessible_text().contains("Příliš")
    })
}

fn cycle(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    let window = &viewing.window;
    let first = window.get_caption().to_string();
    viewing.wait(app, "plain.txt", STEP, |window| {
        window.get_accessible_text().contains("0000 ")
    })?;
    viewing.click_content(app);
    input::key(viewing.slint(), Key::Space);
    viewing.wait(app, "Space to show the next file", STEP, |window| {
        window.get_caption() != first.as_str() && !window.get_accessible_text().contains("0000 ")
    })?;
    let next = window.get_caption().to_string();
    app.screenshot_window(
        context,
        &viewing.software,
        "next-file",
        &format!("Space: the next file in the panel ({next})"),
    );
    input::key(viewing.slint(), Key::Backspace);
    viewing.wait(app, "Backspace to return to plain.txt", STEP, |window| {
        window.get_caption() == first.as_str() && window.get_accessible_text().contains("0000 ")
    })?;
    // The aliases: Alt+Right shows the next file, Alt+Left the previous one.
    input::chord(viewing.slint(), &[Key::Alt], Key::RightArrow);
    viewing.wait(app, "Alt+Right to show the next file", STEP, |window| {
        window.get_caption() == next.as_str()
    })?;
    input::chord(viewing.slint(), &[Key::Alt], Key::LeftArrow);
    viewing.wait(app, "Alt+Left to return to plain.txt", STEP, |window| {
        window.get_caption() == first.as_str() && window.get_accessible_text().contains("0000 ")
    })?;
    // Backspace on the first file wraps to the last one, Space comes back.
    let order = app.names(PaneId::Left);
    input::key(viewing.slint(), Key::Backspace);
    viewing.wait(app, "Backspace to show the previous file", STEP, |window| {
        window.get_caption() != first.as_str()
    })?;
    let previous = window.get_caption().to_string();
    input::key(viewing.slint(), Key::Space);
    viewing.wait(app, "Space to return to plain.txt", STEP, |window| {
        window.get_caption() == first.as_str()
    })?;
    context.note(format!(
        "Next/previous file: {first:?} → {next:?} → {first:?}; Alt+Right/Alt+Left the same; \
         Backspace → {previous:?} → Space back (panel order {order:?})"
    ));
    Ok(())
}

fn plain_text(app: &mut App, context: &mut Context) -> Result<(), String> {
    panel::select_row(app, PaneId::Left, "README.md")?;
    let viewing = support::view(app, true, OPEN_LIMIT)?;
    let checked = viewing
        .wait(app, "the Markdown source as plain text", STEP, |window| {
            window.get_accessible_text().contains("# Fixture README")
        })
        .and_then(|()| {
            if viewing.window.get_markdown_mode() {
                Err("Alt+F3 rendered the Markdown instead of showing plain text".into())
            } else {
                Ok(())
            }
        });
    app.screenshot_window(
        context,
        &viewing.software,
        "alt-f3-markdown",
        "Alt+F3 on README.md: the plain-text viewer shows the Markdown source",
    );
    viewing.close(app)?;
    checked
}

fn pdf(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    viewing.wait(app, "the PDF page counter", STEP, |window| {
        window.get_document_pages() >= 1 && window.get_document_page() >= 1
    })?;
    let window = &viewing.window;
    context.note(format!(
        "PDF: page {} of {}, layout {}, zoom {:.1}%, status {:?}",
        window.get_document_page(),
        window.get_document_pages(),
        window.get_document_layout(),
        window.get_zoom_percent(),
        viewing.status()
    ));
    let other = if window.get_document_layout() == 1 {
        "Paged document layout"
    } else {
        "Continuous document layout"
    };
    let wanted = if window.get_document_layout() == 1 {
        0
    } else {
        1
    };
    input::click(viewing.slint(), AccessibleRole::Button, other)?;
    viewing.wait(app, other, STEP, |window| {
        window.get_document_layout() == wanted
    })?;
    app.screenshot_window(
        context,
        &viewing.software,
        "pdf-layout",
        &format!("report.pdf after choosing {other}"),
    );
    Ok(())
}

fn zoom(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    viewing.wait(app, "the image", STEP, |window| window.get_image_mode())?;
    let window = &viewing.window;
    let initial = window.get_zoom_percent();
    input::chord(viewing.slint(), &[Key::Control], "1");
    viewing.wait(app, "Ctrl+1 to show actual pixels", STEP, |window| {
        (window.get_zoom_percent() - 100.0).abs() < 0.5
    })?;
    input::chord(viewing.slint(), &[Key::Control], "+");
    viewing.wait(app, "Ctrl+Plus to zoom in", STEP, |window| {
        window.get_zoom_percent() > 110.0
    })?;
    let zoomed = window.get_zoom_percent();
    input::chord(viewing.slint(), &[Key::Control], "-");
    viewing.wait(app, "Ctrl+Minus to zoom out", STEP, |window| {
        window.get_zoom_percent() < zoomed - 1.0
    })?;
    // Ctrl+Equal (the "=" key without Shift) zooms in like Ctrl+Plus: one step of 1.25.
    let equal = window.get_zoom_percent();
    input::chord(viewing.slint(), &[Key::Control], "=");
    viewing.wait(app, "Ctrl+Equal to zoom in", STEP, |window| {
        (window.get_zoom_percent() - equal * 1.25).abs() < 0.5
    })?;
    let before = window.get_zoom_percent();
    input::click(viewing.slint(), AccessibleRole::Button, "Zoom image in")?;
    viewing.wait(app, "the Zoom in button", STEP, |window| {
        window.get_zoom_percent() > before + 1.0
    })?;
    app.screenshot_window(
        context,
        &viewing.software,
        "image-zoomed",
        &format!(
            "photo.png zoomed in to {:.1}% (Ctrl+1, Ctrl+Plus, Ctrl+Minus, Zoom in button)",
            window.get_zoom_percent()
        ),
    );
    // Ctrl+wheel over the image zooms around the pointer.
    let before = window.get_zoom_percent();
    let position = LogicalPosition::new(
        window.get_content_left() + window.get_content_width() / 2.0,
        window.get_content_top() + window.get_content_height() / 2.0,
    );
    input::press(viewing.slint(), Key::Control);
    viewing
        .slint()
        .dispatch_event(WindowEvent::PointerMoved { position });
    viewing
        .slint()
        .dispatch_event(WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y: 120.0,
        });
    input::release(viewing.slint(), Key::Control);
    viewing.wait(app, "Ctrl+wheel to zoom", STEP, |window| {
        (window.get_zoom_percent() - before).abs() > 1.0
    })?;
    let wheeled = window.get_zoom_percent();
    // Fit: the whole picture in the content, never enlarged (`image_zoom.rs`): the zoom
    // it opened with, as Fit is the default image zoom, and not the Ctrl+wheel zoom.
    let fit = (window.get_content_width() / PHOTO[0])
        .min(window.get_content_height() / PHOTO[1])
        .min(1.0)
        * 100.0;
    input::click(
        viewing.slint(),
        AccessibleRole::Button,
        "Fit image to window",
    )?;
    viewing
        .wait(app, "Fit", STEP, |window| {
            let zoom = window.get_zoom_percent();
            (zoom - fit).abs() < 0.5 && (zoom - wheeled).abs() > 1.0
        })
        .map_err(|error| {
            format!(
                "{error}; Fit should show {fit:.1}% (it opened at {initial:.1}%), the Ctrl+wheel \
                 zoom was {wheeled:.1}%"
            )
        })?;
    if (initial - fit).abs() >= 0.5 {
        return Err(format!(
            "photo.png opened at {initial:.1}%, not fitted ({fit:.1}%), although Fit is the \
             default image zoom"
        ));
    }
    context.note(format!(
        "Image zoom: initial {initial:.1}%, Ctrl+Plus {zoomed:.1}%, Ctrl+wheel {wheeled:.1}%, \
         Fit {:.1}% (expected {fit:.1}%); status {:?}",
        window.get_zoom_percent(),
        clip(&viewing.status(), 80)
    ));
    Ok(())
}

/// `images/photo.png` of the fixture: 160 x 120 pixels.
const PHOTO: [f32; 2] = [160.0, 120.0];
