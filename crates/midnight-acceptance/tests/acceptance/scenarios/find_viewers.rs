//! Viewers opened from Find through the installed viewers (a staged process, see
//! `viewers_stage`): View, F3 and Alt+F3 in the results open the selected file (Markdown
//! rendered, or its plain text), and only that file: Space does not walk to another one;
//! a viewer keeps the content it captured when the file changes afterwards (a new View
//! of the changed result is refused);
//! F3 and Alt+F3 in a fed list open the focused result, and Space and Backspace walk the
//! list's results in its order.
use crate::{
    Context,
    app::{App, Config},
    input,
    scenarios::find_support::{
        self as find, BASE, Findings, Query, feed_cursor, feed_names, feed_view, search,
        select_result,
    },
    tools_support as tools, viewers_stage,
    viewers_support::{self as support, Viewing},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::platform::{Key, WindowAdapter};
use std::{
    path::Path,
    time::{Duration, SystemTime},
};

pub const TIMEOUT: Duration = Duration::from_secs(10 * 60);
const OPEN: Duration = Duration::from_secs(30);
const STEP: Duration = Duration::from_secs(10);

/// The files and the text each viewer must show.
const FILES: [(&str, &str); 4] = [
    ("notes.md", "Find notes"),
    ("other.txt", "other viewer text"),
    ("plain.txt", "plain viewer text"),
    ("sub/third.txt", "third viewer text"),
];

pub fn viewers(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

fn tree(root: &Path) -> Result<(), String> {
    let now = SystemTime::now();
    for (path, text) in FILES {
        let data = if path.ends_with(".md") {
            format!("# {text}\n\nSome *Markdown* text.\n")
        } else {
            format!("{text}\n")
        };
        tools::write_at(&root.join(path), data.as_bytes(), now)?;
    }
    Ok(())
}

fn run(context: &mut Context) -> Result<(), String> {
    let root = context.work.join("view");
    tree(&root)?;
    let mut config = Config::new(&root, &context.work);
    config.plugins = Some(context.work.join("plugins"));
    let mut app = App::start(config);
    let mut findings = Findings::default();
    findings.check("from the results", from_results(&mut app, context, &root));
    find::reset(&mut app);
    findings.check("from a fed list", from_feed(&mut app, context, &root));
    find::reset(&mut app);
    app.shutdown()?;
    context.note("View, F3 and Alt+F3 from Find results and F3, Alt+F3, Space and Backspace from a fed list, through the installed viewers");
    findings.into_result()
}

/// Wait for the next viewer and for `text` in it; `markdown` is the rendering wanted.
fn opened(
    app: &mut App,
    before: usize,
    how: &str,
    text: &str,
    markdown: bool,
) -> Result<Viewing, String> {
    let viewing =
        support::next_viewer(app, before, OPEN).map_err(|error| format!("{how}: {error}"))?;
    if let Err(error) = &viewing.result {
        return Err(format!("{how}: the viewer reported {error}"));
    }
    let wanted = text.to_owned();
    viewing.wait(
        app,
        &format!("{how} to show {text:?}"),
        STEP,
        move |window| {
            window.get_accessible_text().contains(wanted.as_str())
                && window.get_markdown_mode() == markdown
        },
    )?;
    Ok(viewing)
}

fn from_results(app: &mut App, context: &mut Context, root: &Path) -> Result<(), String> {
    let window = find::open_find(app)?;
    let w = window.window();
    let (text, rows) = search(
        app,
        &window,
        Query {
            masks: "*.txt;*.md",
            ..BASE
        },
    )?;
    if !text.starts_with("Completed") || rows.len() != FILES.len() {
        return Err(format!(
            "*.txt;*.md ended {text:?} with {} rows",
            rows.len()
        ));
    }
    // View: the selected text file, and only it (Space walks no further).
    select_result(app, &window, "plain.txt", None)?;
    let before = support::viewers_opened(app);
    tools::click_last(w, "View")?;
    let viewing = opened(app, before, "View on plain.txt", "plain viewer text", false)?;
    let caption = viewing.window.get_caption().to_string();
    app.screenshot_window(
        context,
        &viewing.software,
        "view-result",
        &format!("View on a Find result: {caption}"),
    );
    // The viewer keeps what it captured: the file changes on disk, a new View of the
    // result is refused (the file is no longer the one found), and the open viewer still
    // shows the captured text, not the new one.
    let plain = root.join("plain.txt");
    tools::write_at(&plain, b"changed viewer text, longer\n", SystemTime::now())?;
    let viewers_before = support::viewers_opened(app);
    tools::click_last(w, "View")?;
    find::wait_error(app, &window, "View of the changed result", |text| {
        text.contains("plain.txt") || text.contains("changed")
    })?;
    let kept = viewing.text();
    let reopened = support::viewers_opened(app) != viewers_before;
    tools::write_at(&plain, b"plain viewer text\n", SystemTime::now())?;
    if !kept.contains("plain viewer text") || kept.contains("changed viewer text") || reopened {
        viewing.close(app)?;
        return Err(format!(
            "after plain.txt changed, the open viewer shows {kept:?} and a new View opened a viewer: {reopened}"
        ));
    }
    // The viewer's tooltips are text-mode Tooltips (styled text): the harness reads them.
    input::chord(viewing.slint(), &[Key::Control], "f");
    app.settle();
    let next = tools::control(viewing.slint(), AccessibleRole::Button, "Next match")?;
    let tips = tools::hover_tooltip(app, viewing.slint(), &next);
    if !tips.iter().any(|tip| tip == "Next match (F3)") {
        viewing.close(app)?;
        return Err(format!(
            "hovering the viewer's Next match shows the tooltip texts {tips:?}, not \"Next match (F3)\""
        ));
    }
    viewing.click_content(app);
    input::key(viewing.slint(), Key::Space);
    app.idle(Duration::from_millis(500));
    let after = (viewing.window.get_caption().to_string(), viewing.text());
    viewing.close(app)?;
    if !caption.contains("plain.txt")
        || after.0 != caption
        || !after.1.contains("plain viewer text")
    {
        return Err(format!(
            "the viewer of a Find result: caption {caption:?}; after Space {:?}",
            after.0
        ));
    }
    // F3 renders Markdown; Alt+F3 shows its source.
    select_result(app, &window, "notes.md", None)?;
    let before = support::viewers_opened(app);
    input::key(w, Key::F3);
    opened(app, before, "F3 on notes.md", "Find notes", true)?.close(app)?;
    let before = support::viewers_opened(app);
    input::chord(w, &[Key::Alt], Key::F3);
    let viewing = opened(app, before, "Alt+F3 on notes.md", "# Find notes", false)?;
    app.screenshot_window(
        context,
        &viewing.software,
        "plain-result",
        "Alt+F3 on a Find result: the Markdown source",
    );
    viewing.close(app)?;
    find::close(app, &window);
    Ok(())
}

fn from_feed(app: &mut App, context: &mut Context, root: &Path) -> Result<(), String> {
    let window = find::open_find(app)?;
    let (text, _) = search(
        app,
        &window,
        Query {
            masks: "*.txt;*.md",
            ..BASE
        },
    )?;
    if !text.starts_with("Completed") {
        return Err(format!("*.txt;*.md ended {text:?}"));
    }
    find::wait_names(
        app,
        &window,
        &["notes.md", "other.txt", "plain.txt", "third.txt"],
        "Name ascending",
    )?;
    tools::click_last(window.window(), "Feed to panel")?;
    app.until("the results list", |app| feed_view(app, PaneId::Left).open);
    // Name order: notes.md, other.txt, plain.txt, third.txt.
    let names = feed_names(app, PaneId::Left);
    let order: Vec<&str> = FILES
        .iter()
        .map(|(path, _)| path.rsplit('/').next().unwrap_or(path))
        .collect();
    if names != order {
        return Err(format!("the fed list shows {names:?}"));
    }
    input::key(app.slint(), Key::Home);
    input::key(app.slint(), Key::DownArrow);
    app.settle();
    if feed_cursor(app, PaneId::Left).as_deref() != Some("other.txt") {
        return Err(format!(
            "the list's cursor is on {:?}",
            feed_cursor(app, PaneId::Left)
        ));
    }
    let before = support::viewers_opened(app);
    input::key(app.slint(), Key::F3);
    let viewing = opened(
        app,
        before,
        "F3 in the fed list",
        "other viewer text",
        false,
    )?;
    // Space and Backspace walk the list's results.
    viewing.click_content(app);
    input::key(viewing.slint(), Key::Space);
    viewing.wait(app, "Space to show the next result", STEP, |window| {
        window.get_accessible_text().contains("plain viewer text")
    })?;
    input::key(viewing.slint(), Key::Space);
    viewing.wait(app, "Space to show the result in sub/", STEP, |window| {
        window.get_accessible_text().contains("third viewer text")
    })?;
    app.screenshot_window(
        context,
        &viewing.software,
        "feed-walk",
        "Space walked the fed results to sub/third.txt",
    );
    input::key(viewing.slint(), Key::Backspace);
    viewing.wait(
        app,
        "Backspace to show the previous result",
        STEP,
        |window| window.get_accessible_text().contains("plain viewer text"),
    )?;
    viewing.close(app)?;
    // Alt+F3 on the Markdown result: its source.
    input::key(app.slint(), Key::Home);
    app.settle();
    let before = support::viewers_opened(app);
    input::chord(app.slint(), &[Key::Alt], Key::F3);
    opened(app, before, "Alt+F3 in the fed list", "# Find notes", false)?.close(app)?;
    input::key(app.slint(), Key::Backspace);
    let root = root.to_path_buf();
    app.until("the list to close", move |app| {
        !feed_view(app, PaneId::Left).open
            && app.location(PaneId::Left).as_deref() == Some(root.as_path())
    });
    Ok(())
}
