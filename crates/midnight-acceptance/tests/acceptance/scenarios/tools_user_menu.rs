//! The F9 User menu: its items in order (the built-in actions, then the configured
//! Custom Commands with a separator rule and a disabled command, then Close), each
//! built-in item doing what it says (the command editor, Manage Hot Paths, Properties,
//! folder sizes, Checksums, Close), the keyboard (Tab and Enter, Escape), a grouped
//! and a per-item command run from the menu on a multi-selection and on a folder shown
//! only inside an expanded tree branch (verified on disk), the disabled command running
//! nothing, and without commands the empty state whose Set up opens Options › Custom
//! Commands.
//!
//! The commands are harmless programs writing into the scenario's `out` folder
//! (`/bin/sh` with a literal script and `touch`), seeded into the settings before start.
use crate::{
    Context,
    app::{App, Config},
    input, inspect,
    options_support::{self as options, SettingsStore},
    panel,
    tools_support::{self as tools},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{PaneId, UserCommand, UserCommandId, UserCommandInvocation, UserCommandLaunch};
use midnight_ui::HotPathsPresentation;
use slint::{
    ComponentHandle, LogicalPosition,
    platform::{Key, WindowAdapter},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const LEFT: PaneId = PaneId::Left;
const LIST: &str = "List selection";
const STAMP: &str = "Stamp each";
const RULE: &str = "Group";
const DISABLED: &str = "Disabled tool";
/// One line per argument after `$0`, written into the working folder.
use crate::tool::LIST_SCRIPT;
const BUILT_IN: [&str; 5] = [
    "Command line",
    "Bookmarks and Hot Paths",
    "Properties · Alt+Enter",
    "Calculate / cancel folder sizes",
    "Checksums",
];
const FILES: [&str; 3] = ["alpha.txt", "beta one.txt", "gamma.md"];

struct Setup {
    panel: PathBuf,
    out: PathBuf,
    sh: PathBuf,
    touch: PathBuf,
}

fn command(
    id: &str,
    name: &str,
    launch: UserCommandLaunch,
    invocation: UserCommandInvocation,
    out: &Path,
) -> UserCommand {
    UserCommand {
        id: UserCommandId::new(id).expect("a valid command id"),
        name: name.to_owned(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: out.display().to_string(),
        launch,
        invocation,
    }
}

fn commands(setup: &Setup) -> Vec<UserCommand> {
    let mut disabled = command(
        "disabled",
        DISABLED,
        UserCommandLaunch::Single,
        UserCommandInvocation::Program {
            executable: setup.touch.clone(),
            arguments: vec!["disabled.flag".into()],
        },
        &setup.out,
    );
    disabled.enabled = false;
    vec![
        command(
            "list",
            LIST,
            UserCommandLaunch::Single,
            UserCommandInvocation::Program {
                executable: setup.sh.clone(),
                arguments: vec![
                    "-c".into(),
                    LIST_SCRIPT.into(),
                    "listing".into(),
                    "${selected.paths}".into(),
                ],
            },
            &setup.out,
        ),
        command(
            "stamp",
            STAMP,
            UserCommandLaunch::PerItem,
            UserCommandInvocation::Program {
                executable: setup.touch.clone(),
                arguments: vec!["${file.name}.stamp".into()],
            },
            &setup.out,
        ),
        command(
            "rule",
            RULE,
            UserCommandLaunch::Single,
            UserCommandInvocation::Separator,
            &setup.out,
        ),
        disabled,
    ]
}

fn start(setup: &Setup, with_commands: bool) -> (App, Arc<SettingsStore>) {
    let seeded = with_commands.then(|| commands(setup));
    let store = SettingsStore::seeded(move |preferences| {
        if let Some(commands) = seeded {
            preferences.user_commands = Arc::new(commands);
        }
    });
    let mut config = Config::new(&setup.panel, &setup.out);
    config.settings = Some(store.clone());
    config.user_commands = true;
    (App::start(config), store)
}

fn wait(app: &mut App, what: &str, condition: impl Fn(&App) -> bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.settle();
        if condition(app) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{what} did not happen; notice {:?}, tool windows {:?}",
                tools::shown_notice(app),
                tools::tool_titles(app)
            ));
        }
        app.idle(Duration::from_millis(10));
    }
}

fn menu_open(app: &App) -> bool {
    inspect::roots(app.slint()).len() > 1
        && inspect::visible_texts(app.slint())
            .iter()
            .any(|text| text == "User menu")
}

/// F9 in the focused left list.
fn open_menu(app: &mut App) -> Result<(), String> {
    panel::focus_list(app, LEFT)?;
    input::key(app.slint(), Key::F9);
    wait(app, "F9 to open the User menu", menu_open)
}

/// The menu's buttons top to bottom: (label, enabled).
fn menu_buttons(app: &App) -> Vec<(String, bool)> {
    let Some(menu) = inspect::roots(app.slint()).pop() else {
        return Vec::new();
    };
    let mut buttons = Vec::new();
    inspect::walk(&menu, &mut |item| {
        if item.accessible_role() == AccessibleRole::Button
            && let Some(rect) = inspect::visible_rect(item)
        {
            buttons.push((
                rect.1,
                inspect::label(item).unwrap_or_default(),
                tools::enabled(item),
            ));
        }
    });
    buttons.sort_by(|a, b| a.0.total_cmp(&b.0));
    buttons
        .into_iter()
        .map(|(_, label, enabled)| (label, enabled))
        .collect()
}

/// Nothing open over the panels and the list focused.
fn quiet(app: &App) -> bool {
    inspect::roots(app.slint()).len() == 1
        && tools::shown_notice(app).is_none()
        && app.tool_windows().is_empty()
        && !app.window.get_command_focused()
        && !app.window.get_options_open()
}

fn close_all(app: &mut App) -> Result<(), String> {
    for _ in 0..4 {
        if quiet(app) {
            break;
        }
        for window in app.tool_windows() {
            crate::app::layout_tool(&window, 900, 640);
            input::key(window.window(), Key::Escape);
        }
        app.idle(Duration::from_millis(60));
        input::key(app.slint(), Key::Escape);
        app.idle(Duration::from_millis(80));
    }
    wait(app, "everything to close", quiet)?;
    panel::focus_list(app, LEFT)
}

fn lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Wait (bounded) until `path` holds `count` lines.
fn wait_lines(app: &mut App, path: &Path, count: usize) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if lines(path).len() >= count {
            app.idle(Duration::from_millis(100));
            break;
        }
        app.idle(Duration::from_millis(25));
    }
    lines(path)
}

fn wait_files(app: &mut App, paths: &[PathBuf]) -> Vec<PathBuf> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && !paths.iter().all(|path| path.exists()) {
        app.idle(Duration::from_millis(25));
    }
    paths
        .iter()
        .filter(|path| !path.exists())
        .cloned()
        .collect()
}

fn mark(app: &mut App, name: &str) -> Result<(), String> {
    let position = panel::row_center(app, LEFT, name)
        .ok_or_else(|| format!("no {name:?} row in the left panel"))?;
    if app
        .last_click
        .is_some_and(|(when, _)| when.elapsed() < Duration::from_millis(650))
    {
        app.idle(Duration::from_millis(650));
    }
    input::click_with(app.slint(), &[Key::Control], position);
    app.last_click = Some((Instant::now(), position));
    app.settle();
    Ok(())
}

fn selected(app: &App) -> Vec<String> {
    let mut names: Vec<String> = app
        .rows(LEFT)
        .iter()
        .filter(|row| row.selected && !row.parent_entry)
        .map(|row| row.name.to_string())
        .collect();
    names.sort();
    names
}

/// Click the expander of `name` in the left panel.
fn expand(app: &mut App, name: &str) -> Result<(), String> {
    let wanted = name.to_owned();
    wait(app, &format!("{name}'s expander"), move |app| {
        app.rows(LEFT).iter().any(|row| {
            row.name == wanted && (row.tree_expanded || matches!(row.tree_disclosure, 1 | 3))
        })
    })?;
    let row = app
        .rows(LEFT)
        .into_iter()
        .find(|row| row.name == name)
        .ok_or_else(|| format!("no {name} row"))?;
    let verb = if row.tree_disclosure == 1 {
        "Discover"
    } else {
        "Expand"
    };
    let label = format!("{verb} folders in {name}");
    let expander = inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .find(|(text, _)| *text == label)
        .map(|(_, item)| item)
        .ok_or_else(|| format!("no {label:?} expander"))?;
    input::click_at(app.slint(), inspect::center(&expander));
    app.settle();
    Ok(())
}

pub fn user_menu(context: &mut Context) -> Result<(), String> {
    let root = context.work.join("user-menu");
    let setup = Setup {
        panel: root.join("panel"),
        out: root.join("out"),
        sh: crate::tool::program(context, "sh")?,
        touch: crate::tool::program(context, "touch")?,
    };
    for folder in ["project/sub-a", "project/sub-b"] {
        fs::create_dir_all(setup.panel.join(folder)).map_err(|error| error.to_string())?;
    }
    fs::write(setup.panel.join("project/sub-a/inside.txt"), "inside\n")
        .map_err(|error| error.to_string())?;
    for name in FILES {
        fs::write(setup.panel.join(name), name).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(&setup.out).map_err(|error| error.to_string())?;
    let (mut app, _store) = start(&setup, true);
    let mut failures: Vec<String> = Vec::new();

    // The items in order: built-in actions, the commands (the separator is a rule, the
    // disabled command is shown disabled), Close.
    open_menu(&mut app)?;
    let buttons = menu_buttons(&app);
    let labels: Vec<&str> = buttons.iter().map(|(label, _)| label.as_str()).collect();
    let mut wanted: Vec<&str> = BUILT_IN.to_vec();
    wanted.extend([LIST, STAMP, DISABLED, "Close"]);
    if labels != wanted {
        failures.push(format!(
            "the User menu lists {labels:?}, expected {wanted:?}"
        ));
    }
    let disabled: Vec<&str> = buttons
        .iter()
        .filter(|(_, enabled)| !enabled)
        .map(|(label, _)| label.as_str())
        .collect();
    if disabled != [DISABLED] {
        failures.push(format!("the User menu disables {disabled:?}"));
    }
    if inspect::visible_texts(app.slint())
        .iter()
        .any(|text| text == RULE)
    {
        failures.push(format!(
            "the separator {RULE:?} shows as text, not as a rule"
        ));
    }
    app.screenshot(context, "user-menu", "F9 User menu with Custom Commands");

    // The disabled command runs nothing and the menu stays open.
    let _ = input::click_button(app.slint(), DISABLED);
    app.idle(Duration::from_millis(500));
    if setup.out.join("disabled.flag").exists() {
        failures.push("clicking the disabled command ran it".into());
    }
    if !menu_open(&app) {
        failures.push("clicking the disabled command closed the menu".into());
    }

    // Escape closes the menu and changes nothing else.
    input::key(app.slint(), Key::Escape);
    wait(&mut app, "Escape to close the User menu", |app| {
        !menu_open(app)
    })?;
    close_all(&mut app)?;

    // Each built-in item.
    panel::select_row(&mut app, LEFT, "alpha.txt")?;
    open_menu(&mut app)?;
    input::click_button(app.slint(), "Command line")?;
    if wait(
        &mut app,
        "Command line to focus the command editor",
        |app| !menu_open(app) && app.window.get_command_focused(),
    )
    .is_err()
    {
        failures.push("User menu › Command line does not focus the command editor".into());
    }
    close_all(&mut app)?;
    open_menu(&mut app)?;
    input::click_button(app.slint(), "Bookmarks and Hot Paths")?;
    if wait(&mut app, "Manage Hot Paths", |app| {
        !app.window
            .global::<HotPathsPresentation>()
            .get_token()
            .is_empty()
    })
    .is_err()
    {
        failures.push("User menu › Bookmarks and Hot Paths opens no Hot Paths manager".into());
    }
    close_all(&mut app)?;
    panel::select_row(&mut app, LEFT, "alpha.txt")?;
    open_menu(&mut app)?;
    input::click_button(app.slint(), "Properties · Alt+Enter")?;
    match wait(&mut app, "Properties", |app| {
        tools::shown_notice(app).is_some()
    }) {
        Ok(()) => {
            let (title, body) = tools::shown_notice(&app).unwrap_or_default();
            if !body.contains("alpha.txt") && !title.contains("alpha.txt") {
                failures.push(format!(
                    "User menu › Properties shows {title:?} without the focused file: {body:?}"
                ));
            }
        }
        Err(error) => failures.push(format!("User menu › Properties: {error}")),
    }
    close_all(&mut app)?;
    open_menu(&mut app)?;
    input::click_button(app.slint(), "Calculate / cancel folder sizes")?;
    if wait(&mut app, "the size of project", |app| {
        app.rows(LEFT).iter().any(|row| {
            row.name == "project"
                && !row.size_running
                && row.size.chars().any(|c| c.is_ascii_digit())
        })
    })
    .is_err()
    {
        failures.push("User menu › Calculate folder sizes measures no folder".into());
    }
    close_all(&mut app)?;
    open_menu(&mut app)?;
    input::click_button(app.slint(), "Checksums")?;
    if wait(&mut app, "the Checksums window", |app| {
        tools::tool_titled(app, "Checksums · Midnight Explorer").is_some()
    })
    .is_err()
    {
        failures.push("User menu › Checksums opens no Checksums window".into());
    }
    close_all(&mut app)?;
    open_menu(&mut app)?;
    let before = (app.location(LEFT), selected(&app), app.cursor(LEFT));
    input::click_button(app.slint(), "Close")?;
    wait(&mut app, "Close to close the User menu", |app| {
        !menu_open(app)
    })?;
    app.idle(Duration::from_millis(200));
    if (app.location(LEFT), selected(&app), app.cursor(LEFT)) != before || !quiet(&app) {
        failures.push("User menu › Close changed something besides closing".into());
    }

    // The keyboard: Tab reaches the first item and Enter runs it.
    open_menu(&mut app)?;
    input::key(app.slint(), Key::Tab);
    input::key(app.slint(), Key::Return);
    if wait(&mut app, "Tab and Enter to run Command line", |app| {
        !menu_open(app) && app.window.get_command_focused()
    })
    .is_err()
    {
        failures.push("in the User menu Tab then Enter does not run its first item".into());
    }
    close_all(&mut app)?;

    // A multi-selection: the grouped command gets every marked path, the per-item one
    // runs once per file.
    for name in FILES {
        mark(&mut app, name)?;
    }
    if selected(&app) != FILES.map(str::to_owned).to_vec() {
        return Err(format!("marking the files selected {:?}", selected(&app)));
    }
    open_menu(&mut app)?;
    input::click_button(app.slint(), LIST)?;
    let listed = wait_lines(&mut app, &setup.out.join("selection.txt"), FILES.len());
    let mut expected: Vec<String> = FILES
        .iter()
        .map(|name| setup.panel.join(name).display().to_string())
        .collect();
    let mut sorted_listed = listed.clone();
    sorted_listed.sort();
    expected.sort();
    if sorted_listed != expected {
        failures.push(format!(
            "{LIST} from the User menu on three marked files wrote {listed:?}, expected {expected:?}"
        ));
    }
    open_menu(&mut app)?;
    input::click_button(app.slint(), STAMP)?;
    let stamps: Vec<PathBuf> = FILES
        .iter()
        .map(|name| setup.out.join(format!("{name}.stamp")))
        .collect();
    let missing = wait_files(&mut app, &stamps);
    if !missing.is_empty() {
        failures.push(format!(
            "{STAMP} from the User menu on three marked files did not create {missing:?}"
        ));
    }

    // A folder only visible in an expanded branch, focused without marks.
    close_all(&mut app)?;
    input::chord(app.slint(), &[Key::Control], "d");
    wait(&mut app, "the marks to clear", |app| {
        selected(app).is_empty()
    })?;
    expand(&mut app, "project")?;
    wait(&mut app, "sub-a in the expanded branch", |app| {
        app.rows(LEFT)
            .iter()
            .any(|row| row.name == "sub-a" && row.tree_depth == 1)
    })?;
    panel::select_row(&mut app, LEFT, "sub-a")?;
    let _ = fs::remove_file(setup.out.join("selection.txt"));
    open_menu(&mut app)?;
    input::click_button(app.slint(), LIST)?;
    let listed = wait_lines(&mut app, &setup.out.join("selection.txt"), 1);
    let nested = setup.panel.join("project").join("sub-a");
    if listed != [nested.display().to_string()] {
        failures.push(format!(
            "{LIST} on the tree row sub-a wrote {listed:?}, expected {}",
            nested.display()
        ));
    }
    open_menu(&mut app)?;
    input::click_button(app.slint(), STAMP)?;
    let missing = wait_files(&mut app, &[setup.out.join("sub-a.stamp")]);
    if !missing.is_empty() {
        failures.push("Stamp each on the tree row sub-a created no sub-a.stamp".into());
    }
    app.screenshot(
        context,
        "user-menu-tree",
        "Custom Commands run on a tree row",
    );
    app.shutdown()?;
    drop(app);

    // Without commands: the empty state and Set up.
    let (mut app, _store) = start(&setup, false);
    open_menu(&mut app)?;
    let texts = inspect::visible_texts(app.slint());
    if !texts
        .iter()
        .any(|text| text == "No Custom Commands configured.")
    {
        failures.push(format!("the User menu without commands shows {texts:?}"));
    }
    app.screenshot(
        context,
        "user-menu-empty",
        "F9 User menu without Custom Commands",
    );
    input::click_button(app.slint(), "Set up Custom Commands")?;
    if wait(&mut app, "Options › Custom Commands", |app| {
        app.window.get_options_open() && app.window.get_options_topic_index() == 9
    })
    .is_err()
    {
        failures.push("Set up in the User menu does not open Options › Custom Commands".into());
    } else {
        app.until("Options to be ready", |app| {
            app.window.get_preferences_ready() && !app.window.get_options_busy()
        });
        options::cancel(&mut app)?;
    }
    // A click outside the menu closes it.
    open_menu(&mut app)?;
    input::click_at(app.slint(), LogicalPosition::new(8.0, 8.0));
    if wait(&mut app, "a click outside to close the User menu", |app| {
        !menu_open(app)
    })
    .is_err()
    {
        failures.push("a click outside the User menu does not close it".into());
    }
    app.shutdown()?;
    if failures.is_empty() {
        context.note("F9: built-in items, keyboard, commands on marks and on a tree row, the disabled command, the empty state and Set up all did what they say");
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}
