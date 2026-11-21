//! Options › Custom Commands end to end: create, edit, drag to reorder and delete
//! commands and a separator; placeholder arguments; grouped and per-item runs from the
//! toolbar verified on disk; a failing command's notice; a dry-run preview; the
//! keyboard editor's `user-command.<id>` entry bound to a key; Cancel discarding;
//! persistence across a restart; and the F9 User menu.
//!
//! Every command runs "directly" (never in a terminal) and only writes into this
//! scenario's `out` folder: `/bin/sh` with a literal script and `touch`.
use crate::{
    Context,
    app::{App, Config},
    input, inspect,
    options_support::{self as options, SettingsStore},
    panel,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{PaneId, UserCommand, UserCommandInvocation, UserCommandLaunch};
use slint::{LogicalPosition, platform::Key};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const LIST: &str = "List selection";
const LIST_RENAMED: &str = "List selected files";
const STAMP: &str = "Stamp each";
const GROUP: &str = "Tools";
const BROKEN: &str = "Broken tool";
const DISCARDED: &str = "Discarded draft";
const SHORTCUT: &str = "Ctrl+Alt+J";
const CAPTION: &str = "List";
const COMMANDS_LIST: &str = "Configured Custom Commands";
/// The grouped command's script: one line per argument after `$0`, written into its
/// working folder.
use crate::tool::LIST_SCRIPT;

struct Setup {
    panel: PathBuf,
    out: PathBuf,
    missing: PathBuf,
    sh: PathBuf,
    touch: PathBuf,
    store: Arc<SettingsStore>,
}

impl Setup {
    fn config(&self) -> Config {
        let mut config = Config::new(&self.panel, &self.out);
        config.settings = Some(self.store.clone());
        config.user_commands = true;
        config
    }
}

/// One argument row: typed literally, or inserted with the Argument value picker.
enum Argument<'a> {
    Text(&'a str),
    Value(&'a str),
}

/// The configured-command rows, top to bottom: (command name, rectangle).
fn command_rows(app: &App) -> Vec<(String, inspect::Rect)> {
    options::list_rows(app, COMMANDS_LIST)
        .into_iter()
        .map(|(label, rect)| {
            let name = label.split(" · ").next().unwrap_or_default();
            (name.trim_start_matches('─').trim().to_owned(), rect)
        })
        .collect()
}

fn command_names(app: &App) -> Vec<String> {
    command_rows(app)
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

fn select_command(app: &mut App, name: &str) -> Result<(), String> {
    let prefix = format!("{name} · ");
    options::click_row(app, COMMANDS_LIST, name, &|label| {
        label.starts_with(&prefix)
    })?;
    let wanted = name.to_owned();
    app.until(&format!("the {name} form"), move |app| {
        options::value(app, AccessibleRole::TextInput, "Command name").as_deref()
            == Some(wanted.as_str())
    });
    Ok(())
}

fn add_argument(app: &mut App, argument: &Argument<'_>) -> Result<(), String> {
    match argument {
        Argument::Text(text) => {
            options::press(app, "Add argument")?;
            input::type_text(app.slint(), text);
        }
        Argument::Value(label) => {
            options::choose(app, "Argument value", label)?;
            options::press(app, "Insert argument value")?;
        }
    }
    app.idle(Duration::from_millis(60));
    Ok(())
}

/// Save (stage) the open draft once its validation enabled Save.
fn save_draft(app: &mut App) -> Result<(), String> {
    app.idle(Duration::from_millis(200));
    options::wait_enabled(app, "Save Custom Command")?;
    options::press(app, "Save Custom Command")?;
    app.idle(Duration::from_millis(100));
    if options::texts_with(app, "Command staged").is_empty() {
        return Err(format!(
            "Save did not stage the command; visible text: {:?}",
            inspect::visible_texts(app.slint())
        ));
    }
    Ok(())
}

/// New Custom Command with a program, arguments and working folder, then Save.
fn new_program(
    app: &mut App,
    name: &str,
    executable: &Path,
    arguments: &[Argument<'_>],
    working: &Path,
    per_item: bool,
) -> Result<(), String> {
    options::press(app, "New Custom Command")?;
    app.until("the new command form", |app| {
        options::value(app, AccessibleRole::TextInput, "Command name").as_deref()
            == Some("New command")
    });
    options::fill(app, "Command name", name)?;
    options::fill(
        app,
        "Program executable (absolute path)",
        &executable.to_string_lossy(),
    )?;
    for argument in arguments {
        add_argument(app, argument)?;
    }
    options::fill(app, "Working folder", &working.to_string_lossy())?;
    if per_item {
        options::choose(
            app,
            "Custom Command execution mode",
            "Run for each selected item",
        )?;
    }
    save_draft(app)
}

fn new_separator(app: &mut App, name: &str) -> Result<(), String> {
    options::press(app, "New Custom Command")?;
    app.until("the new command form", |app| {
        options::value(app, AccessibleRole::TextInput, "Command name").as_deref()
            == Some("New command")
    });
    options::choose(app, "Custom Command type", "Separator")?;
    options::fill(app, "Command name", name)?;
    save_draft(app)
}

/// The toolbar's command buttons and separators ("|"), left to right.
fn toolbar(app: &App, names: &[&str]) -> Vec<String> {
    let mut items: Vec<(f32, String)> = inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .filter(|(label, _)| names.contains(&label.as_str()))
        .map(|(label, item)| (inspect::rect(&item).0, label))
        .chain(
            inspect::controls(app.slint(), AccessibleRole::Text)
                .into_iter()
                .filter(|(label, _)| label == "Command group separator")
                .map(|(_, item)| (inspect::rect(&item).0, "|".to_owned())),
        )
        .collect();
    items.sort_by(|a, b| a.0.total_cmp(&b.0));
    items.into_iter().map(|(_, label)| label).collect()
}

/// The caption shown inside a toolbar command button (empty for icon-only buttons).
fn button_caption(app: &App, name: &str) -> String {
    let Some(button) = inspect::find(app.slint(), AccessibleRole::Button, name) else {
        return String::new();
    };
    let mut caption = String::new();
    inspect::walk(&button, &mut |item| {
        if let Some(text) = options::text_of(item)
            && !text.trim().is_empty()
            && caption.is_empty()
        {
            caption = text;
        }
    });
    caption
}

/// Mark exactly `names` in the left panel (Ctrl+D, then Insert on each).
fn select_files(app: &mut App, names: &[&str]) -> Result<(), String> {
    panel::focus_list(app, PaneId::Left)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    for name in names {
        panel::select_row(app, PaneId::Left, name)?;
        input::key(app.slint(), Key::Insert);
        app.settle();
    }
    let selected: Vec<String> = app
        .rows(PaneId::Left)
        .into_iter()
        .filter(|row| row.selected)
        .map(|row| row.name.to_string())
        .collect();
    let mut wanted: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    wanted.sort();
    let mut actual = selected.clone();
    actual.sort();
    if actual != wanted {
        return Err(format!(
            "the left panel selected {selected:?} instead of {names:?}"
        ));
    }
    Ok(())
}

fn read_lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Wait (bounded) until `path` holds `count` lines.
fn wait_lines(app: &mut App, path: &Path, count: usize, what: &str) -> Result<Vec<String>, String> {
    options::wait_file(app, path, what)?;
    for _ in 0..200 {
        let lines = read_lines(path);
        if lines.len() >= count {
            return Ok(lines);
        }
        app.idle(Duration::from_millis(25));
    }
    Err(format!(
        "{what}: {} holds {:?}",
        path.display(),
        read_lines(path)
    ))
}

fn find_command<'a>(commands: &'a [UserCommand], name: &str) -> Result<&'a UserCommand, String> {
    commands
        .iter()
        .find(|command| command.name == name)
        .ok_or_else(|| format!("the saved commands lack {name:?}"))
}

pub fn custom_commands(context: &mut Context) -> Result<(), String> {
    let root = context.work.join("commands");
    let setup = Setup {
        panel: root.join("panel"),
        out: root.join("out"),
        missing: root.join("missing-tool"),
        sh: crate::tool::program(context, "sh")?,
        touch: crate::tool::program(context, "touch")?,
        store: SettingsStore::new(),
    };
    fs::create_dir_all(&setup.panel).map_err(|error| error.to_string())?;
    fs::create_dir_all(&setup.out).map_err(|error| error.to_string())?;
    for name in ["alpha.txt", "beta one.txt", "gamma.md"] {
        fs::write(setup.panel.join(name), name).map_err(|error| error.to_string())?;
    }
    let mut app = App::start(setup.config());

    // Create: a grouped program, a per-item program, a separator and a failing one.
    options::open(&mut app)?;
    options::page(&mut app, "Custom Commands")?;
    if options::texts_with(&app, "No Custom Commands configured").is_empty() {
        return Err("the empty Custom Commands page does not say so".into());
    }
    new_program(
        &mut app,
        LIST,
        &setup.sh,
        &[
            Argument::Text("-c"),
            Argument::Text(LIST_SCRIPT),
            Argument::Text("listing"),
            Argument::Value("Selected paths"),
        ],
        &setup.out,
        false,
    )?;
    new_program(
        &mut app,
        STAMP,
        &setup.touch,
        &[Argument::Text("${file.name}.stamp")],
        &setup.out,
        true,
    )?;
    new_separator(&mut app, GROUP)?;
    new_program(&mut app, BROKEN, &setup.missing, &[], &setup.out, false)?;
    let names = command_names(&app);
    if names != [LIST, STAMP, GROUP, BROKEN] {
        return Err(format!(
            "the command list shows {names:?} after creating four items"
        ));
    }
    options::review_shot(
        &mut app,
        context,
        "commands-created",
        "Custom Commands: four items created",
    );

    // Reorder by dragging the second row above the first (the list shows up to six rows;
    // scroll the page to it first).
    let first = format!("{LIST} · ");
    let last = format!("{STAMP} · ");
    for prefix in [&first, &last] {
        options::reveal_where(&mut app, "a command row", &|app| {
            options::find_where(app, &|item| {
                item.accessible_role() == AccessibleRole::ListItem
                    && inspect::label(item).is_some_and(|label| label.starts_with(prefix.as_str()))
            })
        })?;
    }
    let rows = command_rows(&app);
    let (from, to) = (rows[1].1, rows[0].1);
    input::drag(
        app.slint(),
        LogicalPosition::new(from.0 + from.2 / 2.0, from.1 + from.3 / 2.0),
        LogicalPosition::new(to.0 + to.2 / 2.0, to.1 + 2.0),
        12,
    );
    app.idle(Duration::from_millis(100));
    let names = command_names(&app);
    if names != [STAMP, LIST, GROUP, BROKEN] {
        return Err(format!(
            "dragging {STAMP:?} to the top left the order {names:?}"
        ));
    }

    // Edit: rename the grouped command and give it a toolbar caption.
    select_command(&mut app, LIST)?;
    options::fill(&mut app, "Command name", LIST_RENAMED)?;
    options::press(&mut app, "Toolbar appearance")?;
    options::fill(&mut app, "Toolbar label (optional)", CAPTION)?;
    save_draft(&mut app)?;
    let names = command_names(&app);
    if names != [STAMP, LIST_RENAMED, GROUP, BROKEN] {
        return Err(format!("renaming left the list {names:?}"));
    }
    if !options::ok(&mut app, &setup.store)? {
        return Err("OK did not save the Custom Commands".into());
    }

    // What was saved: order, ids, programs, placeholders and launch modes.
    let saved = setup.store.current().user_commands.clone();
    let order: Vec<&str> = saved.iter().map(|command| command.name.as_str()).collect();
    if order != [STAMP, LIST_RENAMED, GROUP, BROKEN] {
        return Err(format!("the saved command order is {order:?}"));
    }
    let list = find_command(&saved, LIST_RENAMED)?;
    let list_id = list.id.as_str().to_owned();
    match &list.invocation {
        UserCommandInvocation::Program {
            executable,
            arguments,
        } if *executable == setup.sh
            && *arguments == ["-c", LIST_SCRIPT, "listing", "${selected.paths}"]
            && list.launch == UserCommandLaunch::Single
            && Path::new(&list.working_directory) == setup.out => {}
        other => {
            return Err(format!(
                "{LIST_RENAMED} saved as {other:?}, launch {:?}, folder {:?}",
                list.launch, list.working_directory
            ));
        }
    }
    let stamp = find_command(&saved, STAMP)?;
    match &stamp.invocation {
        UserCommandInvocation::Program {
            executable,
            arguments,
        } if *executable == setup.touch
            && *arguments == ["${file.name}.stamp"]
            && stamp.launch == UserCommandLaunch::PerItem => {}
        other => {
            return Err(format!(
                "{STAMP} saved as {other:?}, launch {:?}",
                stamp.launch
            ));
        }
    }
    if !matches!(
        find_command(&saved, GROUP)?.invocation,
        UserCommandInvocation::Separator
    ) {
        return Err(format!("{GROUP} was not saved as a separator"));
    }

    // The toolbar shows the commands in the saved order and the separator between
    // them (separators show only between commands).
    let names_on_bar = [STAMP, LIST_RENAMED, BROKEN];
    let bar = toolbar(&app, &names_on_bar);
    if bar != [STAMP, LIST_RENAMED, "|", BROKEN] {
        return Err(format!("the command toolbar shows {bar:?}"));
    }
    if button_caption(&app, LIST_RENAMED) != CAPTION
        || list.toolbar_label.as_deref() != Some(CAPTION)
    {
        return Err(format!(
            "the toolbar caption is {:?}, saved {:?}",
            button_caption(&app, LIST_RENAMED),
            list.toolbar_label
        ));
    }
    app.screenshot(
        context,
        "commands-toolbar",
        "The Custom Commands toolbar after OK",
    );

    // Grouped run: one process receives every selected path.
    select_files(&mut app, &["alpha.txt", "beta one.txt"])?;
    input::click_button(app.slint(), LIST_RENAMED)?;
    let listing = setup.out.join("selection.txt");
    let lines = wait_lines(&mut app, &listing, 2, "the grouped command")?;
    let expected = [
        setup.panel.join("alpha.txt"),
        setup.panel.join("beta one.txt"),
    ]
    .map(|path| path.to_string_lossy().into_owned());
    if lines != expected {
        return Err(format!(
            "the grouped command listed {lines:?} instead of {expected:?}"
        ));
    }

    // Per-item run: one process per selected file, the file name as its argument.
    select_files(&mut app, &["alpha.txt", "gamma.md"])?;
    input::click_button(app.slint(), STAMP)?;
    for name in ["alpha.txt.stamp", "gamma.md.stamp"] {
        options::wait_file(&mut app, &setup.out.join(name), "the per-item command")?;
    }
    if setup.out.join("beta one.txt.stamp").exists() {
        return Err("the per-item command also ran for an unselected file".into());
    }

    // A failing command reports why.
    input::click_button(app.slint(), BROKEN)?;
    let (title, body) = options::wait_notice(&mut app, "the failing command")?;
    if !title.contains("Custom Command") || !body.contains("missing-tool") {
        return Err(format!("the failing command reported {title:?}: {body:?}"));
    }
    app.screenshot(
        context,
        "commands-failure",
        "A Custom Command whose program is missing",
    );
    // Reported at the end, so that the remaining checks still run.
    let mut findings = Vec::new();
    if body.contains("external editor") {
        findings.push(format!(
            "finding OPT-CMDERR: the notice for a Custom Command whose program is missing names the operation \"external editor\": {body:?}"
        ));
    }
    options::dismiss_notice(&mut app)?;

    // Preview is a dry run: it shows the plan and starts nothing.
    fs::remove_file(&listing).map_err(|error| error.to_string())?;
    select_files(&mut app, &["gamma.md"])?;
    options::open(&mut app)?;
    options::page(&mut app, "Custom Commands")?;
    select_command(&mut app, LIST_RENAMED)?;
    options::press(&mut app, "Preview Custom Command")?;
    // The preview shows each argument Debug-escaped (USER_COMMAND_SETTINGS.md): quoted,
    // with a Windows path's `\` doubled.
    let gamma = setup.panel.join("gamma.md").to_string_lossy().into_owned();
    let gamma_shown = format!("{gamma:?}");
    let mut preview = Vec::new();
    for _ in 0..200 {
        // The full path appears only in the preview (the panel shows the name).
        preview = options::texts_with(&app, &gamma_shown);
        if !preview.is_empty() {
            break;
        }
        app.idle(Duration::from_millis(25));
    }
    if preview.is_empty() {
        return Err(format!(
            "the preview does not show the selected path {gamma:?}; visible text: {:?}",
            inspect::visible_texts(app.slint())
        ));
    }
    options::review_shot(
        &mut app,
        context,
        "commands-preview",
        "Custom Command preview (dry run)",
    );
    app.idle(Duration::from_millis(300));
    if listing.exists() {
        return Err("Preview ran the command".into());
    }

    // Cancel discards a staged new command.
    let saves = setup.store.saves();
    new_program(&mut app, DISCARDED, &setup.sh, &[], &setup.out, false)?;
    options::cancel(&mut app)?;
    if setup.store.saves() != saves
        || setup
            .store
            .current()
            .user_commands
            .iter()
            .any(|command| command.name == DISCARDED)
    {
        return Err("Cancel kept the staged command".into());
    }
    if inspect::find(app.slint(), AccessibleRole::Button, DISCARDED).is_some() {
        return Err("the toolbar shows the cancelled command".into());
    }

    // The keyboard editor lists the command as user-command.<id>; bind a key to it.
    options::open(&mut app)?;
    options::page(&mut app, "Keyboard")?;
    options::fill(&mut app, "Search commands", LIST_RENAMED)?;
    let entry = options::reveal(&mut app, AccessibleRole::ListItem, LIST_RENAMED)?;
    let action = entry
        .accessible_string_property(i_slint_core::accessibility::AccessibleStringProperty::Value)
        .map(|value| value.to_string())
        .unwrap_or_default();
    if action != format!("user-command.{list_id}") {
        return Err(format!(
            "the keyboard editor's entry is {action:?}, not user-command.{list_id}"
        ));
    }
    options::click_item(&mut app, &entry)?;
    let capture = options::reveal(&mut app, AccessibleRole::TextInput, "Press shortcut keys")?;
    options::click_item(&mut app, &capture)?;
    input::chord(app.slint(), &[Key::Control, Key::Alt], "j");
    app.idle(Duration::from_millis(60));
    let candidate = options::value(&app, AccessibleRole::TextInput, "Press shortcut keys");
    if candidate.as_deref() != Some(SHORTCUT) {
        return Err(format!("the captured shortcut is {candidate:?}"));
    }
    options::press(&mut app, "Assign")?;
    let assigned = options::value(
        &app,
        AccessibleRole::Combobox,
        "Shortcuts for selected command",
    );
    if assigned.as_deref() != Some(SHORTCUT) {
        return Err(format!(
            "after Assign the command's shortcut is {assigned:?}"
        ));
    }
    options::review_shot(
        &mut app,
        context,
        "commands-shortcut",
        "Keyboard: a key for the Custom Command",
    );
    options::ok(&mut app, &setup.store)?;
    let bound = setup
        .store
        .current()
        .shortcuts
        .iter()
        .any(|binding| binding.action == action && binding.keys == [SHORTCUT]);
    if !bound {
        return Err(format!(
            "the saved shortcuts lack {action} = {SHORTCUT}: {:?}",
            setup.store.current().shortcuts
        ));
    }
    // The key runs the command on the current selection.
    panel::focus_list(&mut app, PaneId::Left)?;
    input::chord(app.slint(), &[Key::Control, Key::Alt], "j");
    let lines = wait_lines(&mut app, &listing, 1, "the command's shortcut")?;
    if lines != [gamma.clone()] {
        return Err(format!("the shortcut's run listed {lines:?}"));
    }

    // F9 opens the User menu (it lists built-in actions; commands run from the toolbar).
    panel::focus_list(&mut app, PaneId::Left)?;
    input::key(app.slint(), Key::F9);
    app.until("the User menu", |app| inspect::roots(app.slint()).len() > 1);
    let menu = inspect::visible_texts(app.slint());
    if !menu.iter().any(|text| text == "User menu") {
        return Err(format!("F9 opened no User menu: {menu:?}"));
    }
    app.screenshot(context, "commands-f9", "F9 User menu");
    input::key(app.slint(), Key::Escape);
    app.until("the User menu to close", |app| {
        inspect::roots(app.slint()).len() == 1
    });

    // Restart: the commands, their order and the key are still there.
    app.shutdown()?;
    drop(app);
    let mut app = App::start(setup.config());
    let bar = toolbar(&app, &names_on_bar);
    if bar != [STAMP, LIST_RENAMED, "|", BROKEN] {
        return Err(format!("after a restart the command toolbar shows {bar:?}"));
    }
    fs::remove_file(&listing).map_err(|error| error.to_string())?;
    select_files(&mut app, &["beta one.txt"])?;
    input::chord(app.slint(), &[Key::Control, Key::Alt], "j");
    let lines = wait_lines(&mut app, &listing, 1, "the shortcut after a restart")?;
    let beta = setup
        .panel
        .join("beta one.txt")
        .to_string_lossy()
        .into_owned();
    if lines != [beta] {
        return Err(format!(
            "after a restart the shortcut's run listed {lines:?}"
        ));
    }

    // Delete the separator: Cancel keeps it, OK removes it.
    options::open(&mut app)?;
    options::page(&mut app, "Custom Commands")?;
    select_command(&mut app, GROUP)?;
    options::press(&mut app, "Delete Custom Command")?;
    if command_names(&app).iter().any(|name| name == GROUP) {
        return Err("Delete left the separator in the list".into());
    }
    options::cancel(&mut app)?;
    if toolbar(&app, &names_on_bar) != [STAMP, LIST_RENAMED, "|", BROKEN] {
        return Err("Cancel did not keep the deleted separator".into());
    }
    options::open(&mut app)?;
    options::page(&mut app, "Custom Commands")?;
    select_command(&mut app, GROUP)?;
    options::press(&mut app, "Delete Custom Command")?;
    options::ok(&mut app, &setup.store)?;
    let order: Vec<String> = setup
        .store
        .current()
        .user_commands
        .iter()
        .map(|command| command.name.clone())
        .collect();
    if order != [STAMP, LIST_RENAMED, BROKEN] {
        return Err(format!(
            "after deleting the separator the saved order is {order:?}"
        ));
    }
    let bar = toolbar(&app, &names_on_bar);
    if bar != [STAMP, LIST_RENAMED, BROKEN] {
        return Err(format!(
            "after deleting the separator the toolbar shows {bar:?}"
        ));
    }
    app.screenshot(
        context,
        "commands-final",
        "Custom Commands after deleting the separator",
    );
    context.note(format!(
        "Custom Commands: created grouped (${{selected.paths}}), per-item (${{file.name}}), separator and failing items; drag reorder, rename with a toolbar caption, Cancel and Delete verified; grouped run listed 2 paths, per-item run stamped 2 files, preview started nothing, the missing program raised a notice, {SHORTCUT} bound to user-command.{list_id} ran the command before and after a restart. F9 opens the User menu, which lists no Custom Commands (they run from the toolbar and keys)"
    ));
    app.shutdown()?;
    if findings.is_empty() {
        Ok(())
    } else {
        Err(findings.join("; "))
    }
}
