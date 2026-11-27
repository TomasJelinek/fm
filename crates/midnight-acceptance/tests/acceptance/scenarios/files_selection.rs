//! Selection with real keys and clicks: Insert, Space (with a folder size), Ctrl+A,
//! Ctrl+Shift+A, Ctrl+D, Ctrl+Shift+D, Ctrl+I, Ctrl+Shift+I (the plain key includes
//! folders, Shift acts on files only), Ctrl+click, Shift+click, select and
//! deselect by mask with the numeric keypad's + and − (and F10), the keypad's *
//! invert, and a Copy of a multi-selection.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, TransferSpec, io_error},
    input, panel,
};
use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_core::PaneId;
use midnight_ui::PanelChoices;
use slint::{ComponentHandle, platform::Key};
use std::{
    fs,
    time::{Duration, Instant},
};

const FILES: [&str; 6] = [".hidden", "a1.txt", "a2.txt", "b1.log", "b2.log", "c.dat"];
const FOLDERS: [&str; 2] = ["d1", "d2"];

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

fn strings(names: &[&str]) -> Vec<String> {
    sorted(names.iter().map(|name| (*name).to_owned()).collect())
}

/// Wait until the pane's selection is exactly `wanted`.
fn expect_selection(app: &mut App, failures: &mut Vec<String>, what: &str, wanted: &[&str]) {
    let wanted = strings(wanted);
    let expected = wanted.clone();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        app.settle();
        if sorted(support::selected_names(app, PaneId::Left)) == expected {
            return;
        }
        app.idle(Duration::from_millis(20));
    }
    failures.push(format!(
        "{what}: selection {:?}, expected {wanted:?}",
        sorted(support::selected_names(app, PaneId::Left))
    ));
}

/// A numeric keypad key as the native event loop delivers it: the window's physical
/// key hook resolves it before Slint sees the typed character. Synthetic Slint key
/// events carry only the character, which the number row produces as well.
fn keypad_key(app: &mut App, name: &str) -> Result<(), String> {
    panel::focus_list(app, PaneId::Left)?;
    if !app
        .window
        .invoke_physical_shortcut(name.into(), false, false, false, false)
    {
        return Err(format!("{name} was not handled in the file list"));
    }
    app.settle();
    Ok(())
}

pub(crate) fn mask_dialog_open(app: &App) -> bool {
    !app.window
        .global::<PanelChoices>()
        .get_mask_token()
        .is_empty()
}

fn checked(app: &App, label: &str) -> Option<bool> {
    crate::inspect::find(app.slint(), AccessibleRole::Checkbox, label)?
        .accessible_string_property(AccessibleStringProperty::Checked)
        .map(|value| value == "true")
}

/// In the open Select/Deselect files dialog: type the mask and press Enter in the
/// field, or click Include folders and then OK (Enter on a focused check box
/// toggles it, as on every check box).
pub(crate) fn apply_mask(app: &mut App, mask: &str, folders: bool) -> Result<(), String> {
    app.until("the Select/Deselect files dialog", mask_dialog_open);
    input::fill(app.slint(), "Selection mask", mask)?;
    if checked(app, "Include folders") == Some(folders) {
        input::key(app.slint(), Key::Return);
    } else {
        input::click(app.slint(), AccessibleRole::Checkbox, "Include folders")?;
        app.settle();
        if checked(app, "Include folders") != Some(folders) {
            return Err(format!("Include folders did not become {folders}"));
        }
        input::click_button(app.slint(), "OK")?;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        app.settle();
        if !mask_dialog_open(app) {
            return Ok(());
        }
        app.idle(Duration::from_millis(20));
    }
    let field = crate::inspect::find(app.slint(), AccessibleRole::TextInput, "Selection mask")
        .and_then(|item| item.accessible_string_property(AccessibleStringProperty::Value));
    Err(format!(
        "the mask dialog stayed open after {mask:?} (field {field:?}, error {:?})",
        app.window.global::<PanelChoices>().get_mask_error()
    ))
}

fn keypad(app: &mut App, context: &mut Context, failures: &mut Vec<String>) -> Result<(), String> {
    let pane = PaneId::Left;
    panel::focus_list(app, pane)?;
    input::chord(app.slint(), &[Key::Control], "d");
    expect_selection(app, failures, "Ctrl+D before the keypad", &[]);

    // Num +: Select files, starting from "*"; masks as in the panel filter.
    keypad_key(app, "NumpadAdd")?;
    app.until("Select files", mask_dialog_open);
    let choices = app.window.global::<PanelChoices>();
    if !choices.get_mask_select() || choices.get_mask_pattern() != "*" {
        failures.push(format!(
            "Num + opened select {} with {:?}",
            choices.get_mask_select(),
            choices.get_mask_pattern()
        ));
    }
    app.screenshot(context, "keypad-plus", "Num +: Select files by mask");
    apply_mask(app, "*.TXT;*.dat", false)?;
    expect_selection(
        app,
        failures,
        "Num + *.TXT;*.dat",
        &["a1.txt", "a2.txt", "c.dat"],
    );

    // Num −: Deselect files, starting from the last mask.
    keypad_key(app, "NumpadSubtract")?;
    app.until("Deselect files", mask_dialog_open);
    let choices = app.window.global::<PanelChoices>();
    if choices.get_mask_select() || choices.get_mask_pattern() != "*.TXT;*.dat" {
        failures.push(format!(
            "Num − opened select {} with {:?}",
            choices.get_mask_select(),
            choices.get_mask_pattern()
        ));
    }
    apply_mask(app, "a2*", false)?;
    expect_selection(app, failures, "Num − a2*", &["a1.txt", "c.dat"]);

    // Folders take part only when chosen; then Num * inverts folders too.
    keypad_key(app, "NumpadAdd")?;
    apply_mask(app, "d*|*.log", true)?;
    expect_selection(
        app,
        failures,
        "Num + d*|*.log with folders",
        &["a1.txt", "c.dat", "d1", "d2"],
    );
    keypad_key(app, "NumpadMultiply")?;
    expect_selection(
        app,
        failures,
        "Num * with folders",
        &[".hidden", "a2.txt", "b1.log", "b2.log"],
    );
    keypad_key(app, "NumpadAdd")?;
    apply_mask(app, "*.log", false)?;
    keypad_key(app, "NumpadMultiply")?;
    expect_selection(app, failures, "Num * files only", &["a1.txt", "c.dat"]);

    // Escape closes the dialog without selecting anything.
    keypad_key(app, "NumpadSubtract")?;
    app.until("Deselect files", mask_dialog_open);
    input::key(app.slint(), Key::Escape);
    app.until("the mask dialog to close", |app| !mask_dialog_open(app));
    expect_selection(
        app,
        failures,
        "Escape in the mask dialog",
        &["a1.txt", "c.dat"],
    );

    // The same dialog from F10 → Selection, by mouse.
    panel::focus_list(app, pane)?;
    input::key(app.slint(), Key::F10);
    app.until("the main menu", |app| {
        crate::inspect::roots(app.slint()).len() > 1
    });
    support::click_scrolled(app.slint(), AccessibleRole::Button, "Select by mask…")?;
    app.settle();
    apply_mask(app, "b1*", false)?;
    expect_selection(
        app,
        failures,
        "F10 Select by mask… b1*",
        &["a1.txt", "b1.log", "c.dat"],
    );
    support::dismiss_all(app);

    // The number row's + stays typed text: it starts a filename search.
    panel::focus_list(app, pane)?;
    input::key(app.slint(), "+");
    app.settle();
    let query = app.window.get_left_search_query().to_string();
    if mask_dialog_open(app) || !app.window.get_left_search_active() || query != "+" {
        failures.push(format!(
            "the number row's + opened the dialog ({}) or searched {query:?}",
            mask_dialog_open(app)
        ));
    }
    input::key(app.slint(), Key::Escape);
    app.settle();
    context.note(
        "keypad: Num + (*.TXT;*.dat, then d*|*.log with folders), Num − (a2*), Num * with and without folders, Escape, F10 Select by mask…, number-row + searches",
    );
    Ok(())
}

fn click_row(app: &mut App, name: &str, modifiers: &[Key]) -> Result<(), String> {
    panel::wait_input(app);
    let index = panel::index_of(app, PaneId::Left, name)?;
    let position = panel::row_position(app, PaneId::Left, index);
    // Never a double click: wait out the interval since the previous click.
    app.idle(Duration::from_millis(650));
    input::click_with(app.slint(), modifiers, position);
    app.last_click = Some((std::time::Instant::now(), position));
    app.settle();
    Ok(())
}

pub fn selection(context: &mut Context) -> Result<(), String> {
    let folder = support::scratch(context, "selection")?;
    let target = support::scratch(context, "selection-target")?;
    for name in FILES {
        fs::write(folder.join(name), name).map_err(|error| io_error(&folder, error))?;
    }
    for name in FOLDERS {
        fs::create_dir(folder.join(name)).map_err(|error| io_error(&folder, error))?;
    }
    fs::write(folder.join("d1/x.bin"), vec![7_u8; 1000])
        .map_err(|error| io_error(&folder, error))?;
    let mut app = App::start(Config::new(&folder, &target));
    let pane = PaneId::Left;
    let mut failures = Vec::new();

    // Insert toggles the row and moves to the next one.
    panel::select_row(&mut app, pane, "a1.txt")?;
    let next = panel::index_of(&app, pane, "a1.txt")? + 1;
    input::key(app.slint(), Key::Insert);
    expect_selection(&mut app, &mut failures, "Insert", &["a1.txt"]);
    if app.cursor(pane) != next as i32 {
        failures.push(format!(
            "Insert left the cursor on row {} instead of {next}",
            app.cursor(pane)
        ));
    }
    input::key(app.slint(), Key::Insert);
    expect_selection(
        &mut app,
        &mut failures,
        "second Insert",
        &["a1.txt", "a2.txt"],
    );
    panel::select_row(&mut app, pane, "a1.txt")?;
    input::key(app.slint(), Key::Insert);
    expect_selection(&mut app, &mut failures, "Insert again", &["a2.txt"]);

    // Space on a folder selects it and measures it.
    panel::select_row(&mut app, pane, "d1")?;
    input::key(app.slint(), Key::Space);
    expect_selection(&mut app, &mut failures, "Space on d1", &["a2.txt", "d1"]);
    app.until("d1's size", |app| {
        app.rows(PaneId::Left).iter().any(|row| {
            row.name == "d1"
                && !row.size_running
                && row.size.chars().any(|character| character.is_ascii_digit())
        })
    });
    let d1 = app
        .rows(pane)
        .into_iter()
        .find(|row| row.name == "d1")
        .ok_or("no d1 row")?;
    context.note(format!("Space measured d1 as {:?}", d1.size));
    if support::cursor_name(&app, pane).as_deref() != Some("d2") {
        failures.push(format!(
            "Space left the cursor on {:?} instead of d2",
            support::cursor_name(&app, pane)
        ));
    }

    // Ctrl+A: files and folders; Ctrl+Shift+A: files only; Ctrl+Shift+D: the files
    // leave and the folder marks stay; Ctrl+D: nothing.
    let mut everything: Vec<&str> = FILES.to_vec();
    everything.extend(FOLDERS);
    input::chord(app.slint(), &[Key::Control], "a");
    expect_selection(&mut app, &mut failures, "Ctrl+A", &everything);
    input::chord(app.slint(), &[Key::Control, Key::Shift], "a");
    expect_selection(&mut app, &mut failures, "Ctrl+Shift+A", &FILES);
    input::chord(app.slint(), &[Key::Control], "a");
    expect_selection(&mut app, &mut failures, "Ctrl+A again", &everything);
    input::chord(app.slint(), &[Key::Control, Key::Shift], "d");
    expect_selection(&mut app, &mut failures, "Ctrl+Shift+D", &FOLDERS);
    input::chord(app.slint(), &[Key::Control], "d");
    expect_selection(&mut app, &mut failures, "Ctrl+D", &[]);

    // Ctrl+Shift+I inverts files and keeps folder marks; Ctrl+I inverts everything.
    panel::select_row(&mut app, pane, "a1.txt")?;
    input::key(app.slint(), Key::Insert);
    panel::select_row(&mut app, pane, "d1")?;
    input::key(app.slint(), Key::Insert);
    expect_selection(
        &mut app,
        &mut failures,
        "Insert a1 and d1",
        &["a1.txt", "d1"],
    );
    input::chord(app.slint(), &[Key::Control, Key::Shift], "i");
    expect_selection(
        &mut app,
        &mut failures,
        "Ctrl+Shift+I",
        &[".hidden", "a2.txt", "b1.log", "b2.log", "c.dat", "d1"],
    );
    input::chord(app.slint(), &[Key::Control], "i");
    expect_selection(&mut app, &mut failures, "Ctrl+I", &["a1.txt", "d2"]);
    input::chord(app.slint(), &[Key::Control], "d");
    expect_selection(&mut app, &mut failures, "Ctrl+D", &[]);

    // Ctrl+click toggles single rows; Shift+click selects a range.
    click_row(&mut app, "b1.log", &[Key::Control])?;
    click_row(&mut app, "c.dat", &[Key::Control])?;
    expect_selection(&mut app, &mut failures, "Ctrl+click", &["b1.log", "c.dat"]);
    click_row(&mut app, "c.dat", &[Key::Control])?;
    expect_selection(&mut app, &mut failures, "Ctrl+click again", &["b1.log"]);
    click_row(&mut app, "a1.txt", &[])?;
    click_row(&mut app, "b2.log", &[Key::Shift])?;
    expect_selection(
        &mut app,
        &mut failures,
        "click a1.txt, Shift+click b2.log",
        &["a1.txt", "a2.txt", "b1.log", "b2.log"],
    );
    app.screenshot(context, "range-selection", "Shift+click range selection");

    // The numeric keypad's +, − and * select and deselect by mask and invert
    // (Total Commander and Salamander conventions).
    if let Err(error) = keypad(&mut app, context, &mut failures) {
        failures.push(error);
        app.screenshot(context, "keypad-failure", "Select by mask did not finish");
        return support::report(&failures);
    }

    // A multi-selection copied with F5: exactly the marked files arrive, and the
    // completed roots are deselected.
    panel::focus_list(&mut app, pane)?;
    let marked = sorted(support::selected_names(&app, pane));
    match support::transfer(&mut app, context, TransferSpec::copy()) {
        Err(error) => failures.push(format!("F5 of the selection: {error}")),
        Ok(run) => {
            if let Err(error) = support::completed(&run, "F5 of the selection") {
                failures.push(error);
            }
            let copied = support::names_on_disk(&target);
            if copied != marked {
                failures.push(format!("F5 of {marked:?} put {copied:?} into the target"));
            }
            for name in &marked {
                support::wait_row(&mut app, PaneId::Right, name, true);
            }
            expect_selection(&mut app, &mut failures, "after F5", &[]);
        }
    }
    app.screenshot(
        context,
        "after-selection-copy",
        "After copying the selection",
    );
    app.shutdown()?;
    support::report(&failures)
}
