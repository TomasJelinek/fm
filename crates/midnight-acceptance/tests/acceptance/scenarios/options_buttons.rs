//! Every button on every Options page does something a user can see: each enabled
//! button of a freshly opened page is clicked, and a text, a control's value or state,
//! a popup, a window, a notice, the file chooser or a command to the backend must
//! change within a moment. Options is then closed with Escape, which discards the
//! session: nothing is ever saved. Disabled buttons are listed in the notes.
//!
//! Pages start with data that enables their buttons: a Custom Command, command history,
//! and the default colour, editor and comparator settings.
use crate::{
    Context,
    app::{App, Config},
    input, inspect,
    options_support::{self as options, PAGES, SettingsStore},
    tools_support::{self as tools},
};
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Clip},
};
use midnight_core::{
    CommandHistoryEntry, ShellFlavor, UserCommand, UserCommandId, UserCommandInvocation,
    UserCommandLaunch,
};
use slint::platform::{Key, WindowAdapter};
use std::{
    fs,
    sync::Arc,
    time::{Duration, Instant},
};

/// Footer and navigation buttons other scenarios cover; Reset to defaults may change
/// nothing when every setting is already a default.
const FOOTER: [&str; 4] = ["OK", "Cancel", "Reset to defaults", "Options topic"];

/// Buttons whose service this harness does not install, with the reason.
const NOT_HERE: [(&str, &str); 1] = [(
    "Clear thumbnail cache",
    "the thumbnail service is installed only in staged viewer processes; the button would clear a real cache",
)];

fn shown(item: &ItemRc) -> bool {
    let mut current = Some(item.clone());
    while let Some(node) = current {
        if let Some(clip) = node.downcast::<Clip>() {
            let clip = clip.as_pin_ref();
            if clip.is_visibility_clip() && clip.clip() {
                return false;
            }
        }
        current = node.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    true
}

fn property(item: &ItemRc, which: AccessibleStringProperty) -> String {
    item.accessible_string_property(which)
        .map(|value| value.to_string())
        .unwrap_or_default()
}

/// Everything a user can see of the windows' controls and texts, and what reached the
/// file chooser and the backend.
#[derive(Clone, PartialEq, Debug)]
struct Look {
    controls: Vec<String>,
    roots: usize,
    tools: Vec<String>,
    notice: Option<String>,
    picks: usize,
    options: bool,
    topic: i32,
}

fn look(app: &App) -> Look {
    let mut controls = Vec::new();
    // The topmost surface: Options itself, or what a button opened over it.
    if let Some(root) = inspect::roots(app.slint()).last() {
        inspect::walk(root, &mut |item| {
            if !shown(item) {
                return;
            }
            if let Some(text) = tools::text_of(item)
                && !text.trim().is_empty()
            {
                controls.push(format!("text {text}"));
                return;
            }
            let role = item.accessible_role();
            if matches!(
                role,
                AccessibleRole::Button
                    | AccessibleRole::Checkbox
                    | AccessibleRole::Combobox
                    | AccessibleRole::TextInput
                    | AccessibleRole::ListItem
                    | AccessibleRole::Switch
                    | AccessibleRole::Slider
                    | AccessibleRole::Spinbox
                    | AccessibleRole::Tab
                    | AccessibleRole::RadioButton
            ) {
                controls.push(format!(
                    "{role:?} {:?} value={:?} checked={:?} enabled={:?} selected={:?}",
                    inspect::label(item).unwrap_or_default(),
                    property(item, AccessibleStringProperty::Value),
                    property(item, AccessibleStringProperty::Checked),
                    property(item, AccessibleStringProperty::Enabled),
                    property(item, AccessibleStringProperty::ItemSelected),
                ));
            }
        });
    }
    Look {
        controls,
        roots: inspect::roots(app.slint()).len(),
        tools: tools::tool_titles(app),
        notice: tools::shown_notice(app).map(|(title, _)| title),
        picks: app.picker.requests().len(),
        options: app.window.get_options_open(),
        topic: app.window.get_options_topic_index(),
    }
}

/// The commands the presenter sent since `before` (navigation acknowledgements aside).
fn commands_since(before: &[String], app: &App) -> Vec<String> {
    let after = app.recent_events();
    let start = before
        .last()
        .and_then(|last| after.iter().rposition(|line| line == last))
        .map_or(0, |index| index + 1);
    after[start..]
        .iter()
        .filter(|line| line.starts_with('→') && !line.contains("PaneCommitted"))
        .cloned()
        .collect()
}

/// What changed between two looks, briefly.
fn change(before: &Look, after: &Look) -> Vec<String> {
    let mut changed = Vec::new();
    if before.roots != after.roots {
        changed.push(format!("popups {} → {}", before.roots - 1, after.roots - 1));
    }
    if before.tools != after.tools {
        changed.push(format!("windows {:?}", after.tools));
    }
    if before.notice != after.notice {
        changed.push(format!("notice {:?}", after.notice));
    }
    if before.picks != after.picks {
        changed.push("the file chooser".into());
    }
    if before.options != after.options {
        changed.push(format!("Options open={}", after.options));
    }
    if before.topic != after.topic {
        changed.push(format!("page {} → {}", before.topic, after.topic));
    }
    let added: Vec<&String> = after
        .controls
        .iter()
        .filter(|line| !before.controls.contains(line))
        .take(2)
        .collect();
    let removed = before
        .controls
        .iter()
        .filter(|line| !after.controls.contains(line))
        .count();
    if !added.is_empty() || removed > 0 {
        changed.push(format!("controls: {removed} gone, new {added:?}"));
    }
    changed
}

fn close_everything(app: &mut App) {
    for _ in 0..8 {
        if !app.window.get_options_open()
            && app.tool_windows().is_empty()
            && tools::shown_notice(app).is_none()
            && inspect::roots(app.slint()).len() == 1
        {
            return;
        }
        for window in app.tool_windows() {
            crate::app::layout_tool(&window, 900, 640);
            input::key(window.window(), Key::Escape);
        }
        input::key(app.slint(), Key::Escape);
        app.idle(Duration::from_millis(80));
    }
}

/// Open Options on `page`, from a closed state.
fn fresh(app: &mut App, page: &str) -> Result<(), String> {
    close_everything(app);
    if app.window.get_options_open() {
        options::cancel(app)?;
    }
    options::open(app)?;
    options::page(app, page)?;
    app.idle(Duration::from_millis(150));
    Ok(())
}

/// A shown button of the Options dialog (scrolled into view or not).
struct Button {
    label: String,
    enabled: bool,
    /// A selector button that is already the current one (a section tab).
    current: bool,
}

fn buttons(app: &App) -> Vec<Button> {
    let mut found: Vec<Button> = Vec::new();
    if let Some(dialog) = inspect::roots(app.slint()).last() {
        inspect::walk(dialog, &mut |item| {
            if item.accessible_role() == AccessibleRole::Button && shown(item) {
                let label = inspect::label(item).unwrap_or_default();
                if !found.iter().any(|known| known.label == label) {
                    found.push(Button {
                        enabled: property(item, AccessibleStringProperty::Enabled) != "false",
                        current: property(item, AccessibleStringProperty::Checked) == "true"
                            || property(item, AccessibleStringProperty::ItemSelected) == "true",
                        label,
                    });
                }
            }
        });
    }
    found
}

fn seed(
    command: &str,
    sh: std::path::PathBuf,
    out: &std::path::Path,
) -> Result<Arc<SettingsStore>, String> {
    let out = out.display().to_string();
    let command = command.to_owned();
    Ok(SettingsStore::seeded(move |preferences| {
        preferences.user_commands = Arc::new(vec![UserCommand {
            output: Default::default(),
            id: UserCommandId::new("note").expect("a valid command id"),
            name: command,
            enabled: true,
            toolbar_label: None,
            icon: None,
            working_directory: out,
            launch: UserCommandLaunch::Single,
            invocation: UserCommandInvocation::Program {
                executable: sh,
                arguments: vec!["-c".into(), "true".into()],
            },
        }]);
        preferences.command_history = vec![CommandHistoryEntry {
            command: "echo kept".into(),
            shell: ShellFlavor::Posix,
        }];
    }))
}

pub fn every_button(context: &mut Context) -> Result<(), String> {
    let folder = context.work.join("buttons");
    fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    fs::write(folder.join("file.txt"), "file\n").map_err(|error| error.to_string())?;
    let store = seed(
        "Harmless note",
        crate::tool::program(context, "sh")?,
        &folder,
    )?;
    let mut config = Config::new(&folder, &folder);
    config.settings = Some(store.clone());
    config.user_commands = true;
    let mut app = App::start(config);
    let mut failures: Vec<String> = Vec::new();
    let mut clicked = 0;
    let mut disabled: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let started = Instant::now();
    let titles: Vec<&str> = PAGES.iter().map(|(title, _)| *title).collect();
    for (page, _) in PAGES {
        fresh(&mut app, page)?;
        let listed = buttons(&app);
        for Button {
            label,
            enabled,
            current,
        } in listed
        {
            if titles.contains(&label.as_str()) || FOOTER.contains(&label.as_str()) {
                continue;
            }
            if current {
                skipped.push(format!("{page} › {label}: already the current choice"));
                continue;
            }
            if let Some((_, reason)) = NOT_HERE.iter().find(|(known, _)| *known == label) {
                skipped.push(format!("{page} › {label}: {reason}"));
                continue;
            }
            if !enabled {
                disabled.push(format!("{page} › {label}"));
                continue;
            }
            fresh(&mut app, page)?;
            // A quiet baseline: two looks a moment apart must agree, or a page still
            // settling would pass for the click's effect.
            let mut before = look(&app);
            let mut quiet = false;
            for _ in 0..8 {
                app.idle(Duration::from_millis(250));
                let again = look(&app);
                if again == before {
                    quiet = true;
                    break;
                }
                before = again;
            }
            if !quiet {
                failures.push(format!(
                    "{page} › {label}: the page keeps changing by itself for 2 s, so the click's effect cannot be judged"
                ));
                continue;
            }
            let trace = app.recent_events();
            if let Err(error) = options::press(&mut app, &label) {
                failures.push(format!("{page} › {label}: {error}"));
                continue;
            }
            let deadline = Instant::now() + Duration::from_millis(1500);
            let mut changed = Vec::new();
            // Sampled every few milliseconds: a short-lived status counts too.
            while Instant::now() < deadline {
                app.idle(Duration::from_millis(5));
                changed = change(&before, &look(&app));
                let sent = commands_since(&trace, &app);
                if !sent.is_empty() {
                    changed.push(format!("sent {}", sent.join(", ")));
                }
                if !changed.is_empty() {
                    break;
                }
            }
            if changed.is_empty() {
                failures.push(format!(
                    "{page} › {label}: clicking it changes nothing visible"
                ));
            } else {
                clicked += 1;
                context.note(format!("{page} › {label}: {}", changed.join("; ")));
            }
        }
    }
    close_everything(&mut app);
    if store.saves() != 0 {
        failures.push(format!(
            "Escape and Cancel saved settings {} times during the sweep",
            store.saves()
        ));
    }
    context.note(format!(
        "{clicked} buttons had a visible effect in {:.0} s; disabled at first: {disabled:?}; not clicked: {skipped:?}",
        started.elapsed().as_secs_f64()
    ));
    app.shutdown()?;
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}
