//! Shift+Backspace (and Ctrl+Backspace) to the root of the focused panel, browsing
//! through the installed native filesystem plugin as the application does (a staged
//! process): from a deep folder in the left and in the right panel, each focused in
//! turn (the other panel stays); after a filename search ended with Escape; after the
//! panel was focused by a mouse click; in a second tab (the first tab stays); and
//! inside an archive, from a folder nested in it, where the root is the archive's own
//! root, not the filesystem's.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support},
    input, panel,
    scenarios::panels_support::{self as ps, LEFT, RIGHT},
    viewers_stage,
};
use midnight_core::PaneId;
use slint::{ComponentHandle, Model, platform::Key};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};

pub const TIMEOUT: Duration = Duration::from_secs(8 * 60);
const LIST_LIMIT: Duration = Duration::from_secs(30);

pub fn root_keys(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, body)
}

fn path_bar(app: &App, pane: PaneId) -> String {
    match pane {
        PaneId::Left => app.window.get_left_path(),
        PaneId::Right => app.window.get_right_path(),
    }
    .to_string()
}

fn reached(app: &mut App, pane: PaneId, path: &Path) -> bool {
    let wanted = path.to_path_buf();
    ps::wait(app, LIST_LIMIT, |app| {
        app.location(pane).as_ref() == Some(&wanted) && !app.loading(pane)
    })
}

fn go(app: &mut App, pane: PaneId, path: &Path) -> Result<(), String> {
    panel::focus_list(app, pane)?;
    input::chord(app.slint(), &[Key::Control], "g");
    app.settle();
    input::replace_text(app.slint(), &path.to_string_lossy());
    input::key(app.slint(), Key::Return);
    if !reached(app, pane, path) {
        return Err(format!(
            "Ctrl+G did not open {} in the {pane:?} panel",
            path.display()
        ));
    }
    Ok(())
}

fn shift_backspace(app: &mut App) {
    input::chord(app.slint(), &[Key::Shift], Key::Backspace);
    app.settle();
}

/// Shift+Backspace in `pane` must show the filesystem root there and leave the other
/// panel where it was.
fn expect_root(app: &mut App, pane: PaneId, what: &str, failures: &mut Vec<String>) {
    let other = match pane {
        PaneId::Left => RIGHT,
        PaneId::Right => LEFT,
    };
    let other_before = app.location(other);
    let before = app.location(pane);
    shift_backspace(app);
    if !reached(app, pane, Path::new("/")) {
        failures.push(format!(
            "{what}: Shift+Backspace left the {pane:?} panel at {:?} (from {before:?}), expected /",
            app.location(pane)
        ));
    }
    if app.location(other) != other_before {
        failures.push(format!(
            "{what}: Shift+Backspace also moved the {other:?} panel to {:?}",
            app.location(other)
        ));
    }
}

fn body(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let deep = root.join("deep/level-01/level-02/level-03");
    let mut config = Config::new(&deep, &deep);
    config.plugins = Some(context.work.join("plugins"));
    let mut app = App::start(config);
    let mut failures = Vec::new();

    // The left panel focused by the keyboard.
    panel::focus_list(&mut app, LEFT)?;
    expect_root(&mut app, LEFT, "left panel, deep folder", &mut failures);
    // The right panel focused with Tab.
    go(&mut app, LEFT, &deep)?;
    panel::focus_list(&mut app, LEFT)?;
    input::key(app.slint(), Key::Tab);
    app.settle();
    if app.window.get_focused_pane() != 1 {
        failures.push("Tab did not focus the right panel".into());
    }
    expect_root(&mut app, RIGHT, "right panel focused by Tab", &mut failures);
    // Ctrl+Backspace, the other default, in the right panel.
    go(&mut app, RIGHT, &deep)?;
    input::chord(app.slint(), &[Key::Control], Key::Backspace);
    if !reached(&mut app, RIGHT, Path::new("/")) {
        failures.push(format!(
            "Ctrl+Backspace left the right panel at {:?}",
            app.location(RIGHT)
        ));
    }
    // The right panel focused by a mouse click on one of its rows.
    go(&mut app, RIGHT, &deep)?;
    panel::focus_list(&mut app, LEFT)?;
    let row = ps::shown(&app, RIGHT)
        .into_iter()
        .next()
        .ok_or("the deep folder lists nothing")?;
    let position = panel::row_center(&app, RIGHT, &row)
        .ok_or_else(|| format!("no {row} row in the right panel"))?;
    panel::click(&mut app, position);
    app.settle();
    if app.window.get_focused_pane() != 1 {
        failures.push("a click on a right-panel row did not focus it".into());
    }
    expect_root(
        &mut app,
        RIGHT,
        "right panel focused by a click",
        &mut failures,
    );
    // After a filename search ended with Escape.
    go(&mut app, LEFT, &deep)?;
    panel::focus_list(&mut app, LEFT)?;
    input::type_text(app.slint(), "l");
    app.settle();
    if !app.window.get_left_search_active() {
        failures.push("typing l did not start a filename search".into());
    }
    input::key(app.slint(), Key::Escape);
    app.settle();
    if app.window.get_left_search_active() {
        failures.push("Escape did not end the filename search".into());
    }
    expect_root(
        &mut app,
        LEFT,
        "left panel after a search ended with Escape",
        &mut failures,
    );
    // During an active search (recorded: the catalog gives Shift+Backspace to the root
    // in filename search too).
    go(&mut app, LEFT, &deep)?;
    panel::focus_list(&mut app, LEFT)?;
    input::type_text(app.slint(), "l");
    app.settle();
    shift_backspace(&mut app);
    ps::wait(&mut app, Duration::from_secs(3), |app| {
        app.location(LEFT).as_deref() == Some(Path::new("/"))
    });
    context.note(format!(
        "Shift+Backspace during the search \"l\": left panel at {:?}, search {:?} active {}",
        app.location(LEFT),
        app.window.get_left_search_query(),
        app.window.get_left_search_active()
    ));
    if app.window.get_left_search_active() {
        input::key(app.slint(), Key::Escape);
        app.settle();
    }
    // In a second tab; the first tab stays.
    go(&mut app, LEFT, &root.join("docs"))?;
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "t");
    ps::wait(&mut app, Duration::from_secs(5), |app| {
        app.window.get_left_tabs().iter().count() == 2
    });
    go(&mut app, LEFT, &deep)?;
    expect_root(&mut app, LEFT, "a second tab", &mut failures);
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control, Key::Shift], Key::Tab);
    if !reached(&mut app, LEFT, &root.join("docs")) {
        failures.push(format!(
            "Shift+Backspace in the second tab changed the first tab: it shows {:?}",
            app.location(LEFT)
        ));
    }
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "w");
    app.settle();

    // The path bar check holds for a native folder (its control).
    go(&mut app, LEFT, &deep)?;
    if !bar_shows(&app, "level-03") {
        failures.push(format!(
            "control: in a native deep folder the path bar does not show level-03 whole: {:?}",
            bar_texts(&app)
        ));
    }
    // Inside an archive: from bundle.zip/bundle/nested/deeper to the archive's root.
    archive(&mut app, context, &root, LEFT, &mut failures)?;
    archive(&mut app, context, &root, RIGHT, &mut failures)?;
    // This staged process has the viewers: Make File List's Internal viewer output.
    file_list_viewer(&mut app, context, &root, &mut failures)?;
    app.shutdown()?;
    support::report(&failures)
}

fn shown(app: &App, pane: PaneId) -> BTreeSet<String> {
    ps::shown(app, pane).into_iter().collect()
}

fn listing(app: &mut App, pane: PaneId, wanted: &[&str]) -> bool {
    let wanted: BTreeSet<String> = wanted.iter().map(|name| (*name).to_owned()).collect();
    ps::wait(app, LIST_LIMIT, |app| {
        !app.loading(pane) && shown(app, pane) == wanted
    })
}

fn open(app: &mut App, pane: PaneId, name: &str) -> Result<(), String> {
    panel::select_row(app, pane, name)?;
    input::key(app.slint(), Key::Return);
    app.settle();
    Ok(())
}

fn archive(
    app: &mut App,
    context: &mut Context,
    root: &Path,
    pane: PaneId,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let home: PathBuf = root.join("archives");
    go(app, pane, &home)?;
    open(app, pane, "bundle.zip")?;
    if !listing(app, pane, &["bundle"]) {
        return Err(format!(
            "bundle.zip did not open in the {pane:?} panel: it shows {:?}, error {:?}",
            shown(app, pane),
            app.error(pane)
        ));
    }
    for (folder, inside) in [
        (
            "bundle",
            &["empty", "nested", "readme.txt", "ünïcödé 🦀.txt"][..],
        ),
        ("nested", &["deeper", "notes.md"][..]),
        ("deeper", &["data.bin"][..]),
    ] {
        open(app, pane, folder)?;
        if !listing(app, pane, inside) {
            return Err(format!(
                "bundle.zip's {folder} shows {:?} in the {pane:?} panel",
                shown(app, pane)
            ));
        }
    }
    let deep_bar = path_bar(app, pane);
    if pane == LEFT && !bar_shows(app, "deeper") {
        failures.push(format!(
            "inside bundle.zip/bundle/nested/deeper the path bar does not show \"deeper\" whole (it holds {deep_bar:?}; shown texts {:?})",
            bar_texts(app)
        ));
    }
    shift_backspace(app);
    let at_root = listing(app, pane, &["bundle"]);
    let bar = path_bar(app, pane);
    let location = app.location(pane);
    if !at_root
        || location.as_deref() == Some(Path::new("/"))
        || !bar.trim_end_matches('/').ends_with("bundle.zip")
    {
        failures.push(format!(
            "Shift+Backspace in bundle.zip/bundle/nested/deeper ({pane:?} panel, path bar {deep_bar:?}) went to {location:?} showing {:?} with the path bar {bar:?}, expected the archive's root (bundle)",
            shown(app, pane)
        ));
    } else {
        app.screenshot(
            context,
            &format!("archive-root-{pane:?}").to_lowercase(),
            "Shift+Backspace inside bundle.zip: the archive's root",
        );
    }
    if pane == LEFT && !bar_shows(app, "bundle.zip") {
        failures.push(format!(
            "at bundle.zip's root the path bar does not show \"bundle.zip\" whole (it holds {bar:?}; shown texts {:?}): a long archive path is cut at its end instead of in its middle",
            bar_texts(app)
        ));
    }
    // Leave the archive.
    panel::focus_list(app, pane)?;
    for _ in 0..4 {
        if app.location(pane).as_deref() == Some(home.as_path()) && !app.loading(pane) {
            break;
        }
        input::key(app.slint(), Key::Backspace);
        app.settle();
        ps::wait(app, Duration::from_secs(10), |app| !app.loading(pane));
    }
    if !reached(app, pane, &home) {
        go(app, pane, &home)?;
    }
    Ok(())
}

/// The texts the left path bar shows: (text or accessible name, shown width, needed
/// width).
fn bar_texts(app: &App) -> Vec<(String, f32, f32)> {
    let (left, top) = (
        app.window.get_left_path_left(),
        app.window.get_left_path_top(),
    );
    let (width, height) = (
        app.window.get_left_path_width(),
        app.window.get_left_path_control_height(),
    );
    let mut found = Vec::new();
    for root in crate::inspect::roots(app.slint()).iter().take(1) {
        crate::inspect::walk(root, &mut |item| {
            let text = item
                .downcast::<i_slint_core::items::SimpleText>()
                .map(|text| text.as_pin_ref().text().to_string())
                .or_else(|| {
                    item.downcast::<i_slint_core::items::ComplexText>()
                        .map(|text| text.as_pin_ref().text().to_string())
                });
            let Some(text) = text else {
                return;
            };
            let Some((x, y, shown, _)) = crate::inspect::visible_rect(item) else {
                return;
            };
            if x >= left - 1.0 && x < left + width && y >= top - 1.0 && y < top + height {
                let name = crate::inspect::label(item).unwrap_or(text);
                let needed = crate::scenarios::panels_view_sort::preferred(item);
                found.push((name, shown, needed));
            }
        });
    }
    found
}

/// The left path bar shows `name` as a whole last component.
fn bar_shows(app: &App, name: &str) -> bool {
    bar_texts(app).iter().any(|(text, shown, needed)| {
        let text = text.trim_end_matches(std::path::MAIN_SEPARATOR);
        (text == name || text.ends_with(&format!("{}{name}", std::path::MAIN_SEPARATOR)))
            && *needed <= *shown + 1.0
    })
}

/// Ctrl+M with the Internal viewer output opens the generated list in the basic text
/// viewer, which shows exactly the list; Escape closes it.
fn file_list_viewer(
    app: &mut App,
    context: &mut Context,
    root: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    use crate::viewers_support;
    go(app, LEFT, &root.join("text"))?;
    for name in ["plain.txt", "utf8.txt"] {
        panel::select_row(app, LEFT, name)?;
        input::key(app.slint(), Key::Insert);
    }
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "m");
    if !ps::wait(app, Duration::from_secs(5), |app| {
        app.window
            .global::<midnight_ui::FileListPresentation>()
            .get_active()
    }) {
        failures.push("Ctrl+M opened no Make File List".into());
        return Ok(());
    }
    input::fill(app.slint(), "File list line template", "$(FileName)$(LF)")?;
    crate::options_support::choose(app, "File list output destination", "Internal viewer")?;
    let before = viewers_support::viewers_opened(app);
    input::click_button(app.slint(), "Create file list")?;
    match viewers_support::next_viewer(app, before, Duration::from_secs(30)) {
        Err(error) => failures.push(format!("Make File List to the Internal viewer: {error}")),
        Ok(viewing) => {
            let text = viewing.text();
            if !text.contains("plain.txt\nutf8.txt") {
                failures.push(format!(
                    "the Internal viewer shows {:?}, expected the list plain.txt, utf8.txt",
                    text.chars().take(200).collect::<String>()
                ));
            }
            app.screenshot(
                context,
                "file-list-viewer",
                "Make File List in the Internal viewer",
            );
            viewing.close(app)?;
        }
    }
    if app
        .window
        .global::<midnight_ui::FileListPresentation>()
        .get_active()
    {
        failures.push("Make File List stayed open after the Internal viewer opened".into());
        crate::files_support::dismiss_all(app);
    }
    Ok(())
}
