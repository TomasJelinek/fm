//! Selection beyond `files_selection`: Ctrl+=, Ctrl++ and Ctrl+− (no keypad needed)
//! with the remembered mask and Include folders; an invalid mask changes nothing and
//! says why; Ctrl+Shift+click adds a range; Alt+Down and Alt+Up visit the marked rows
//! without wrapping; Insert and Space stop at the last row; `..` is never marked; the
//! rubber band marks the rows it touches (plain replaces, Shift adds, Ctrl toggles);
//! the details strip counts the marked files and folders and their bytes after every
//! change, compared with the disk.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support},
    input, panel,
    scenarios::panels_support::{self as ps, LEFT},
};
use midnight_ui::PanelChoices;
use slint::{ComponentHandle, LogicalPosition, platform::Key};
use std::{path::Path, time::Duration};

const FILES: [(&str, usize); 6] = [
    ("a1.txt", 11),
    ("a2.txt", 22),
    ("b1.log", 33),
    ("b2.log", 44),
    ("c.dat", 55),
    ("d.dat", 66),
];
const FOLDERS: [&str; 2] = ["f1", "f2"];

fn mask_open(app: &App) -> bool {
    !app.window
        .global::<PanelChoices>()
        .get_mask_token()
        .is_empty()
}

/// The marks must be `wanted` and the details strip must count them, with the
/// files' bytes taken from the disk.
fn expect(app: &mut App, folder: &Path, what: &str, wanted: &[&str], failures: &mut Vec<String>) {
    let target = ps::sorted(wanted);
    if !ps::wait(app, Duration::from_secs(5), |app| {
        ps::marked(app, LEFT) == target
    }) {
        failures.push(format!(
            "{what}: marks {:?}, expected {target:?}",
            ps::marked(app, LEFT)
        ));
        return;
    }
    let files: Vec<&&str> = wanted
        .iter()
        .filter(|name| folder.join(name).is_file())
        .collect();
    let folders = wanted.len() - files.len();
    let bytes: u64 = files
        .iter()
        .map(|name| std::fs::metadata(folder.join(name)).map_or(0, |metadata| metadata.len()))
        .sum();
    let details = ps::details(app);
    let value = |label: &str| {
        details
            .iter()
            .find(|(field, _)| field == label)
            .map(|(_, value)| value.clone())
    };
    if wanted.is_empty() {
        if value("Selected").is_some() {
            failures.push(format!(
                "{what}: nothing is marked but the strip says {details:?}"
            ));
        }
        return;
    }
    let counted = format!(
        "{} {}, {folders} {}",
        files.len(),
        if files.len() == 1 { "file" } else { "files" },
        if folders == 1 { "folder" } else { "folders" }
    );
    // Updating values settle within moments.
    if !ps::wait(app, Duration::from_secs(5), |app| {
        ps::details(app)
            .iter()
            .any(|(field, shown)| field == "Selected" && *shown == counted)
    }) {
        failures.push(format!(
            "{what}: the details strip says {:?}, expected Selected {counted:?}",
            ps::details(app)
        ));
    }
    if !files.is_empty() {
        let shown = ps::details(app)
            .into_iter()
            .find(|(field, _)| field == "Selected file bytes")
            .map(|(_, value)| value);
        let digits = shown.as_deref().and_then(crate::tools_support::number);
        if digits != Some(bytes) {
            failures.push(format!(
                "{what}: the details strip shows {shown:?} for the marked files' {bytes} bytes"
            ));
        }
    }
}

/// Open the mask dialog with `modifiers`+`key`, check its kind and starting mask, type
/// `mask` (setting Include folders to `folders`) and press Enter or OK.
#[allow(clippy::too_many_arguments)] // Each argument is one expectation of the dialog.
fn mask(
    app: &mut App,
    key: &str,
    select: bool,
    starts: &str,
    starts_folders: bool,
    mask: &str,
    folders: bool,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], key);
    if !ps::wait(app, Duration::from_secs(5), mask_open) {
        failures.push(format!("Ctrl+{key} opened no mask dialog"));
        return Ok(());
    }
    let choices = app.window.global::<PanelChoices>();
    let shown = ps::value(
        app,
        i_slint_core::items::AccessibleRole::TextInput,
        "Selection mask",
    );
    if choices.get_mask_select() != select || shown.as_deref() != Some(starts) {
        failures.push(format!(
            "Ctrl+{key}: select {} with {shown:?}, expected select {select} starting from {starts:?}",
            choices.get_mask_select()
        ));
    }
    if ps::checked(app, "Include folders") != Some(starts_folders) {
        failures.push(format!(
            "Ctrl+{key}: Include folders starts {:?}, expected {starts_folders} (remembered with the mask)",
            ps::checked(app, "Include folders")
        ));
    }
    input::fill(app.slint(), "Selection mask", mask)?;
    if ps::checked(app, "Include folders") != Some(folders) {
        input::click(
            app.slint(),
            i_slint_core::items::AccessibleRole::Checkbox,
            "Include folders",
        )?;
        app.settle();
        input::click_button(app.slint(), "OK")?;
    } else {
        input::key(app.slint(), Key::Return);
    }
    app.settle();
    Ok(())
}

fn click(app: &mut App, name: &str, modifiers: &[Key]) -> Result<(), String> {
    panel::wait_input(app);
    let position = panel::row_center(app, LEFT, name).ok_or_else(|| format!("no row {name}"))?;
    app.idle(Duration::from_millis(650));
    input::click_with(app.slint(), modifiers, position);
    app.last_click = Some((std::time::Instant::now(), position));
    app.settle();
    Ok(())
}

pub fn selection_more(context: &mut Context) -> Result<(), String> {
    let folder = support::scratch(context, "selection-more")?;
    for (name, size) in FILES {
        ps::write(&folder.join(name), vec![b'x'; size])?;
    }
    for name in FOLDERS {
        ps::folder(&folder.join(name))?;
    }
    let mut app = App::start(Config::new(&folder, &context.work));
    let mut failures = Vec::new();

    // Ctrl+= starts from *, Include folders off; a*|*.dat... marks the matches.
    mask(
        &mut app,
        "=",
        true,
        "*",
        false,
        "a*;*.dat",
        false,
        &mut failures,
    )?;
    expect(
        &mut app,
        &folder,
        "Ctrl+= a*;*.dat",
        &["a1.txt", "a2.txt", "c.dat", "d.dat"],
        &mut failures,
    );
    // Ctrl++ selects too, from the remembered mask; with Include folders.
    mask(
        &mut app,
        "+",
        true,
        "a*;*.dat",
        false,
        "f1",
        true,
        &mut failures,
    )?;
    expect(
        &mut app,
        &folder,
        "Ctrl++ f1 with folders",
        &["a1.txt", "a2.txt", "c.dat", "d.dat", "f1"],
        &mut failures,
    );
    // Ctrl+− deselects, remembering the last mask and its Include folders.
    mask(
        &mut app,
        "-",
        false,
        "f1",
        true,
        "*.dat",
        false,
        &mut failures,
    )?;
    expect(
        &mut app,
        &folder,
        "Ctrl+− *.dat",
        &["a1.txt", "a2.txt", "f1"],
        &mut failures,
    );
    // Invalid masks (two |, nothing after |) change nothing and say why.
    mask(
        &mut app,
        "=",
        true,
        "*.dat",
        false,
        "*.dat",
        false,
        &mut failures,
    )?;
    expect(
        &mut app,
        &folder,
        "Ctrl+= *.dat again",
        &["a1.txt", "a2.txt", "c.dat", "d.dat", "f1"],
        &mut failures,
    );
    for invalid in ["a|b|c", "*.txt|"] {
        invalid_mask(&mut app, context, invalid, &mut failures)?;
    }
    support::dismiss_all(&mut app);
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "d");
    expect(&mut app, &folder, "Ctrl+D", &[], &mut failures);

    // Ctrl+Shift+click adds a range to the marks.
    click(&mut app, "a1.txt", &[Key::Control])?;
    click(&mut app, "b2.log", &[Key::Control])?;
    click(&mut app, "c.dat", &[Key::Control, Key::Shift])?;
    expect(
        &mut app,
        &folder,
        "Ctrl+click a1, b2, Ctrl+Shift+click c.dat",
        &["a1.txt", "b2.log", "c.dat"],
        &mut failures,
    );
    click(&mut app, "f1", &[])?;
    click(&mut app, "a2.txt", &[Key::Control, Key::Shift])?;
    expect(
        &mut app,
        &folder,
        "click f1, Ctrl+Shift+click a2.txt",
        &["a1.txt", "a2.txt", "b2.log", "c.dat", "f1", "f2"],
        &mut failures,
    );

    // Alt+Down and Alt+Up visit the marked rows in displayed order, without wrapping.
    let marked_order: Vec<String> = ps::shown(&app, LEFT)
        .into_iter()
        .filter(|name| ps::marked(&app, LEFT).contains(name))
        .collect();
    panel::select_row(&mut app, LEFT, "..")?;
    let mut visited = Vec::new();
    for _ in 0..marked_order.len() + 1 {
        input::chord(app.slint(), &[Key::Alt], Key::DownArrow);
        app.settle();
        visited.push(ps::cursor(&app, LEFT).unwrap_or_default());
    }
    let mut wanted = marked_order.clone();
    wanted.push(marked_order.last().cloned().unwrap_or_default());
    if visited != wanted {
        failures.push(format!(
            "Alt+Down from .. visited {visited:?}, expected {wanted:?}"
        ));
    }
    input::chord(app.slint(), &[Key::Alt], Key::UpArrow);
    app.settle();
    let previous = marked_order
        .get(marked_order.len().saturating_sub(2))
        .cloned();
    if ps::cursor(&app, LEFT) != previous {
        failures.push(format!(
            "Alt+Up from the last marked row went to {:?}, expected {previous:?}",
            ps::cursor(&app, LEFT)
        ));
    }
    let marks = ps::marked(&app, LEFT);
    if marks.len() != 6 {
        failures.push(format!("Alt+Up/Down changed the marks to {marks:?}"));
    }
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();

    // Insert and Space stop at the last row; `..` is never marked.
    panel::focus_list(&mut app, LEFT)?;
    input::key(app.slint(), Key::End);
    app.settle();
    let last = ps::cursor(&app, LEFT).unwrap_or_default();
    input::key(app.slint(), Key::Insert);
    app.settle();
    input::key(app.slint(), Key::Space);
    app.settle();
    if ps::cursor(&app, LEFT).as_deref() != Some(last.as_str()) {
        failures.push(format!(
            "Insert and Space at the last row moved the cursor to {:?}",
            ps::cursor(&app, LEFT)
        ));
    }
    expect(
        &mut app,
        &folder,
        "Insert then Space on the last row",
        &[],
        &mut failures,
    );
    input::key(app.slint(), Key::Home);
    app.settle();
    input::key(app.slint(), Key::Insert);
    app.settle();
    input::chord(app.slint(), &[Key::Control, Key::Shift], "a");
    app.settle();
    input::chord(app.slint(), &[Key::Control, Key::Shift], "i");
    app.settle();
    if app
        .rows(LEFT)
        .iter()
        .any(|row| row.parent_entry && row.selected)
    {
        failures.push("`..` was marked by Insert, Ctrl+Shift+A or Ctrl+Shift+I".into());
    }
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();

    // The rubber band from the gutter beside the rows: plain replaces, Shift adds,
    // Ctrl toggles.
    rubber(&mut app, context, &folder, &mut failures)?;
    app.shutdown()?;
    support::report(&failures)
}

fn rubber(
    app: &mut App,
    context: &mut Context,
    folder: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let gutter = app.window.get_left_list_left() + app.window.get_left_list_grid_width() + 4.0;
    let row = |app: &App, name: &str| panel::row_center(app, LEFT, name);
    let (Some(a2), Some(b2), Some(c), Some(d)) = (
        row(app, "a2.txt"),
        row(app, "b2.log"),
        row(app, "c.dat"),
        row(app, "d.dat"),
    ) else {
        failures.push("the rubber band's rows are not shown".into());
        return Ok(());
    };
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    // Plain: a2 through b2.
    ps::rubber_band(
        app,
        LogicalPosition::new(gutter, a2.y),
        LogicalPosition::new(a2.x, b2.y),
        &[],
    );
    expect(
        app,
        folder,
        "rubber band a2–b2",
        &["a2.txt", "b1.log", "b2.log"],
        failures,
    );
    app.screenshot(context, "rubber-band", "Rubber band from the gutter");
    // Shift adds d.dat.
    ps::rubber_band(
        app,
        LogicalPosition::new(gutter, d.y),
        LogicalPosition::new(d.x, d.y + 2.0),
        &[Key::Shift],
    );
    expect(
        app,
        folder,
        "Shift rubber band on d.dat",
        &["a2.txt", "b1.log", "b2.log", "d.dat"],
        failures,
    );
    // Ctrl toggles b2 through c.
    ps::rubber_band(
        app,
        LogicalPosition::new(gutter, b2.y),
        LogicalPosition::new(b2.x, c.y),
        &[Key::Control],
    );
    expect(
        app,
        folder,
        "Ctrl rubber band on b2–c",
        &["a2.txt", "b1.log", "c.dat", "d.dat"],
        failures,
    );
    // Plain again replaces everything with c.dat.
    ps::rubber_band(
        app,
        LogicalPosition::new(gutter, c.y),
        LogicalPosition::new(c.x, c.y + 2.0),
        &[],
    );
    expect(
        app,
        folder,
        "plain rubber band on c.dat",
        &["c.dat"],
        failures,
    );
    Ok(())
}

/// Ctrl+= with an invalid mask: the marks stay and the reason is shown (in the dialog,
/// the panel's status, the status line, the details strip or a notice).
fn invalid_mask(
    app: &mut App,
    context: &mut Context,
    text: &str,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let before = ps::marked(app, LEFT);
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "=");
    if !ps::wait(app, Duration::from_secs(5), mask_open) {
        failures.push(format!("Ctrl+= for {text:?} opened no dialog"));
        return Ok(());
    }
    input::fill(app.slint(), "Selection mask", text)?;
    input::key(app.slint(), Key::Return);
    app.settle();
    let reason = |app: &App| {
        let mut places = vec![
            app.window
                .global::<PanelChoices>()
                .get_mask_error()
                .to_string(),
            app.window.get_left_status().to_string(),
            app.window.get_desktop_status().to_string(),
        ];
        places.extend(ps::details(app).into_iter().map(|(_, value)| value));
        places.extend(support::shown_notice(app).map(|(title, body)| format!("{title} {body}")));
        places
            .into_iter()
            .find(|text| text.to_lowercase().contains("mask") || text.contains('|'))
    };
    ps::wait(app, Duration::from_secs(2), |app| reason(app).is_some());
    if ps::marked(app, LEFT) != before {
        failures.push(format!(
            "the invalid mask {text:?} changed the marks to {:?}",
            ps::marked(app, LEFT)
        ));
    }
    let open = mask_open(app);
    match reason(app) {
        None => failures.push(format!(
            "the invalid mask {text:?} gave no reason anywhere: dialog open {open}, panel status {:?}, status line {:?}",
            app.window.get_left_status(),
            app.window.get_desktop_status()
        )),
        Some(reason) => context.note(format!("invalid mask {text:?}: {reason:?} (dialog open {open})")),
    }
    support::dismiss_all(app);
    Ok(())
}
