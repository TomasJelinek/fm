//! The Find window where its behavior depends on size, density and theme: no clipped
//! or overlapping controls at its preferred size, narrow and compact (Blue, Dark and
//! Touch with large text); a narrow window stacks Remove under Add and puts Browse… and
//! OK under the typed path, and scrolls the result columns sideways with Left and Right;
//! a compact window names its field "Query", hides the query area while a search runs
//! and, with text larger than 20 px, shows the idle status above the results; a column
//! drag never sorts and its width is kept for the next search. The fitted footer: its
//! buttons and More… hold every action once, those in More… the first of their order,
//! each whole and 8 px from the next, at 1000, 560 and 320 px and with 32 px text at
//! 320×200; More… by pointer (an entry runs) and from the keyboard (Tab, Space, Enter,
//! Escape); the details page keeps Back.
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
    let mut config = Config::new(&root, &context.work);
    // More…'s Copy details reaches the in-process clipboard.
    config.clipboard = true;
    let mut app = App::start(config);
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
    findings.check(
        "the footer and More…",
        footer_more(&mut app, context, &window, &root),
    );
    find::close(&mut app, &window);
    app.shutdown()?;
    // The largest text Options offers (32 px) at the minimum size: the details page
    // keeps Back whole beside More….
    let store = SettingsStore::seeded(|preferences| preferences.desktop_font_size = 32);
    let mut config = Config::new(&root, &context.work);
    config.settings = Some(store);
    let mut app = App::start(config);
    let window = open_find(&mut app)?;
    findings.check(
        "the footer with 32 px text at 320×200",
        footer_large_text(&mut app, context, &window),
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

/// The query page's footer actions, in order, and the order in which they move into
/// More… when the footer is too narrow.
const QUERY_ACTIONS: [&str; 6] = ["Find", "Go to file", "View", "Edit", "Open", "Copy details"];
const QUERY_DROPS: [&str; 5] = ["Copy details", "Open", "Edit", "View", "Go to file"];
/// The details page's.
const DETAILS_ACTIONS: [&str; 3] = ["Find", "Copy", "Back"];
const DETAILS_DROPS: [&str; 2] = ["Copy", "Find"];

/// The fitted footer at `size`: the buttons and the actions in More… make up the page's
/// whole set once each; the moved ones are the first of `drops`; every button (More…
/// and Close included) is whole inside the window, 8 px from the next, and no layout
/// finding names one of them.
fn footer_fit(
    app: &mut App,
    context: &mut Context,
    window: &Tool,
    (width, height): (u32, u32),
    (actions, drops): (&[&str], &[&str]),
    name: &str,
) -> Result<tools::Footer, String> {
    app::layout_tool(window, width, height);
    app.settle();
    let w = window.window();
    let footer =
        tools::footer_actions(app, w, "Close").map_err(|error| format!("{name}: {error}"))?;
    app.screenshot_window(
        context,
        window,
        name,
        &format!("The footer at {width}×{height}"),
    );
    let shown = footer.labels();
    let moved = footer.menu_labels();
    let mut problems = Vec::new();
    let mut every: Vec<String> = shown.iter().chain(&moved).cloned().collect();
    every.sort();
    let mut wanted: Vec<String> = actions.iter().map(|label| (*label).to_owned()).collect();
    wanted.sort();
    if every != wanted {
        problems.push(format!(
            "buttons {shown:?} and More… {moved:?} are not the set {actions:?}"
        ));
    }
    let moved_set: std::collections::BTreeSet<&str> = moved.iter().map(String::as_str).collect();
    let first: std::collections::BTreeSet<&str> = drops.iter().take(moved.len()).copied().collect();
    if moved_set != first {
        problems.push(format!(
            "More… holds {moved:?}, not the first {} of {drops:?}",
            moved.len()
        ));
    }
    if moved.is_empty() != footer.more.is_none() {
        problems.push(format!(
            "More… shown {} with {moved:?} in it",
            footer.more.is_some()
        ));
    }
    let mut row: Vec<&tools::Action> = footer.buttons.iter().collect();
    row.extend(footer.more.as_ref());
    row.push(&footer.dismiss);
    row.sort_by(|a, b| a.rect.0.total_cmp(&b.rect.0));
    for action in &row {
        let (x, y, w, h) = action.rect;
        if x < -0.5 || y < -0.5 || x + w > width as f32 + 0.5 || y + h > height as f32 + 0.5 {
            problems.push(format!(
                "{:?} at {:?} is not whole in the window",
                action.label, action.rect
            ));
        }
    }
    for pair in row.windows(2) {
        let gap = pair[1].rect.0 - (pair[0].rect.0 + pair[0].rect.2);
        if (gap - 8.0).abs() > 0.75 {
            problems.push(format!(
                "{:?} and {:?} are {gap:.1} px apart, not 8",
                pair[0].label, pair[1].label
            ));
        }
    }
    let _ = app::render(window);
    let labels: Vec<&str> = row.iter().map(|action| action.label.as_str()).collect();
    for finding in inspect::check_layout(w, width as f32, height as f32) {
        // Findings quote the controls they name.
        if labels
            .iter()
            .any(|label| finding.detail.contains(&format!("{label:?}")))
        {
            problems.push(format!("{}: {}", finding.kind, finding.detail));
        }
    }
    if problems.is_empty() {
        Ok(footer)
    } else {
        Err(format!(
            "{name} ({width}×{height}): {}",
            problems.join("; ")
        ))
    }
}

/// The footer fits its actions: every one a button at 1000 px, some in More… at 560 px,
/// only Find and More… at 320×200, whole and 8 px apart; More… opens from the keyboard
/// (Tab to it, Space) and Enter runs its first action (Go to file), Escape closes it, a
/// click on an entry runs it (Copy details); the details page keeps Back.
fn footer_more(
    app: &mut App,
    context: &mut Context,
    window: &Tool,
    root: &Path,
) -> Result<(), String> {
    let w = window.window();
    app::layout_tool(window, find::SIZE.0, find::SIZE.1);
    app.settle();
    // The compact check left a content search in the form: back to every name.
    find::fill(app, window, BASE)?;
    let text = find::run(app, window)?;
    if !text.starts_with("Completed") {
        return Err(format!("*.* ended {text:?}"));
    }
    // The first file result: Go to file from More… must bring the panel to it.
    let rows = find::all_rows(app, window)?;
    let chosen = rows
        .iter()
        .find(|row| row.name.ends_with(".txt") && row.size != "<DIR>")
        .cloned()
        .ok_or_else(|| format!("no file among the results {:?}", find::ordered(&rows, root)))?;
    let name = chosen.name.as_str();
    find::select_result(app, window, name, Some(Path::new(&chosen.folder)))?;
    let mut problems = Vec::new();
    let query = (&QUERY_ACTIONS[..], &QUERY_DROPS[..]);
    match footer_fit(app, context, window, (1000, 620), query, "footer-1000") {
        Ok(footer) if footer.more.is_some() => problems.push(format!(
            "at 1000 px More… holds {:?}; every action should be a button",
            footer.menu_labels()
        )),
        Ok(_) => {}
        Err(error) => problems.push(error),
    }
    match footer_fit(app, context, window, (560, 420), query, "footer-560") {
        // A click on an entry of More… runs it: Copy details copies the search's status.
        Ok(footer) => match &footer.more {
            Some(more) if footer.menu_labels().contains(&"Copy details".to_owned()) => {
                // Copy details copies the search's progress: the status line without its
                // selection part (" · 1 selected · 7 B").
                let status = find::status(window);
                let line = status
                    .find(" selected")
                    .and_then(|end| status[..end].rfind(" · "))
                    .map_or(status.clone(), |start| status[..start].to_owned());
                let before = tools::clipboard_writes(app);
                let (x, y, width, height) = more.rect;
                input::click_at(w, LogicalPosition::new(x + width / 2.0, y + height / 2.0));
                app.settle();
                match tools::choose(app, w, "Copy details")
                    .and_then(|()| tools::wait_copied(app, before, "More…'s Copy details"))
                {
                    Ok(text) if text.starts_with(&line) => {}
                    Ok(text) => problems.push(format!(
                        "More…'s Copy details copied {text:?}, not the status {line:?} first"
                    )),
                    Err(error) => problems.push(error),
                }
            }
            _ => problems.push(format!(
                "at 560×420 More… does not hold Copy details: buttons {:?}, More… {:?}",
                footer.labels(),
                footer.menu_labels()
            )),
        },
        Err(error) => problems.push(error),
    }
    let small = footer_fit(app, context, window, (320, 200), query, "footer-320x200");
    match &small {
        Ok(footer) if footer.labels() != ["Find"] || footer.more.is_none() => {
            problems.push(format!(
                "at 320×200 the footer shows {:?} and More… {:?}, not Find and More…",
                footer.labels(),
                footer.menu_labels()
            ))
        }
        Ok(_) => {}
        Err(error) => problems.push(error.clone()),
    }
    // The keyboard: Tab to More…, Space opens it, Enter runs its first action, which with
    // a result under the cursor is Go to file.
    if let Ok(footer) = &small
        && let Some(more) = &footer.more
    {
        let mut reached = false;
        for _ in 0..40 {
            if tools::focus_in(w, more.rect) {
                reached = true;
                break;
            }
            input::key(w, Key::Tab);
            app.settle();
        }
        if !reached {
            problems.push("Tab never reached More… at 320×200".into());
        } else {
            input::key(w, " ");
            app.settle();
            if !tools::popup_open(w) {
                problems.push("Space on More… opened no menu".into());
            } else {
                input::key(w, Key::Return);
                app.settle();
                let wanted = std::path::PathBuf::from(&chosen.folder);
                let target = name.to_owned();
                let moved = (|| {
                    let deadline = std::time::Instant::now() + Duration::from_secs(15);
                    loop {
                        app.settle();
                        let names = app.names(midnight_core::PaneId::Left);
                        let at = usize::try_from(app.cursor(midnight_core::PaneId::Left))
                            .ok()
                            .and_then(|row| names.get(row).cloned());
                        if app.location(midnight_core::PaneId::Left).as_deref()
                            == Some(wanted.as_path())
                            && at.as_deref() == Some(target.as_str())
                        {
                            return true;
                        }
                        if std::time::Instant::now() >= deadline {
                            return false;
                        }
                        app.idle(Duration::from_millis(20));
                    }
                })();
                if !moved || tools::popup_open(w) {
                    problems.push(format!(
                        "Enter in More… did not run Go to file: the panel shows {:?}, the menu open {}",
                        app.location(midnight_core::PaneId::Left),
                        tools::popup_open(w)
                    ));
                }
            }
        }
        // Escape closes the menu and leaves the window.
        input::click_at(
            w,
            LogicalPosition::new(
                more.rect.0 + more.rect.2 / 2.0,
                more.rect.1 + more.rect.3 / 2.0,
            ),
        );
        app.settle();
        input::key(w, Key::Escape);
        app.settle();
        if tools::popup_open(w) || !w.is_visible() {
            problems.push(format!(
                "Escape in More…: the menu open {}, the window shown {}",
                tools::popup_open(w),
                w.is_visible()
            ));
        }
    }
    // The details page keeps Back; Find and Copy move into More….
    app::layout_tool(window, find::SIZE.0, find::SIZE.1);
    app.settle();
    let line = find::status(window);
    match tools::control(w, AccessibleRole::Button, &line) {
        Ok(item) => {
            input::click_at(w, inspect::center(&item));
            app.settle();
            let details = (&DETAILS_ACTIONS[..], &DETAILS_DROPS[..]);
            match footer_fit(app, context, window, (320, 200), details, "details-320x200") {
                Ok(footer) if !footer.labels().contains(&"Back".to_owned()) => problems.push(
                    format!("the details page at 320×200 shows {:?}", footer.labels()),
                ),
                Ok(_) => {}
                Err(error) => problems.push(error),
            }
            app::layout_tool(window, find::SIZE.0, find::SIZE.1);
            app.settle();
            input::click_button(w, "Back")?;
            app.settle();
        }
        Err(error) => problems.push(format!("the status line: {error}")),
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// 32 px text at 320×200: the query page shows Find and More…, the details page Back and
/// More…, each whole and 8 px apart (on Linux Back used to be cut there).
fn footer_large_text(app: &mut App, context: &mut Context, window: &Tool) -> Result<(), String> {
    let w = window.window();
    let text = find::run(app, window)?;
    if !text.starts_with("Completed") {
        return Err(format!("*.* ended {text:?}"));
    }
    let mut problems = Vec::new();
    match footer_fit(
        app,
        context,
        window,
        (320, 200),
        (&QUERY_ACTIONS[..], &QUERY_DROPS[..]),
        "large-text-320x200",
    ) {
        Ok(footer) if footer.labels() != ["Find"] => problems.push(format!(
            "with 32 px text at 320×200 the footer shows {:?}",
            footer.labels()
        )),
        Ok(_) => {}
        Err(error) => problems.push(error),
    }
    app::layout_tool(window, find::SIZE.0, find::SIZE.1);
    app.settle();
    let line = find::status(window);
    let item = tools::control(w, AccessibleRole::Button, &line)?;
    input::click_at(w, inspect::center(&item));
    app.settle();
    match footer_fit(
        app,
        context,
        window,
        (320, 200),
        (&DETAILS_ACTIONS[..], &DETAILS_DROPS[..]),
        "large-text-details-320x200",
    ) {
        Ok(footer) if footer.labels() != ["Back"] => problems.push(format!(
            "with 32 px text the details page at 320×200 shows {:?}",
            footer.labels()
        )),
        Ok(_) => {}
        Err(error) => problems.push(error),
    }
    app::layout_tool(window, find::SIZE.0, find::SIZE.1);
    app.settle();
    input::click_button(w, "Back")?;
    app.settle();
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
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
    // A layout finding is reported with the compact checks below, not instead of them.
    let mut problems = Vec::new();
    if let Err(problem) = layout_clean(app, context, window, size, name) {
        problems.push(problem);
    }
    // The label may be scrolled out of the short query area: look at every text.
    let labels = all_texts(window);
    if !labels.iter().any(|text| text == "Query") || labels.iter().any(|text| text == "File name") {
        problems.push(format!(
            "a compact window does not call the field Query: {labels:?}"
        ));
        return Err(problems.join("\n"));
    }
    // An idle status: in the query area with large text, under the results otherwise.
    let text = find::run(app, window)?;
    app.settle();
    let (under, above) = status_places(window)?;
    if (under, above) != (!large_text, large_text) {
        problems.push(format!(
            "compact, large text {large_text}: after {text:?} a status under the results {under}, above them {above}"
        ));
        return Err(problems.join("\n"));
    }
    // Shown there, not only placed there: the user sees how the search ended.
    let _ = app::render(window);
    app.screenshot_window(
        context,
        window,
        &format!("{name}-idle"),
        "A compact window after a search",
    );
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
