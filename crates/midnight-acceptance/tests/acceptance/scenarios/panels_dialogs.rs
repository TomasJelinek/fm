//! Every panel dialog, opened by its key and by the pointer (toolbar, footer, menus),
//! closed in every way it offers (Escape, the header's ×, its Cancel or Close button)
//! and then used once for real. Each close must leave plain browsing behind: no
//! notice, no window, no popup, input accepted and the keyboard back in the file list,
//! and nothing changed on disk; each dialog shows one close control in its header and
//! one Cancel/Close button. After all the cancelled rounds every dialog still does its
//! job (a dialog that only works the first time, or a Cancel that leaves a notice
//! behind, fails here).
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, tree_facts},
    input, panel,
    scenarios::panels_support::{self as ps, LEFT},
};
use i_slint_core::items::AccessibleRole;
use midnight_ui::{FileListPresentation, HotPathsPresentation, PanelChoices};
use slint::{ComponentHandle, Model, platform::Key};
use std::{fs, path::Path, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dialog {
    CreateFolder,
    NewFile,
    Rename,
    Attributes,
    Case,
    Filter,
    SelectMask,
    DeselectMask,
    FileList,
    HotPaths,
    Properties,
    DriveInfo,
}

const ALL: [Dialog; 12] = [
    Dialog::CreateFolder,
    Dialog::NewFile,
    Dialog::Rename,
    Dialog::Attributes,
    Dialog::Case,
    Dialog::Filter,
    Dialog::SelectMask,
    Dialog::DeselectMask,
    Dialog::FileList,
    Dialog::HotPaths,
    Dialog::Properties,
    Dialog::DriveInfo,
];

#[derive(Clone, Copy, Debug)]
enum Close {
    Escape,
    Header,
    Footer,
}

fn operation(app: &App, kind: i32) -> bool {
    !app.window.get_operation_token().is_empty() && app.window.get_operation_kind() == kind
}

impl Dialog {
    fn name(self) -> &'static str {
        match self {
            Self::CreateFolder => "Create directory (F7)",
            Self::NewFile => "New empty file (Shift+F4)",
            Self::Rename => "Rename (F2)",
            Self::Attributes => "Attributes (Ctrl+F2)",
            Self::Case => "Filename case (Ctrl+F7)",
            Self::Filter => "Panel filter (Ctrl+F12)",
            Self::SelectMask => "Select files (Ctrl+=)",
            Self::DeselectMask => "Deselect files (Ctrl+−)",
            Self::FileList => "Make File List (Ctrl+M)",
            Self::HotPaths => "Manage Hot Paths",
            Self::Properties => "Properties (Alt+Enter)",
            Self::DriveInfo => "Drive information (Ctrl+F1)",
        }
    }

    fn is_open(self, app: &App) -> bool {
        let metadata = !app.window.get_metadata_token().is_empty();
        let choices = app.window.global::<PanelChoices>();
        let mask = !choices.get_mask_token().is_empty();
        let notice = support::shown_notice(app);
        match self {
            Self::CreateFolder => operation(app, 3),
            Self::NewFile => operation(app, 4),
            Self::Rename => operation(app, 0),
            Self::Attributes => metadata && !app.window.get_metadata_start_case(),
            Self::Case => metadata && app.window.get_metadata_start_case(),
            Self::Filter => app.window.get_panel_filter_view().open,
            Self::SelectMask => mask && choices.get_mask_select(),
            Self::DeselectMask => mask && !choices.get_mask_select(),
            Self::FileList => app.window.global::<FileListPresentation>().get_active(),
            Self::HotPaths => !app
                .window
                .global::<HotPathsPresentation>()
                .get_token()
                .is_empty(),
            Self::Properties => notice.is_some_and(|(title, _)| title != "Drive information"),
            Self::DriveInfo => notice.is_some_and(|(title, _)| title == "Drive information"),
        }
    }

    /// The header's close control and the footer's dismiss button.
    fn close_labels(self) -> (&'static str, &'static str) {
        match self {
            Self::Properties | Self::DriveInfo | Self::HotPaths => ("Close dialog", "Close"),
            Self::FileList => ("Cancel file list", "Cancel"),
            _ => ("Cancel dialog", "Cancel"),
        }
    }

    /// Open it with its keyboard shortcut (the Hot Paths manager has none: F10).
    fn open_key(self, app: &mut App) -> Result<(), String> {
        panel::focus_list(app, LEFT)?;
        let window = app.slint();
        match self {
            Self::CreateFolder => input::key(window, Key::F7),
            Self::NewFile => input::chord(window, &[Key::Shift], Key::F4),
            Self::Rename => input::key(window, Key::F2),
            Self::Attributes => input::chord(window, &[Key::Control], Key::F2),
            Self::Case => input::chord(window, &[Key::Control], Key::F7),
            Self::Filter => input::chord(window, &[Key::Control], Key::F12),
            Self::SelectMask => input::chord(window, &[Key::Control], "="),
            Self::DeselectMask => input::chord(window, &[Key::Control], "-"),
            Self::FileList => input::chord(window, &[Key::Control], "m"),
            Self::HotPaths => return main_menu(app, "Manage Hot Paths…"),
            Self::Properties => input::chord(window, &[Key::Alt], Key::Return),
            Self::DriveInfo => input::chord(window, &[Key::Control], Key::F1),
        }
        app.settle();
        Ok(())
    }

    /// Open it with the pointer: a toolbar button, the footer (with a held modifier
    /// for its Ctrl layer), the Hot Paths menu or F10.
    fn open_pointer(self, app: &mut App) -> Result<(), String> {
        panel::focus_list(app, LEFT)?;
        match self {
            Self::CreateFolder => footer(app, &[], "F7 "),
            Self::NewFile => ps::click_starting(app, "New empty file").map(drop),
            Self::Rename => footer(app, &[], "F2 "),
            Self::Attributes => ps::click_starting(app, "Attributes, times and case").map(drop),
            Self::Case => footer(app, &[Key::Control], "Ctrl+F7 "),
            Self::Filter => ps::click_starting(app, "Left panel filter").map(drop),
            Self::SelectMask => main_menu(app, "Select by mask…"),
            Self::DeselectMask => main_menu(app, "Deselect by mask…"),
            Self::FileList => ps::click_starting(app, "Make File List").map(drop),
            Self::HotPaths => {
                input::click(app.slint(), AccessibleRole::Combobox, "Hot Paths")?;
                app.settle();
                input::click_button(app.slint(), "Manage Hot Paths…")?;
                app.settle();
                Ok(())
            }
            Self::Properties => ps::click_starting(app, "Properties").map(drop),
            Self::DriveInfo => footer(app, &[Key::Control], "Ctrl+F1 "),
        }
    }
}

/// F10, then the menu item `label`.
fn main_menu(app: &mut App, label: &str) -> Result<(), String> {
    input::key(app.slint(), Key::F10);
    if !ps::wait(app, Duration::from_secs(5), |app| {
        crate::inspect::roots(app.slint()).len() > 1
    }) {
        return Err("F10 opened no main menu".into());
    }
    support::click_scrolled(app.slint(), AccessibleRole::Button, label)?;
    app.settle();
    Ok(())
}

/// Click the footer's function cell starting with `prefix` while `held` keys are down
/// (the footer shows the layer of the held modifiers).
fn footer(app: &mut App, held: &[Key], prefix: &str) -> Result<(), String> {
    for key in held {
        input::press(app.slint(), *key);
    }
    app.settle();
    let result = ps::click_starting(app, prefix);
    for key in held.iter().rev() {
        input::release(app.slint(), *key);
    }
    app.settle();
    result.map(drop)
}

/// One open and close round; problems are recorded, never raised, so every dialog
/// gets its rounds.
fn round(
    app: &mut App,
    context: &mut Context,
    dialog: Dialog,
    pointer: bool,
    close: Close,
    failures: &mut Vec<String>,
) {
    let what = format!(
        "{} opened by {} and closed with {close:?}",
        dialog.name(),
        if pointer { "the pointer" } else { "its key" }
    );
    if let Err(error) = panel::select_row(app, LEFT, "a.txt") {
        // Recorded, then retried once so the round still tests its dialog.
        failures.push(format!("{what}: clicking a.txt: {error}"));
        if let Err(error) = panel::select_row(app, LEFT, "a.txt") {
            failures.push(format!("{what}: clicking a.txt again: {error}"));
            return;
        }
    }
    let opened = if pointer {
        dialog.open_pointer(app)
    } else {
        dialog.open_key(app)
    };
    if let Err(error) = opened {
        failures.push(format!("{what}: {error}"));
        support::dismiss_all(app);
        return;
    }
    if !ps::wait(app, Duration::from_secs(10), |app| dialog.is_open(app)) {
        failures.push(format!(
            "{what}: the dialog did not open (notice {:?}, modal {}, popups {})",
            support::shown_notice(app),
            app.window.get_embedded_modal_open(),
            crate::inspect::roots(app.slint()).len() - 1
        ));
        support::dismiss_all(app);
        return;
    }
    // A drive information or Properties notice fills in after a moment.
    app.idle(Duration::from_millis(150));
    let (header, footer) = dialog.close_labels();
    let buttons: Vec<String> = ps::popup_buttons(app)
        .into_iter()
        .map(|(label, _)| label)
        .collect();
    let headers = buttons
        .iter()
        .filter(|label| ps::HEADER_CLOSE.contains(&label.as_str()))
        .count();
    let footers = buttons.iter().filter(|label| *label == footer).count();
    if headers != 1 || !buttons.iter().any(|label| label == header) || footers != 1 {
        failures.push(format!(
            "{what}: expected one {header:?} in the header and one {footer:?} button, found {headers} header close control(s) and {footers} {footer:?}; buttons {buttons:?}"
        ));
    }
    if !pointer && matches!(close, Close::Escape) {
        app.screenshot(
            context,
            &format!("{:?}", dialog).to_lowercase(),
            dialog.name(),
        );
    }
    let closed = match close {
        Close::Escape => {
            input::key(app.slint(), Key::Escape);
            Ok(())
        }
        Close::Header => ps::click_popup_button(app, header),
        Close::Footer => ps::click_popup_button(app, footer),
    };
    if let Err(error) = closed {
        failures.push(format!("{what}: {error}"));
        support::dismiss_all(app);
        return;
    }
    if !ps::wait(app, Duration::from_secs(5), |app| !dialog.is_open(app)) {
        failures.push(format!("{what}: the dialog stayed open"));
        support::dismiss_all(app);
        return;
    }
    if let Err(error) = ps::stays_idle(app, Duration::from_millis(600)) {
        failures.push(format!("{what}: {error}"));
        support::dismiss_all(app);
        return;
    }
    // The keyboard is back in the file list: Down moves the cursor.
    let before = app.cursor(LEFT);
    input::key(app.slint(), Key::DownArrow);
    app.settle();
    let after = app.cursor(LEFT);
    if after == before || app.window.get_focused_pane() != 0 {
        failures.push(format!(
            "{what}: afterwards Down does not move the panel cursor (row {before} → {after}, focused pane {}): the keyboard is not back in the file list",
            app.window.get_focused_pane()
        ));
    }
}

pub fn dialogs(context: &mut Context) -> Result<(), String> {
    let folder = support::scratch(context, "dialogs")?;
    for (name, text) in [
        ("a.txt", "alpha"),
        ("b.txt", "bravo"),
        ("c.log", "charlie"),
        ("d.dat", "delta"),
    ] {
        ps::write(&folder.join(name), text)?;
        #[cfg(unix)]
        crate::tools_support::set_mode(&folder.join(name), 0o644)?;
    }
    ps::folder(&folder.join("sub"))?;
    ps::write(&folder.join("sub/inner.txt"), "inner")?;
    let output = context.work.join("file-list.txt");
    let mut config = Config::new(&folder, &context.work);
    config.clipboard = true;
    let mut app = App::start(config);
    let mut failures = Vec::new();
    let before = tree_facts(&folder)?;

    let mut rounds = 0;
    for dialog in ALL {
        for (pointer, close) in [
            (false, Close::Escape),
            (true, Close::Header),
            (false, Close::Footer),
            (true, Close::Escape),
            (true, Close::Footer),
        ] {
            round(&mut app, context, dialog, pointer, close, &mut failures);
            rounds += 1;
        }
    }
    let after = tree_facts(&folder)?;
    let changed = support::compare_trees(&before, &after, true);
    if !changed.is_empty() {
        failures.push(format!("cancelled dialogs changed the folder: {changed:?}"));
    }
    if app.window.get_bookmarks().row_count() != 0 {
        failures.push("a cancelled Hot Paths manager added a Hot Path".into());
    }
    context.note(format!(
        "{rounds} open/close rounds of {} dialogs (key and pointer; Escape, header ×, Cancel/Close)",
        ALL.len()
    ));
    confirm_each(&mut app, context, &folder, &output, &mut failures)?;
    app.shutdown()?;
    support::report(&failures)
}

/// After the cancelled rounds every dialog still does its job.
fn confirm_each(
    app: &mut App,
    context: &mut Context,
    folder: &Path,
    output: &Path,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    // F7 and Shift+F4.
    for (modifiers, key, name, directory) in [
        (&[][..], Key::F7, "made after cancels", true),
        (&[Key::Shift][..], Key::F4, "new after cancels.txt", false),
    ] {
        panel::select_row(app, LEFT, "a.txt")?;
        let start = support::named_operation(app, modifiers, key, name)?;
        let progress = support::wait_operation(app, start);
        let path = folder.join(name);
        if let Err(error) = support::operation_ok(&progress, name) {
            failures.push(format!("after the cancelled rounds: {error}"));
        } else if directory != path.is_dir() || !path.exists() {
            failures.push(format!(
                "after the cancelled rounds {name:?} is not on disk as a {}",
                if directory { "folder" } else { "file" }
            ));
        } else {
            support::wait_row(app, LEFT, name, true);
        }
    }
    // F2.
    panel::select_row(app, LEFT, "b.txt")?;
    let start = support::named_operation(app, &[], Key::F2, "b renamed.txt")?;
    let progress = support::wait_operation(app, start);
    if let Err(error) = support::operation_ok(&progress, "F2 after the cancelled rounds") {
        failures.push(error);
    } else if !folder.join("b renamed.txt").is_file() || folder.join("b.txt").exists() {
        failures.push("F2 after the cancelled rounds did not rename b.txt on disk".into());
    }
    // Ctrl+F2: octal 600 on c.log; on Windows, whose Attributes dialog has attributes
    // instead of permissions, the Read-only attribute set.
    panel::select_row(app, LEFT, "c.log")?;
    input::chord(app.slint(), &[Key::Control], Key::F2);
    app.until("the Attributes dialog to load", |app| {
        !app.window.get_metadata_token().is_empty()
            && !app.window.get_metadata_busy()
            && (!cfg!(unix) || app.window.get_metadata_unix_known_mask() != 0)
    });
    if cfg!(unix) {
        input::fill(app.slint(), "Octal permissions", "600")?;
    } else {
        ps::set_attribute(app, "Read-only", "Set")?;
    }
    app.settle();
    input::click_button(app.slint(), "Review changes")?;
    app.until("the reviewed draft", |app| {
        app.window.get_metadata_preview_current() && !app.window.get_metadata_busy()
    });
    input::click_button(app.slint(), "Apply")?;
    if !ps::wait(app, Duration::from_secs(30), |app| {
        app.window.get_metadata_token().is_empty()
    }) {
        failures.push("Ctrl+F2 Apply after the cancelled rounds left the editor open".into());
        support::dismiss_all(app);
    }
    let mode = support::facts(&folder.join("c.log")).map_or(0, |facts| facts.mode);
    if cfg!(unix) && mode != 0o600 {
        failures.push(format!(
            "Ctrl+F2 after the cancelled rounds left c.log at mode {mode:o}, not 600"
        ));
    }
    if cfg!(windows) && !ps::read_only_attribute(&folder.join("c.log")) {
        failures.push(
            "Ctrl+F2 after the cancelled rounds left c.log without the Read-only attribute".into(),
        );
    }
    // Ctrl+F12 Masks *.log, then Show all.
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], Key::F12);
    app.until("the Panel filter", |app| {
        app.window.get_panel_filter_view().open
    });
    crate::options_support::choose(app, "Filter mode", "Masks")?;
    input::fill(app.slint(), "File filter expression", "*.log")?;
    input::click_button(app.slint(), "OK")?;
    let wanted = ps::sorted(&["c.log", "made after cancels", "sub"]);
    if !ps::wait(app, Duration::from_secs(10), |app| {
        let mut names = ps::top_level(app, LEFT);
        names.sort();
        names == wanted
    }) {
        failures.push(format!(
            "Ctrl+F12 *.log after the cancelled rounds lists {:?}, expected {wanted:?}",
            ps::top_level(app, LEFT)
        ));
    }
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], Key::F12);
    app.until("the Panel filter", |app| {
        app.window.get_panel_filter_view().open
    });
    input::click_button(app.slint(), "Show all")?;
    app.until("the filter to close", |app| {
        !app.window.get_panel_filter_view().open
    });
    // Ctrl+= a* marks a.txt; Ctrl+− * clears it.
    panel::focus_list(app, LEFT)?;
    for (select, mask, wanted) in [(true, "a*", vec!["a.txt"]), (false, "*", vec![])] {
        input::chord(app.slint(), &[Key::Control], if select { "=" } else { "-" });
        app.until("the mask dialog", |app| {
            !app.window
                .global::<PanelChoices>()
                .get_mask_token()
                .is_empty()
        });
        input::fill(app.slint(), "Selection mask", mask)?;
        input::key(app.slint(), Key::Return);
        let expected = ps::sorted(&wanted);
        if !ps::wait(app, Duration::from_secs(10), |app| {
            app.window
                .global::<PanelChoices>()
                .get_mask_token()
                .is_empty()
                && ps::marked(app, LEFT) == expected
        }) {
            failures.push(format!(
                "Ctrl+{} {mask} after the cancelled rounds marked {:?}, expected {expected:?}",
                if select { "=" } else { "−" },
                ps::marked(app, LEFT)
            ));
            support::dismiss_all(app);
        }
    }
    // Ctrl+M of the focused a.txt to a file.
    panel::select_row(app, LEFT, "a.txt")?;
    input::chord(app.slint(), &[Key::Control], "m");
    app.until("Make File List", |app| {
        app.window.global::<FileListPresentation>().get_active()
    });
    crate::options_support::choose(app, "File list output destination", "File")?;
    input::fill(
        app.slint(),
        "File list destination path",
        &output.to_string_lossy(),
    )?;
    input::click_button(app.slint(), "Create file list")?;
    if !ps::wait(app, Duration::from_secs(15), |app| {
        !app.window.global::<FileListPresentation>().get_active()
    }) {
        failures.push("Make File List after the cancelled rounds stayed open".into());
        support::dismiss_all(app);
    }
    match fs::read(output) {
        Ok(bytes) if bytes == b"a.txt\r\n" => {}
        Ok(bytes) => failures.push(format!(
            "Make File List after the cancelled rounds wrote {:?}",
            String::from_utf8_lossy(&bytes)
        )),
        Err(error) => failures.push(format!(
            "Make File List after the cancelled rounds wrote no file: {error}"
        )),
    }
    // The Hot Paths manager adds the current folder.
    Dialog::HotPaths.open_pointer(app)?;
    app.until("the Hot Paths manager", |app| Dialog::HotPaths.is_open(app));
    input::click_button(app.slint(), "Add current")?;
    if !ps::wait(app, Duration::from_secs(5), |app| {
        app.window
            .get_bookmarks()
            .iter()
            .any(|row| Path::new(row.path.as_str()) == folder)
    }) {
        failures.push("Add current after the cancelled rounds saved no Hot Path".into());
    }
    ps::click_popup_button(app, "Close")?;
    app.until("the manager to close", |app| !Dialog::HotPaths.is_open(app));
    // Properties of a.txt names its type, size and permissions.
    panel::select_row(app, LEFT, "a.txt")?;
    input::chord(app.slint(), &[Key::Alt], Key::Return);
    if ps::wait(app, Duration::from_secs(10), |app| {
        support::shown_notice(app).is_some_and(|(_, body)| body.contains("Size:"))
    }) {
        let (title, body) = support::shown_notice(app).unwrap_or_default();
        let mut missing: Vec<&str> = ["Type: File", "Size: 5 bytes", "Modified: "]
            .into_iter()
            .filter(|part| !body.contains(part))
            .collect();
        if cfg!(unix) && !body.contains("Permissions: rw-r--r-- (0644)") {
            missing.push("Permissions: rw-r--r-- (0644)");
        }
        if !title.ends_with("a.txt") || !missing.is_empty() {
            failures.push(format!(
                "Properties of a.txt: title {title:?}, body lacks {missing:?}: {body:?}"
            ));
        }
        app.screenshot(context, "properties-file", "Alt+Enter Properties of a file");
    } else {
        failures.push(format!(
            "Alt+Enter after the cancelled rounds showed no Properties: {:?}",
            support::shown_notice(app)
        ));
    }
    support::dismiss_all(app);
    // Properties of several marked items: their kinds counted and the files' bytes.
    for name in ["a.txt", "d.dat", "sub"] {
        panel::select_row(app, LEFT, name)?;
        input::key(app.slint(), Key::Insert);
    }
    panel::wait_input(app);
    input::chord(app.slint(), &[Key::Alt], Key::Return);
    let bytes = ["a.txt", "d.dat"]
        .iter()
        .map(|name| fs::metadata(folder.join(name)).map_or(0, |metadata| metadata.len()))
        .sum::<u64>();
    let wanted = [
        "2 files, 1 folder".to_owned(),
        format!("Size: {bytes} bytes in 2 files"),
    ];
    if ps::wait(app, Duration::from_secs(10), |app| {
        support::shown_notice(app).is_some()
    }) {
        let (title, body) = support::shown_notice(app).unwrap_or_default();
        let missing: Vec<&String> = wanted.iter().filter(|part| !body.contains(*part)).collect();
        if title != "3 selected items" || !missing.is_empty() {
            failures.push(format!(
                "Properties of a.txt, d.dat and sub/: title {title:?}, missing {missing:?} in {body:?}"
            ));
        }
        app.screenshot(
            context,
            "properties-several",
            "Alt+Enter Properties of three marked items",
        );
    } else {
        failures.push("Alt+Enter on three marked items showed no Properties".into());
    }
    support::dismiss_all(app);
    panel::focus_list(app, LEFT)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    Ok(())
}
