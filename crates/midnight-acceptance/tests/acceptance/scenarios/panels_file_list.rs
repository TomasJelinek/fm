//! Make File List (Ctrl+M) beyond its basic file output: the Clipboard destination;
//! the variables FileNamePart, FileExtension and FileSize, `$$`, `:max` alignment and
//! `<DIR>` for a folder; the file destination's Browse… (the scripted chooser); an
//! existing file asks first and Cancel, Replace and Append each do exactly that; the
//! focused entry when nothing is marked; a panel filter limits and orders the list;
//! the template history brings back a used template, also after a restart. Each output
//! is compared byte for byte.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support},
    input, panel,
    scenarios::panels_support::{self as ps, LEFT},
};
use i_slint_core::items::AccessibleRole;
use midnight_ui::FileListPresentation;
use slint::{ComponentHandle, platform::Key};
use std::{fs, path::Path, time::Duration};

fn active(app: &App) -> bool {
    app.window.global::<FileListPresentation>().get_active()
}

fn open(app: &mut App) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "m");
    if !ps::wait(app, Duration::from_secs(5), active) {
        return Err("Ctrl+M opened no Make File List".into());
    }
    Ok(())
}

fn template(app: &mut App, text: &str) -> Result<(), String> {
    input::fill(app.slint(), "File list line template", text)?;
    app.settle();
    Ok(())
}

fn destination(app: &mut App, choice: &str, path: Option<&Path>) -> Result<(), String> {
    crate::options_support::choose(app, "File list output destination", choice)?;
    if let Some(path) = path {
        input::fill(
            app.slint(),
            "File list destination path",
            &path.to_string_lossy(),
        )?;
    }
    app.settle();
    Ok(())
}

/// Press Create file list; the dialog must close with nothing else open.
fn create(app: &mut App, what: &str, failures: &mut Vec<String>) -> Result<(), String> {
    input::click_button(app.slint(), "Create file list")?;
    if !ps::wait(app, Duration::from_secs(15), |app| !active(app)) {
        failures.push(format!(
            "{what}: Make File List stayed open (buttons {:?})",
            ps::popup_buttons(app)
                .into_iter()
                .map(|(label, _)| label)
                .collect::<Vec<_>>()
        ));
        support::dismiss_all(app);
        return Ok(());
    }
    if let Err(error) = ps::stays_idle(app, Duration::from_millis(400)) {
        failures.push(format!("{what}: {error}"));
        support::dismiss_all(app);
    }
    Ok(())
}

fn read(path: &Path) -> String {
    fs::read(path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_else(|error| format!("<{error}>"))
}

pub fn file_list(context: &mut Context) -> Result<(), String> {
    let folder = support::scratch(context, "file-list")?;
    for (name, text) in [
        ("a.txt", "abc"),
        ("bb.log", "0123456789"),
        ("noext", "12345"),
    ] {
        ps::write(&folder.join(name), text)?;
    }
    ps::folder(&folder.join("sub"))?;
    let output = context.work.join("list-output.txt");
    let store = crate::options_support::SettingsStore::new();
    let mut config = Config::new(&folder, &context.work);
    config.clipboard = true;
    config.settings = Some(store.clone());
    let mut app = App::start(config);
    let mut failures = Vec::new();
    let clipboard = |app: &App| {
        app.clipboard
            .as_ref()
            .and_then(|clipboard| clipboard.text())
    };

    // Clipboard: the marked files with name parts, extension, size and `$$`.
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "a");
    ps::wait(&mut app, Duration::from_secs(5), |app| {
        ps::marked(app, LEFT).len() == 3
    });
    open(&mut app)?;
    template(
        &mut app,
        "$(FileNamePart)|$(FileExtension)|$(FileSize)$$$(LF)",
    )?;
    destination(&mut app, "Clipboard", None)?;
    app.screenshot(context, "clipboard", "Make File List to the clipboard");
    create(&mut app, "Clipboard", &mut failures)?;
    let wanted = "a|txt|3$\nbb|log|10$\nnoext||5$\n";
    if !ps::wait(&mut app, Duration::from_secs(5), |app| {
        clipboard(app).as_deref() == Some(wanted)
    }) {
        failures.push(format!(
            "the Clipboard list is {:?}, expected {wanted:?}",
            clipboard(&app)
        ));
    }
    if !ps::marked(&app, LEFT).is_empty() {
        failures.push("the marks stayed after a successful Clipboard list".into());
    }

    // A folder and a file to a file, names aligned with :max; the folder says <DIR>.
    for name in ["sub", "a.txt"] {
        panel::select_row(&mut app, LEFT, name)?;
        input::key(app.slint(), Key::Insert);
    }
    open(&mut app)?;
    template(&mut app, "$(FileName:max)|$(FileSize)$(LF)")?;
    destination(&mut app, "File", None)?;
    // Browse… asks the chooser; its answer becomes the destination.
    app.picker.answer(Some(output.clone()));
    let asked = app.picker.requests().len();
    match input::click_button(app.slint(), "Browse file list destination") {
        Err(error) => failures.push(format!("Browse…: {error}")),
        Ok(()) => {
            let wanted = output.to_string_lossy().into_owned();
            if !ps::wait(&mut app, Duration::from_secs(5), |app| {
                ps::value(app, AccessibleRole::TextInput, "File list destination path")
                    .is_some_and(|value| value == wanted)
            }) {
                failures.push(format!(
                    "Browse… ({} chooser request(s)) left the destination {:?}",
                    app.picker.requests().len() - asked,
                    ps::value(
                        &app,
                        AccessibleRole::TextInput,
                        "File list destination path"
                    )
                ));
                destination(&mut app, "File", Some(&output))?;
            }
        }
    }
    create(&mut app, "File with :max", &mut failures)?;
    let wanted = "sub  |<DIR>\na.txt|3\n";
    if read(&output) != wanted {
        failures.push(format!(
            "the :max list is {:?}, expected {wanted:?}",
            read(&output)
        ));
    }

    // The existing file: Cancel keeps it, Replace replaces it, Append adds to it. A
    // replaced or appended original is kept as a recovery copy that a notice names.
    let before = read(&output);
    for (choice, append, wanted) in [
        ("Cancel", false, before.clone()),
        ("Replace", false, "bb.log\r\n".to_owned()),
        ("Append", true, "bb.log\r\nbb.log\r\n".to_owned()),
    ] {
        let previous = read(&output);
        panel::select_row(&mut app, LEFT, "bb.log")?;
        open(&mut app)?;
        template(&mut app, "$(FileName)$(CRLF)")?;
        destination(&mut app, "File", Some(&output))?;
        if append {
            input::click(app.slint(), AccessibleRole::Checkbox, "Append to file")?;
            app.settle();
        }
        input::click_button(app.slint(), "Create file list")?;
        if !ps::wait(&mut app, Duration::from_secs(10), |app| {
            crate::inspect::find(
                app.slint(),
                AccessibleRole::Button,
                "Confirm existing file list output",
            )
            .is_some()
        }) {
            failures.push(format!(
                "{choice}: an existing destination was not confirmed first (file now {:?})",
                read(&output)
            ));
            support::dismiss_all(&mut app);
            continue;
        }
        if choice == "Cancel" {
            ps::click_popup_button(&mut app, "Cancel")?;
            if !ps::wait(&mut app, Duration::from_secs(5), |app| !active(app)) {
                failures.push("Cancel at the existing-file question left the dialog open".into());
                support::dismiss_all(&mut app);
            }
        } else {
            let caption = ps::popup_buttons(&app)
                .into_iter()
                .map(|(label, _)| label)
                .find(|label| label == "Confirm existing file list output");
            if caption.is_none() {
                failures.push(format!("{choice}: no confirm button"));
            }
            input::click_button(app.slint(), "Confirm existing file list output")?;
            if !ps::wait(&mut app, Duration::from_secs(10), |app| !active(app)) {
                failures.push(format!("{choice}: the dialog stayed open"));
                support::dismiss_all(&mut app);
            }
            recovery(&mut app, choice, &previous, &mut failures);
        }
        if let Err(error) = ps::stays_idle(&mut app, Duration::from_millis(400)) {
            failures.push(format!("{choice}: {error}"));
            support::dismiss_all(&mut app);
        }
        if read(&output) != wanted {
            failures.push(format!(
                "{choice} of the existing list: the file holds {:?}, expected {wanted:?}",
                read(&output)
            ));
        }
    }

    // A panel filter limits the list to what the panel shows, in its order.
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], Key::F12);
    app.until("the Panel filter", |app| {
        app.window.get_panel_filter_view().open
    });
    crate::options_support::choose(&mut app, "Filter mode", "Masks")?;
    input::fill(app.slint(), "File filter expression", "*.log;*.txt")?;
    input::click_button(app.slint(), "OK")?;
    app.until("the filter to apply", |app| {
        !app.window.get_panel_filter_view().open
    });
    input::chord(app.slint(), &[Key::Control], "a");
    ps::wait(&mut app, Duration::from_secs(5), |app| {
        ps::marked(app, LEFT).len() == 2
    });
    open(&mut app)?;
    // The template history offers the templates used before.
    input::click(
        app.slint(),
        AccessibleRole::Combobox,
        "File list template history",
    )?;
    app.settle();
    let used = "$(FileNamePart)|$(FileExtension)|$(FileSize)$$$(LF)";
    let offered: Vec<String> = ps::popup_buttons(&app)
        .into_iter()
        .map(|(label, _)| label)
        .collect();
    match ps::click_popup_button(&mut app, used) {
        Err(_) => failures.push(format!(
            "the template history does not offer {used:?}: {offered:?}"
        )),
        Ok(()) => {
            let field = ps::value(&app, AccessibleRole::TextInput, "File list line template");
            if field.as_deref() != Some(used) {
                failures.push(format!("choosing a used template filled {field:?}"));
            }
        }
    }
    support::dismiss_all(&mut app);
    open(&mut app)?;
    template(&mut app, "$(FileName)$(LF)")?;
    destination(&mut app, "Clipboard", None)?;
    create(&mut app, "filtered Clipboard", &mut failures)?;
    let wanted = "a.txt\nbb.log\n";
    if !ps::wait(&mut app, Duration::from_secs(5), |app| {
        clipboard(app).as_deref() == Some(wanted)
    }) {
        failures.push(format!(
            "with the *.log;*.txt filter the list is {:?}, expected {wanted:?}",
            clipboard(&app)
        ));
    }
    panel::focus_list(&mut app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], Key::F12);
    app.until("the Panel filter", |app| {
        app.window.get_panel_filter_view().open
    });
    input::click_button(app.slint(), "Show all")?;
    app.until("the filter to close", |app| {
        !app.window.get_panel_filter_view().open
    });
    app.shutdown()?;
    drop(app);
    // The template history is saved: after a restart it still offers the templates used.
    let mut config = Config::new(&folder, &context.work);
    config.settings = Some(store);
    let mut app = App::start(config);
    open(&mut app)?;
    input::click(
        app.slint(),
        AccessibleRole::Combobox,
        "File list template history",
    )?;
    app.settle();
    let offered: Vec<String> = ps::popup_buttons(&app)
        .into_iter()
        .map(|(label, _)| label)
        .collect();
    for used in [
        "$(FileName)$(LF)",
        "$(FileNamePart)|$(FileExtension)|$(FileSize)$$$(LF)",
    ] {
        if !offered.iter().any(|label| label == used) {
            failures.push(format!(
                "after a restart the template history does not offer {used:?}: {offered:?}"
            ));
        }
    }
    support::dismiss_all(&mut app);
    app.shutdown()?;
    support::report(&failures)
}

/// The notice after a confirmed Replace or Append names the recovery copy of the
/// original, which holds its bytes; Escape closes it.
fn recovery(app: &mut App, choice: &str, previous: &str, failures: &mut Vec<String>) {
    if !ps::wait(app, Duration::from_secs(5), |app| {
        support::shown_notice(app).is_some()
    }) {
        failures.push(format!(
            "{choice}: no notice names where the original list was kept"
        ));
        return;
    }
    let (title, body) = support::shown_notice(app).unwrap_or_default();
    let copy = body
        .lines()
        .map(str::trim)
        .find(|line| line.contains(".midnight-recovery-"))
        .map(std::path::PathBuf::from);
    match copy {
        None => failures.push(format!(
            "{choice}: the notice {title:?} names no recovery copy: {body:?}"
        )),
        Some(copy) if read(&copy) != previous => failures.push(format!(
            "{choice}: the recovery copy {} holds {:?}, not the original {previous:?}",
            copy.display(),
            read(&copy)
        )),
        Some(_) => {}
    }
    input::key(app.slint(), Key::Escape);
    if !ps::wait(app, Duration::from_secs(5), |app| {
        support::shown_notice(app).is_none()
    }) {
        failures.push(format!(
            "{choice}: Escape did not close the recovery notice"
        ));
        support::dismiss_all(app);
    }
}
