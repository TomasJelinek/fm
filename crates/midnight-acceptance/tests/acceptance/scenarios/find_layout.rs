//! The Find window where its behavior depends on size, density and theme: no clipped
//! or overlapping controls at its preferred size, narrow and compact (Blue, Dark and
//! Touch with large text); a narrow window stacks Remove under Add and puts Browse… and
//! OK under the typed path, and scrolls the result columns sideways with Left and Right;
//! a compact window names its field "Query", hides the query area while a search runs
//! and, with text larger than 20 px, shows the idle status above the results; a column
//! drag never sorts and its width is kept for the next search.
use crate::{
    Context,
    app::{self, App, Config},
    input, inspect,
    options_support::SettingsStore,
    scenarios::find_support::{self as find, BASE, Findings, Query, open_find, search},
    tools_support::{self as tools, Tool},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{PreferenceTheme, UiDensity};
use slint::{
    LogicalPosition,
    platform::{Key, WindowAdapter},
};
use std::{
    path::Path,
    time::{Duration, SystemTime},
};

/// The UI review's layout checks for the Find window at `width` × `height`.
fn layout_clean(
    app: &mut App,
    context: &mut Context,
    window: &Tool,
    (width, height): (u32, u32),
    name: &str,
) -> Result<(), String> {
    app::layout_tool(window, width, height);
    app.settle();
    let _ = app::render(window);
    let problems: Vec<String> = inspect::check_layout(window.window(), width as f32, height as f32)
        .into_iter()
        .map(|finding| format!("{}: {}", finding.kind, finding.detail))
        .collect();
    app.screenshot_window(
        context,
        window,
        name,
        &format!("The Find window at {width}×{height}"),
    );
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("{name} at {width}×{height}: {problems:?}"))
    }
}

fn rect(window: &Tool, role: AccessibleRole, label: &str) -> Result<inspect::Rect, String> {
    tools::control(window.window(), role, label).map(|item| inspect::rect(&item))
}

fn tree(root: &Path) -> Result<(), String> {
    let now = SystemTime::now();
    for index in 0..40 {
        tools::write_at(
            &root
                .join(format!("folder-{}", index % 4))
                .join(format!("a-rather-long-result-name-{index:02}.txt")),
            b"layout\n",
            now,
        )?;
    }
    std::fs::create_dir_all(root.join("slow")).map_err(|error| error.to_string())?;
    tools::sparse(&root.join("slow/huge.txt"), 1 << 30)
}

pub fn layout(context: &mut Context) -> Result<(), String> {
    let root = context.work.join("layout");
    tree(&root)?;
    let mut findings = Findings::default();
    let mut app = App::start(Config::new(&root, &context.work));
    let window = open_find(&mut app)?;
    findings.check(
        "Blue at the preferred size",
        layout_clean(&mut app, context, &window, (780, 560), "blue-780"),
    );
    findings.check("a narrow window", narrow(&mut app, context, &window));
    findings.check("shortened result paths", path_tails(&mut app, &window));
    findings.check("columns", columns(&mut app, &window));
    findings.check(
        "a compact window",
        compact(&mut app, context, &window, false),
    );
    find::close(&mut app, &window);
    app.shutdown()?;

    let mut config = Config::new(&root, &context.work);
    config.theme = PreferenceTheme::Dark;
    let mut app = App::start(config);
    let window = open_find(&mut app)?;
    findings.check(
        "Dark at the preferred size",
        layout_clean(&mut app, context, &window, (780, 560), "dark-780"),
    );
    findings.check(
        "Dark narrow",
        layout_clean(&mut app, context, &window, (560, 520), "dark-560"),
    );
    find::close(&mut app, &window);
    app.shutdown()?;

    // Touch with 24 px text: wider "narrow" threshold, idle status in the query area.
    let store = SettingsStore::seeded(|preferences| preferences.touch_font_size = 24);
    let mut config = Config::new(&root, &context.work);
    config.density = UiDensity::Touch;
    config.settings = Some(store);
    let mut app = App::start(config);
    let window = open_find(&mut app)?;
    findings.check(
        "Touch with large text",
        layout_clean(&mut app, context, &window, (900, 640), "touch-900"),
    );
    findings.check("Touch compact", compact(&mut app, context, &window, true));
    find::close(&mut app, &window);
    app.shutdown()?;
    context.note("Find window layout clean in Blue, Dark and Touch (24 px) at preferred, narrow and compact sizes; narrow stacking and sideways column scrolling; compact Query field, hidden query area while searching, status above the results with large text; a column drag never sorts and its width is kept");
    findings.into_result()
}

/// Below 620 px: Remove under Add, Browse… and OK under the typed path; the result
/// columns scroll sideways with Right and Left.
fn narrow(app: &mut App, context: &mut Context, window: &Tool) -> Result<(), String> {
    layout_clean(app, context, window, (560, 560), "blue-560")?;
    let w = window.window();
    let add = rect(window, AccessibleRole::Button, "Add")?;
    let remove = rect(window, AccessibleRole::Button, "Remove")?;
    if remove.1 < add.1 + add.3 - 0.5 || (remove.0 - add.0).abs() > 1.0 {
        return Err(format!("narrow: Add at {add:?}, Remove at {remove:?}"));
    }
    input::click_button(w, "Add")?;
    app.settle();
    let field = rect(window, AccessibleRole::TextInput, "Folder to add")?;
    for button in ["Browse…", "OK"] {
        let shown = rect(window, AccessibleRole::Button, button)?;
        if shown.1 < field.1 + field.3 - 0.5 {
            return Err(format!(
                "narrow: {button} at {shown:?} is not under the path at {field:?}"
            ));
        }
    }
    layout_clean(app, context, window, (560, 560), "blue-560-add")?;
    input::key(w, Key::Escape);
    app.settle();
    // Sideways scrolling of the result columns. The form keeps its defaults (*.*,
    // Subfolders), so nothing in the narrow, scrolled query area needs setting.
    let text = find::run(app, window)?;
    find::all_rows(app, window)?;
    if !text.starts_with("Completed") {
        return Err(format!("*.* ended {text:?}"));
    }
    find::select_result(app, window, "a-rather-long-result-name-00.txt", None)?;
    // The headers' places before Right, after it and after Left. One step may scroll a
    // whole column out of view (how far depends on the font), so the first header that
    // stays in view is measured.
    const HEADERS: [&str; 3] = ["Sort by Name", "Sort by Extension", "Sort by Path"];
    let places = |window: &Tool| {
        HEADERS.map(|label| {
            rect(window, AccessibleRole::Button, label)
                .ok()
                .map(|place| place.0)
        })
    };
    let before = places(window);
    input::key(w, Key::RightArrow);
    app.settle();
    let scrolled = places(window);
    input::key(w, Key::LeftArrow);
    app.settle();
    let back = places(window);
    let measured = (0..HEADERS.len()).find_map(|index| {
        Some((
            HEADERS[index],
            before[index]?,
            scrolled[index]?,
            back[index]?,
        ))
    });
    match measured {
        Some((_, before, scrolled, back))
            if scrolled < before - 10.0 && (back - before).abs() <= 0.5 => {}
        _ => {
            return Err(format!(
                "Right and Left in the results: header places {HEADERS:?} at x {before:?} → \
                 {scrolled:?} → {back:?}"
            ));
        }
    }
    Ok(())
}

/// A narrow window shortens the Path cells; like the fed list's Folder cells (and every
/// other shortened path, see `PathText`), they keep the last folder, so results with a
/// common parent folder stay distinct.
fn path_tails(app: &mut App, window: &Tool) -> Result<(), String> {
    app::layout_tool(window, 560, 560);
    app.settle();
    let text = find::run(app, window)?;
    find::all_rows(app, window)?;
    if !text.starts_with("Completed") {
        return Err(format!("*.* ended {text:?}"));
    }
    let (shortened, missing) = missing_tails(window);
    if shortened == 0 {
        return Err("no Path cell is shortened at 560 px wide".into());
    }
    if missing.is_empty() {
        return Ok(());
    }
    Err(format!(
        "finding FIND-RESULTS-PATH-TAIL: {} of {shortened} shortened Path cells cut the end of the path and hide its last folder, so results in different folders look alike (the fed list keeps it): {:?}",
        missing.len(),
        missing.first()
    ))
}

/// The number of shown result rows whose Path cell is shortened (its "..." marker
/// shows), and those without the path's last component (`/folder`): (row, the texts the
/// row shows).
fn missing_tails(window: &Tool) -> (usize, Vec<(String, Vec<String>)>) {
    let _ = app::render(window);
    let mut shortened_rows = 0;
    let mut missing = Vec::new();
    if let Some(list) = inspect::find(window.window(), AccessibleRole::List, "Find results") {
        inspect::walk(&list, &mut |item| {
            if item.accessible_role() == AccessibleRole::ListItem
                && inspect::visible_rect(item).is_some()
                && let Some(label) = inspect::label(item)
            {
                let row = find::parse_row(&label, 0, false);
                let tail = Path::new(&row.folder)
                    .file_name()
                    .map(|name| format!("{}{}", std::path::MAIN_SEPARATOR, name.to_string_lossy()))
                    .unwrap_or_default();
                let texts = tools::item_texts(item);
                // A shortened path shows the omission marker; its label keeps the whole
                // text, cut by its width.
                if texts.iter().any(|text| text == "...") {
                    shortened_rows += 1;
                    if !texts.contains(&tail) {
                        missing.push((row.name, texts));
                    }
                }
            }
        });
    }
    (shortened_rows, missing)
}

/// A separator drag resizes without sorting; the width is kept for the next search.
fn columns(app: &mut App, window: &Tool) -> Result<(), String> {
    app::layout_tool(window, 1300, 900);
    app.settle();
    let w = window.window();
    let (_, rows) = search(app, window, BASE)?;
    let order: Vec<String> = rows.iter().map(|row| row.name.clone()).collect();
    let before = rect(window, AccessibleRole::Button, "Sort by Name")?;
    let grip = LogicalPosition::new(before.0 + before.2 + 5.0, before.1 + before.3 / 2.0);
    input::drag(w, grip, LogicalPosition::new(grip.x + 40.0, grip.y), 6);
    app.settle();
    let after = rect(window, AccessibleRole::Button, "Sort by Name")?;
    if (after.2 - before.2 - 40.0).abs() > 1.5 {
        return Err(format!(
            "the Name separator drag changed its width {:.0} → {:.0}",
            before.2, after.2
        ));
    }
    app.idle(Duration::from_millis(200));
    let now: Vec<String> = find::all_rows(app, window)?
        .iter()
        .map(|row| row.name.clone())
        .collect();
    if now != order
        || !inspect::visible_texts(w)
            .iter()
            .any(|text| text == "Name ↑")
    {
        return Err("a separator drag sorted the results".into());
    }
    let (_, _) = search(
        app,
        window,
        Query {
            masks: "*.txt",
            ..BASE
        },
    )?;
    let kept = rect(window, AccessibleRole::Button, "Sort by Name")?;
    if (kept.2 - after.2).abs() > 0.5 {
        return Err(format!(
            "the next search reset the Name width {:.0} → {:.0}",
            after.2, kept.2
        ));
    }
    Ok(())
}

/// Every text of the window, shown or scrolled away.
fn all_texts(window: &Tool) -> Vec<String> {
    let mut texts = Vec::new();
    for root in inspect::roots(window.window()) {
        inspect::walk(&root, &mut |item| {
            if let Some(text) = tools::text_of(item)
                && !text.trim().is_empty()
            {
                texts.push(text);
            }
        });
    }
    texts
}

/// Every status line of the window, shown or scrolled away: their rectangles.
fn status_rects(window: &Tool) -> Vec<inspect::Rect> {
    let mut found = Vec::new();
    for root in inspect::roots(window.window()) {
        inspect::walk(&root, &mut |item| {
            if item.accessible_role() == AccessibleRole::Button
                && inspect::label(item).is_some_and(|label| find::is_status(&label))
            {
                found.push(inspect::rect(item));
            }
        });
    }
    found
}

/// Whether a status line is under the results list, and whether one is in the query
/// area above it. A line in the query area counts as above even where that area's
/// scroller holds it out of view, below its viewport and so below the list in window
/// coordinates: the area that clips it ends above the list. Whether a status line is
/// in view is checked separately.
fn status_places(window: &Tool) -> Result<(bool, bool), String> {
    let list = rect(window, AccessibleRole::List, "Find results")?;
    let (mut under, mut above) = (false, false);
    for root in inspect::roots(window.window()) {
        inspect::walk(&root, &mut |item| {
            if item.accessible_role() == AccessibleRole::Button
                && inspect::label(item).is_some_and(|label| find::is_status(&label))
            {
                let shown = inspect::rect(item);
                let in_area_above =
                    inspect::clip_rect(item).is_some_and(|clip| clip.1 + clip.3 <= list.1 + 1.0);
                if in_area_above || shown.1 + shown.3 <= list.1 + 1.0 {
                    above = true;
                } else if shown.1 >= list.1 + list.3 - 1.0 {
                    under = true;
                }
            }
        });
    }
    Ok((under, above))
}

/// Below 360 px high: the field is "Query"; while a search runs the query area is
/// hidden and the results keep the status under them; idle, text over 20 px moves the
/// status into the query area. The form is filled at full size (a short window scrolls
/// its query area), then the window is made short.
fn compact(
    app: &mut App,
    context: &mut Context,
    window: &Tool,
    large_text: bool,
) -> Result<(), String> {
    let size = (700, 340);
    let name = if large_text {
        "touch-700x340"
    } else {
        "blue-700x340"
    };
    let w = window.window();
    app::layout_tool(window, find::SIZE.0, find::SIZE.1);
    find::fill(app, window, BASE)?;
    layout_clean(app, context, window, size, name)?;
    // The label may be scrolled out of the short query area: look at every text.
    let labels = all_texts(window);
    if !labels.iter().any(|text| text == "Query") || labels.iter().any(|text| text == "File name") {
        return Err(format!(
            "a compact window does not call the field Query: {labels:?}"
        ));
    }
    // An idle status: in the query area with large text, under the results otherwise.
    let text = find::run(app, window)?;
    app.settle();
    let (under, above) = status_places(window)?;
    if (under, above) != (!large_text, large_text) {
        return Err(format!(
            "compact, large text {large_text}: after {text:?} a status under the results {under}, above them {above}"
        ));
    }
    // Shown there, not only placed there: the user sees how the search ended.
    let _ = app::render(window);
    app.screenshot_window(
        context,
        window,
        &format!("{name}-idle"),
        "A compact window after a search",
    );
    let mut problems = Vec::new();
    if !inspect::labels(w, AccessibleRole::Button)
        .iter()
        .any(|label| find::is_status(label))
    {
        let query = rect(window, AccessibleRole::TextInput, "File name masks").ok();
        problems.push(format!(
            "finding FIND-COMPACT-STATUS-HIDDEN: after {text:?} no status line is in view in the {}×{} window (large text {large_text}); status lines at {:?}, the query field at {query:?}",
            size.0,
            size.1,
            status_rects(window)
        ));
    }
    // A running search hides the query area; the status stays under the results.
    app::layout_tool(window, find::SIZE.0, find::SIZE.1);
    app.settle();
    let slow = context.work.join("layout/slow");
    find::add_root(app, window, &slow, false)?;
    find::fill(
        app,
        window,
        Query {
            content: Some("needle"),
            ..BASE
        },
    )?;
    app::layout_tool(window, size.0, size.1);
    app.settle();
    tools::click_last(w, "Find")?;
    let watched = window.clone();
    app.until("the compact search to run", move |_| {
        find::status(&watched).starts_with("Searching")
    });
    app.settle();
    let _ = app::render(window);
    let hidden = inspect::find(w, AccessibleRole::TextInput, "File name masks")
        .and_then(|item| inspect::visible_rect(&item))
        .is_none_or(|(_, _, _, height)| height < 2.0);
    let (under, above) = status_places(window)?;
    app.screenshot_window(
        context,
        window,
        &format!("{name}-running"),
        "A compact window while searching",
    );
    tools::click_last(w, "Stop")?;
    let watched = window.clone();
    app.until("the compact search to stop", move |_| {
        find::status(&watched).starts_with("Cancelled")
    });
    app::layout_tool(window, find::SIZE.0, find::SIZE.1);
    app.settle();
    find::remove_root(app, window, &slow)?;
    if !hidden || !under || above {
        problems.push(format!(
            "while a compact search runs: query area hidden {hidden}, status under the results {under}, above them {above}"
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}
