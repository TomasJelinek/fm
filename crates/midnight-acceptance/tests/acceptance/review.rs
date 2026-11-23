//! UI review: the polish gate's sweep (docs/developer/acceptance/ui-polish.md). The main
//! window in several states, its menus and popups, every embedded dialog and the tool
//! windows are rendered across a matrix of themes, densities, window sizes (each window's
//! minimum size included) and scale factors, and every capture runs the gate's checks
//! (`scenarios::ui_gate`). Any flag fails the scenario unless the gate lists it as an
//! exemption with its reason.
//!
//! Dialogs open with their keyboard shortcut where one exists. Options, Hot Paths, Jobs
//! and About have no default shortcut; they open through the same window callbacks the
//! main menu invokes. Job progress windows, decision windows and the viewers have their
//! own gate scenarios (`scenarios::ui_states`).
use super::{
    Context, Scenario,
    app::{App, Config},
    input, inspect, panel, report,
    scenarios::ui_gate::{self as gate, Target},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{
    PaneId, PreferenceTheme, UiDensity, UserCommand, UserCommandId, UserCommandInvocation,
    UserCommandLaunch,
};
use midnight_ui::{FileListPresentation, HotPathsPresentation};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, software_renderer::MinimalSoftwareWindow},
};
use std::{
    path::Path,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

/// Window size class of a combination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// Each window's own minimum size.
    Min,
    Small,
    Normal,
    Large,
}

/// What a combination captures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Every surface of this sweep.
    Full,
    /// The main window's states and popups, one dialog, a notice and Find: the theme
    /// sweep, where colour is what differs.
    Colors,
}

#[derive(Clone, Copy, Debug)]
pub struct Combination {
    pub theme: PreferenceTheme,
    pub density: UiDensity,
    pub size: Size,
    pub scale: f32,
    pub scope: Scope,
}

impl Combination {
    const fn new(theme: PreferenceTheme, density: UiDensity, size: Size, scale: f32) -> Self {
        Self {
            theme,
            density,
            size,
            scale,
            scope: Scope::Full,
        }
    }

    pub fn prefix(&self) -> String {
        format!(
            "{:?}-{:?}-{:?}-x{}",
            self.theme, self.density, self.size, self.scale as u32
        )
    }

    /// The main window's logical size.
    pub fn main_size(&self) -> (usize, usize) {
        match self.size {
            Size::Min => (320, 200),
            Size::Small => (960, 600),
            Size::Normal => (1340, 820),
            Size::Large => (1600, 1000),
        }
    }
}

/// The default matrix: both densities at every size class (the windows' minimum sizes
/// included) in five themes with every surface, and scale 2 in both densities in the
/// colour scope.
pub fn default_matrix() -> Vec<Combination> {
    use PreferenceTheme::*;
    use UiDensity::*;
    let colors = |combination: Combination| Combination {
        scope: Scope::Colors,
        ..combination
    };
    vec![
        Combination::new(Blue, Desktop, Size::Normal, 1.0),
        Combination::new(Dark, Touch, Size::Small, 1.0),
        Combination::new(Light, Desktop, Size::Large, 1.0),
        Combination::new(HighContrastDark, Touch, Size::Normal, 1.0),
        Combination::new(Blue, Desktop, Size::Min, 1.0),
        Combination::new(Dark, Touch, Size::Min, 1.0),
        colors(Combination::new(Light, Desktop, Size::Normal, 2.0)),
        colors(Combination::new(HighContrastLight, Touch, Size::Small, 2.0)),
    ]
}

/// Every theme the default matrix leaves out, in the colour scope.
pub fn theme_matrix() -> Vec<Combination> {
    let covered: Vec<PreferenceTheme> = default_matrix().iter().map(|c| c.theme).collect();
    PreferenceTheme::ALL
        .iter()
        .filter(|theme| !covered.contains(theme))
        .map(|theme| Combination {
            scope: Scope::Colors,
            ..Combination::new(*theme, UiDensity::Desktop, Size::Normal, 1.0)
        })
        .collect()
}

/// The full matrix: every theme in both densities at the normal size, and the base
/// themes in both densities at every size class and both scale factors.
pub fn full_matrix() -> Vec<Combination> {
    use PreferenceTheme::*;
    let mut list = Vec::new();
    for theme in PreferenceTheme::ALL {
        for density in [UiDensity::Desktop, UiDensity::Touch] {
            list.push(Combination::new(theme, density, Size::Normal, 1.0));
        }
    }
    for theme in [Blue, Dark, Light, HighContrastDark, HighContrastLight] {
        for density in [UiDensity::Desktop, UiDensity::Touch] {
            for size in [Size::Min, Size::Small, Size::Normal, Size::Large] {
                for scale in [1.0, 2.0] {
                    if size == Size::Normal && scale == 1.0 {
                        continue;
                    }
                    list.push(Combination::new(theme, density, size, scale));
                }
            }
        }
    }
    list
}

/// Asks for the full matrix (`full`), one shard of it (`full:K/N`) or named combinations
/// (`full:Blue-Desktop-Normal-x1,…`).
const MATRIX: &str = "MIDNIGHT_ACCEPTANCE_UI_MATRIX";

/// Whether the full matrix, or a part of it, was asked for (`--all-themes`, or
/// [`MATRIX`] `full` or `full:…`).
pub fn full_requested(context: &Context) -> bool {
    context.options.all_themes
        || std::env::var(MATRIX).is_ok_and(|value| value == "full" || value.starts_with("full:"))
}

/// The part of the full matrix this run sweeps, and how to describe it: all of it, shard
/// K of N (`full:K/N`: the combinations whose position in [`full_matrix`], counted from 0,
/// leaves the remainder K − 1 when divided by N, so that every shard gets each size class
/// and scale factor and the N shards together sweep each combination once), or the
/// combinations named as [`Combination::prefix`] names them (`full:<name>,<name>…`).
fn full_part() -> Result<(Vec<Combination>, String), String> {
    let matrix = full_matrix();
    let value = std::env::var(MATRIX).unwrap_or_default();
    let Some(part) = value.strip_prefix("full:") else {
        return Ok((matrix, "every combination".to_owned()));
    };
    if let Some((shard, shards)) = part.split_once('/') {
        let shard: usize = shard.trim().parse().unwrap_or(0);
        let shards: usize = shards.trim().parse().unwrap_or(0);
        if shard == 0 || shard > shards {
            return Err(format!(
                "{MATRIX}={value}: name shard K of N as full:K/N with 1 <= K <= N"
            ));
        }
        let chosen = matrix
            .into_iter()
            .enumerate()
            .filter(|(position, _)| position % shards == shard - 1)
            .map(|(_, combination)| combination)
            .collect();
        return Ok((chosen, format!("shard {shard} of {shards}")));
    }
    let mut chosen = Vec::new();
    for name in part
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        let Some(combination) = matrix
            .iter()
            .find(|combination| combination.prefix() == name)
        else {
            return Err(format!(
                "{MATRIX}={value}: {name} is not a combination of the full matrix"
            ));
        };
        chosen.push(*combination);
    }
    if chosen.is_empty() {
        return Err(format!(
            "{MATRIX}={value}: name a shard (full:K/N) or combinations (full:<name>,…)"
        ));
    }
    Ok((chosen, "the named combinations".to_owned()))
}

/// Where an opened surface lives.
enum Host {
    /// A popup on the main window's adapter.
    Main,
    /// Its own platform window.
    Tool(Rc<MinimalSoftwareWindow>),
}

/// One combination's running sweep.
struct Sweep {
    combination: Combination,
    prefix: String,
}

impl Sweep {
    fn capture(
        &self,
        app: &mut App,
        context: &mut Context,
        host: &Host,
        name: &str,
        caption: &str,
    ) {
        let target = match host {
            Host::Main => Target::Main,
            Host::Tool(window) => Target::Tool(window.clone()),
        };
        let surface = format!("{}/{name}", self.prefix);
        let shot = format!("{}-{name}", self.prefix);
        gate::capture(
            app,
            context,
            &target,
            &surface,
            &shot,
            &format!("{caption} · {}", self.prefix),
        );
    }

    /// Size a tool window for this combination: its minimum, three quarters of its
    /// preferred size, its preferred size or 1.4 times that.
    fn size_tool(&self, window: &MinimalSoftwareWindow) {
        let (min, preferred) = gate::window_sizes(window.window());
        let size = match self.combination.size {
            Size::Min => min,
            Size::Small => (
                (preferred.0 * 0.75).max(min.0),
                (preferred.1 * 0.75).max(min.1),
            ),
            Size::Normal => preferred,
            Size::Large => (
                (preferred.0 * 1.4).clamp(min.0, 1700.0),
                (preferred.1 * 1.4).clamp(min.1, 1150.0),
            ),
        };
        gate::size_tool(window, size, self.combination.scale);
    }
}

/// How long a step waits for a dialog or popup: a stuck step fails fast and the sweep
/// goes on (`isolated`).
const STEP: Duration = Duration::from_secs(15);

/// Pump the application until `condition` holds, or panic after [`STEP`].
#[track_caller]
fn wait(app: &mut App, what: &str, condition: impl Fn(&App) -> bool) {
    app.until_for(what, STEP, condition);
}

fn has_popup(app: &App) -> bool {
    inspect::roots(app.slint()).len() > 1
}

fn main_idle(app: &App) -> bool {
    !app.window.get_embedded_modal_open()
        && !has_popup(app)
        && app.window.get_operation_token().is_empty()
        && app.window.get_copy_draft_token().is_empty()
        && app.window.get_metadata_token().is_empty()
        && !app.window.get_mutation_input_blocked()
        && app.tool_windows().is_empty()
        && super::tools_support::shown_notice(app).is_none()
}

/// Wait for a new tool window and size it for this combination.
fn tool(app: &mut App, sweep: &Sweep, what: &str, before: usize) -> Host {
    wait(app, what, move |app| app.tool_windows().len() > before);
    let window = app.tool_windows().pop().expect("tool window");
    sweep.size_tool(&window);
    app.settle();
    Host::Tool(window)
}

fn close(app: &mut App, host: &Host, what: &str) {
    match host {
        Host::Main => input::key(app.slint(), Key::Escape),
        Host::Tool(window) => input::key(window.window(), Key::Escape),
    }
    wait(app, &format!("{what} to close"), main_idle);
}

fn options_pages(app: &mut App, context: &mut Context, sweep: &Sweep) -> Result<(), String> {
    use super::options_support::PAGES;
    app.window.invoke_show_options();
    wait(app, "Options", |app| app.window.get_options_open());
    for (label, topic) in PAGES {
        if app.window.get_options_topic_index() != topic {
            if inspect::find(app.slint(), AccessibleRole::Button, label).is_some() {
                input::click_button(app.slint(), label)?;
            } else {
                // Narrow layouts replace the topic list with a combo box: open it and
                // move through the topics in their displayed order.
                let position =
                    |topic: i32| PAGES.iter().position(|&(_, id)| id == topic).unwrap_or(0);
                let (from, to) = (
                    position(app.window.get_options_topic_index()),
                    position(topic),
                );
                input::click(app.slint(), AccessibleRole::Combobox, "Options topic")?;
                app.settle();
                for _ in 0..from.abs_diff(to) {
                    input::key(
                        app.slint(),
                        if to > from {
                            Key::DownArrow
                        } else {
                            Key::UpArrow
                        },
                    );
                }
                input::key(app.slint(), Key::Return);
            }
            wait(app, &format!("the {label} Options page"), move |app| {
                app.window.get_options_topic_index() == topic
            });
        }
        sweep.capture(
            app,
            context,
            &Host::Main,
            &format!("options-{topic:02}"),
            &format!("Options › {label}"),
        );
        match topic {
            8 => colour_picker(app, context, sweep)?,
            3 => keyboard_questions(app, context, sweep)?,
            _ => {}
        }
    }
    close(app, &Host::Main, "Options");
    Ok(())
}

/// Options › Colors: the colour picker of the first rule's foreground, closed with
/// Escape (which closes only the picker).
fn colour_picker(app: &mut App, context: &mut Context, sweep: &Sweep) -> Result<(), String> {
    use super::options_support as options;
    options::click_row(app, "Filename color rules", "first rule", &|label| {
        label.starts_with("1. ")
    })?;
    options::press(app, "Choose Foreground")?;
    wait(app, "the colour picker", |app| {
        options::find_any(app, AccessibleRole::Button, "Use color").is_some()
    });
    sweep.capture(
        app,
        context,
        &Host::Main,
        "options-08-picker",
        "Options › Colors › the colour picker",
    );
    input::key(app.slint(), Key::Escape);
    wait(app, "the colour picker to close", |app| {
        options::find_any(app, AccessibleRole::Button, "Use color").is_none()
    });
    Ok(())
}

/// Options › Keyboard: the Replace shortcut question (F5 for Create folder) and the
/// Restore all defaults confirmation; both answered so that nothing changes.
fn keyboard_questions(app: &mut App, context: &mut Context, sweep: &Sweep) -> Result<(), String> {
    use super::options_support as options;
    options::fill(app, "Search commands", "Create folder")?;
    let entry = options::reveal(app, AccessibleRole::ListItem, "Create folder")?;
    options::click_item(app, &entry)?;
    let field = options::reveal(app, AccessibleRole::TextInput, "Press shortcut keys")?;
    options::click_item(app, &field)?;
    input::key(app.slint(), Key::F5);
    app.settle();
    options::press(app, "Assign")?;
    let open = |app: &App| {
        inspect::find(app.slint(), AccessibleRole::Groupbox, "Replace shortcut").is_some()
    };
    wait(app, "the Replace shortcut question", open);
    sweep.capture(
        app,
        context,
        &Host::Main,
        "options-03-replace",
        "Options › Keyboard › Replace shortcut",
    );
    input::key(app.slint(), Key::Escape);
    wait(app, "the question to close", move |app| !open(app));
    options::press(app, "Restore all defaults")?;
    sweep.capture(
        app,
        context,
        &Host::Main,
        "options-03-restore-all",
        "Options › Keyboard › Restore all defaults, asking",
    );
    options::press(app, "Keep custom shortcuts")?;
    Ok(())
}

type Step<'a> = Box<dyn FnOnce(&mut App, &mut Context) -> Result<(), String> + 'a>;

/// Run one step; a failure (or panic) becomes a review flag and the sweep goes on after
/// Escape has closed whatever the step left open.
fn isolated(app: &mut App, context: &mut Context, prefix: &str, name: &str, step: Step<'_>) {
    let began = Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| step(app, context)));
    if gate::profiling() {
        context.note(format!(
            "profile step {prefix}/{name}: {} ms",
            began.elapsed().as_millis()
        ));
    }
    let error = match result {
        Ok(Ok(())) => return,
        Ok(Err(error)) => error,
        Err(panic) => panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_owned()))
            .unwrap_or_else(|| "panicked".to_owned()),
    };
    let first = error.lines().next().unwrap_or_default().to_owned();
    context.outcome.flags.push(report::Flag {
        shot: format!("shots/{}/{prefix}-main.png", super::app::context_name()),
        kind: "dialog step failed",
        detail: format!("{prefix}/{name}: {first}"),
    });
    context.note(format!("{prefix} {name} failed: {error}"));
    for _ in 0..3 {
        for window in app.tool_windows() {
            input::key(window.window(), Key::Escape);
        }
        input::key(app.slint(), Key::Escape);
        app.idle(Duration::from_millis(100));
    }
    super::files_support::dismiss_all(app);
}

/// A dialog opened by a shortcut: screenshot name, caption, modifiers, key and the
/// condition that shows it opened.
type ShortcutDialog = (
    &'static str,
    &'static str,
    &'static [Key],
    Key,
    fn(&App) -> bool,
);

fn shortcut_dialogs() -> [ShortcutDialog; 9] {
    [
        ("copy", "Copy (F5)", &[], Key::F5, |app| {
            !app.window.get_copy_draft_token().is_empty()
        }),
        ("move", "Move (F6)", &[], Key::F6, |app| {
            !app.window.get_copy_draft_token().is_empty()
        }),
        ("trash", "Delete to Trash (F8)", &[], Key::F8, |app| {
            !app.window.get_operation_token().is_empty()
        }),
        (
            "delete",
            "Permanent delete (Shift+F8)",
            &[Key::Shift],
            Key::F8,
            |app| !app.window.get_operation_token().is_empty(),
        ),
        ("rename", "Rename (F2)", &[], Key::F2, |app| {
            !app.window.get_operation_token().is_empty()
        }),
        ("mkdir", "Create directory (F7)", &[], Key::F7, |app| {
            !app.window.get_operation_token().is_empty()
        }),
        (
            "newfile",
            "New empty file (Shift+F4)",
            &[Key::Shift],
            Key::F4,
            |app| !app.window.get_operation_token().is_empty(),
        ),
        (
            "compare-dirs",
            "Compare directories (Ctrl+F10)",
            &[Key::Control],
            Key::F10,
            |app| app.window.get_comparison_view().open,
        ),
        (
            "panel-filter",
            "Panel filter (Ctrl+F12)",
            &[Key::Control],
            Key::F12,
            |app| app.window.get_panel_filter_view().open,
        ),
    ]
}

/// A popup of the main window opened by a key: screenshot name, caption, modifiers, key.
const POPUPS: [(&str, &str, &[Key], Key); 3] = [
    ("main-menu", "Main menu (F10)", &[], Key::F10),
    ("user-menu", "User menu (F9)", &[], Key::F9),
    ("volumes", "Volume choices (Alt+F1)", &[Key::Alt], Key::F1),
];

/// The main window's own states: the cursor, marked rows and a filename search.
fn main_states(app: &mut App, context: &mut Context, sweep: &Sweep) {
    let left = PaneId::Left;
    let prefix = sweep.prefix.clone();
    isolated(
        app,
        context,
        &prefix,
        "marked rows",
        Box::new(|app: &mut App, context: &mut Context| {
            panel::select_row(app, left, "alpha.bin")?;
            input::key(app.slint(), Key::Insert);
            input::key(app.slint(), Key::Insert);
            app.settle();
            sweep.capture(
                app,
                context,
                &Host::Main,
                "main-marked",
                "Marked rows and the cursor",
            );
            input::chord(app.slint(), &[Key::Control], "d");
            app.settle();
            Ok(())
        }),
    );
    isolated(
        app,
        context,
        &prefix,
        "filename search",
        Box::new(|app: &mut App, context: &mut Context| {
            panel::select_row(app, left, "alpha.bin")?;
            input::type_text(app.slint(), "be");
            app.settle();
            sweep.capture(app, context, &Host::Main, "main-search", "Filename search");
            input::key(app.slint(), Key::Escape);
            app.settle();
            Ok(())
        }),
    );
    for (name, caption, modifiers, key) in POPUPS {
        isolated(
            app,
            context,
            &prefix,
            caption,
            Box::new(move |app: &mut App, context: &mut Context| {
                panel::focus_list(app, left)?;
                input::chord(app.slint(), modifiers, key);
                wait(app, caption, has_popup);
                sweep.capture(app, context, &Host::Main, name, caption);
                close(app, &Host::Main, caption);
                Ok(())
            }),
        );
    }
    isolated(
        app,
        context,
        &prefix,
        "main menu categories",
        Box::new(|app: &mut App, context: &mut Context| main_menu_categories(app, context, sweep)),
    );
    for (name, label) in PANEL_MENUS {
        isolated(
            app,
            context,
            &prefix,
            label,
            Box::new(move |app: &mut App, context: &mut Context| {
                panel::focus_list(app, left)?;
                if super::options_support::find_any(app, AccessibleRole::Combobox, label).is_none()
                {
                    context.note(format!(
                        "{}: the {label:?} picker is not shown at this size",
                        sweep.prefix
                    ));
                    return Ok(());
                }
                input::click(app.slint(), AccessibleRole::Combobox, label)?;
                wait(app, label, has_popup);
                sweep.capture(app, context, &Host::Main, name, label);
                close(app, &Host::Main, label);
                Ok(())
            }),
        );
    }
    // A compact window moves the Custom Commands toolbar into a popup behind its
    // "Custom commands" button (wider windows show the toolbar in every capture).
    isolated(
        app,
        context,
        &prefix,
        "Custom commands",
        Box::new(|app: &mut App, context: &mut Context| {
            let Some(button) =
                super::options_support::find_any(app, AccessibleRole::Button, "Custom commands")
            else {
                return Ok(());
            };
            // At the minimum size the toolbar clips the button away (no scroll view).
            if inspect::visible_rect(&button).is_none() {
                context.note(format!(
                    "SKIP: {}: the \"Custom commands\" button is clipped away at this size",
                    sweep.prefix
                ));
                return Ok(());
            }
            panel::focus_list(app, left)?;
            super::options_support::click_item(app, &button)?;
            wait(app, "the Custom commands popup", has_popup);
            sweep.capture(
                app,
                context,
                &Host::Main,
                "custom-commands",
                "Custom commands (compact window)",
            );
            close(app, &Host::Main, "the Custom commands popup");
            Ok(())
        }),
    );
}

/// Two Custom Commands for the toolbar and its compact popup; never run (this sweep
/// installs no launcher).
fn seeded_commands(folder: &Path) -> Arc<super::options_support::SettingsStore> {
    let program = folder.join("never-run");
    super::options_support::SettingsStore::seeded(move |preferences| {
        preferences.user_commands = Arc::new(
            [("build", "Build project"), ("deploy", "Deploy preview")]
                .into_iter()
                .map(|(id, name)| UserCommand {
                    output: Default::default(),
                    id: UserCommandId::new(id).expect("a valid command id"),
                    name: name.into(),
                    enabled: true,
                    toolbar_label: None,
                    icon: None,
                    working_directory: "${active.path}".into(),
                    launch: UserCommandLaunch::Single,
                    invocation: UserCommandInvocation::Program {
                        executable: program.clone(),
                        arguments: Vec::new(),
                    },
                })
                .collect(),
        );
    })
}

/// The left panel's menus: screenshot name and the picker's accessible name.
const PANEL_MENUS: [(&str, &str); 2] = [
    ("panel-view-menu", "Left panel view mode"),
    ("panel-sort-menu", "Left panel sort"),
];

/// F10's categories other than "All actions", chosen with Down in its category picker
/// (which has the keyboard when the menu opens).
fn main_menu_categories(app: &mut App, context: &mut Context, sweep: &Sweep) -> Result<(), String> {
    panel::focus_list(app, PaneId::Left)?;
    input::key(app.slint(), Key::F10);
    wait(app, "the main menu", has_popup);
    for category in ["files", "selection", "panels", "tools", "application"] {
        input::key(app.slint(), Key::DownArrow);
        app.settle();
        sweep.capture(
            app,
            context,
            &Host::Main,
            &format!("main-menu-{category}"),
            &format!("Main menu (F10), category {category}"),
        );
    }
    close(app, &Host::Main, "the main menu");
    Ok(())
}

/// An unreadable folder with a long name inside the review folder: entering it shows
/// the error notice that names a long path.
fn locked_folder(left: &Path) -> Result<String, String> {
    let name = "a-folder-whose-long-name-makes-the-error-path-long-enough-to-wrap-in-a-notice";
    let path = left.join(name);
    std::fs::create_dir_all(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    super::fixtures::deny_read(&path)?;
    Ok(name.to_owned())
}

/// The error notice of a folder that cannot be listed, naming its long path, then its
/// details.
fn error_notice(
    app: &mut App,
    context: &mut Context,
    sweep: &Sweep,
    locked: &str,
) -> Result<(), String> {
    panel::select_row(app, PaneId::Left, locked)?;
    input::key(app.slint(), Key::Return);
    wait(app, "an error notice", |app| {
        super::tools_support::shown_notice(app).is_some()
    });
    sweep.capture(
        app,
        context,
        &Host::Main,
        "notice-error",
        "Error notice with a long path",
    );
    // A folder notice always offers Details (as a glyph where its caption does not fit):
    // a missing one fails the step instead of skipping the capture.
    if super::options_support::find_any(app, AccessibleRole::Button, "Details").is_none() {
        input::key(app.slint(), Key::Escape);
        return Err("the error notice offers no Details".to_owned());
    }
    input::click_button(app.slint(), "Details")?;
    app.settle();
    sweep.capture(
        app,
        context,
        &Host::Main,
        "notice-details",
        "Error notice, details shown",
    );
    input::key(app.slint(), Key::Escape);
    wait(app, "the notice to close", |app| {
        super::tools_support::shown_notice(app).is_none()
    });
    Ok(())
}

fn about(app: &mut App, context: &mut Context, sweep: &Sweep) -> Result<(), String> {
    app.window.invoke_show_about();
    wait(app, "About", |app| app.window.get_about_open());
    sweep.capture(app, context, &Host::Main, "about", "About");
    input::click(app.slint(), AccessibleRole::Tab, "Licenses")?;
    app.settle();
    sweep.capture(
        app,
        context,
        &Host::Main,
        "about-licenses",
        "About › Licenses",
    );
    input::click(app.slint(), AccessibleRole::TextInput, "Filter libraries")?;
    input::key(app.slint(), Key::DownArrow);
    input::key(app.slint(), Key::Return);
    app.settle();
    sweep.capture(
        app,
        context,
        &Host::Main,
        "about-license",
        "About › Licenses › a license",
    );
    for _ in 0..3 {
        if !app.window.get_about_open() {
            break;
        }
        input::key(app.slint(), Key::Escape);
        app.idle(Duration::from_millis(100));
    }
    wait(app, "About to close", main_idle);
    Ok(())
}

/// Open, capture and close every embedded dialog and tool window.
fn dialogs(app: &mut App, context: &mut Context, sweep: &Sweep, locked: &str) {
    let left = PaneId::Left;
    let prefix = sweep.prefix.clone();
    let colors = sweep.combination.scope == Scope::Colors;
    isolated(
        app,
        context,
        &prefix,
        "options",
        Box::new(|app: &mut App, context: &mut Context| {
            if colors {
                return Ok(());
            }
            options_pages(app, context, sweep)
        }),
    );
    for (name, caption, modifiers, key, ready) in shortcut_dialogs() {
        if colors && name != "copy" {
            continue;
        }
        isolated(
            app,
            context,
            &prefix,
            caption,
            Box::new(move |app: &mut App, context: &mut Context| {
                panel::select_row(app, left, "alpha.bin")?;
                input::chord(app.slint(), modifiers, key);
                wait(app, caption, ready);
                sweep.capture(app, context, &Host::Main, name, caption);
                close(app, &Host::Main, caption);
                Ok(())
            }),
        );
    }
    isolated(
        app,
        context,
        &prefix,
        "error notice",
        Box::new(|app: &mut App, context: &mut Context| error_notice(app, context, sweep, locked)),
    );
    if colors {
        isolated(
            app,
            context,
            &prefix,
            "Find",
            Box::new(|app: &mut App, context: &mut Context| {
                panel::focus_list(app, left)?;
                let before = app.tool_windows().len();
                input::chord(app.slint(), &[Key::Alt], Key::F7);
                let host = tool(app, sweep, "Find", before);
                sweep.capture(app, context, &host, "find", "Find (Alt+F7)");
                close(app, &host, "Find");
                Ok(())
            }),
        );
        return;
    }
    isolated(
        app,
        context,
        &prefix,
        "Make File List",
        Box::new(|app: &mut App, context: &mut Context| {
            panel::select_row(app, left, "alpha.bin")?;
            input::chord(app.slint(), &[Key::Control], "m");
            wait(app, "Make File List", |app| {
                app.window.global::<FileListPresentation>().get_active()
            });
            sweep.capture(
                app,
                context,
                &Host::Main,
                "file-list",
                "Make File List (Ctrl+M)",
            );
            close(app, &Host::Main, "Make File List");
            Ok(())
        }),
    );
    isolated(
        app,
        context,
        &prefix,
        "Attributes",
        Box::new(|app: &mut App, context: &mut Context| {
            panel::select_row(app, left, "alpha.bin")?;
            input::chord(app.slint(), &[Key::Control], Key::F2);
            wait(app, "Attributes", |app| {
                !app.window.get_metadata_token().is_empty()
            });
            sweep.capture(
                app,
                context,
                &Host::Main,
                "attributes",
                "Attributes (Ctrl+F2)",
            );
            input::click_button(app.slint(), "Times")?;
            app.settle();
            sweep.capture(app, context, &Host::Main, "times", "Attributes › Times");
            // The calendar and the time picker of Modified, each closed with Escape
            // (which keeps the editor open).
            for (button, name, caption) in [
                (
                    "Modified: choose date",
                    "times-calendar",
                    "Attributes › Times › the calendar",
                ),
                (
                    "Modified: choose time",
                    "times-clock",
                    "Attributes › Times › the time picker",
                ),
            ] {
                let popups = inspect::roots(app.slint()).len();
                // Scrolled into view first: a small window shows only part of the page.
                let opener = super::options_support::reveal(app, AccessibleRole::Button, button)?;
                super::options_support::click_item(app, &opener)?;
                wait(app, caption, move |app| {
                    inspect::roots(app.slint()).len() > popups
                });
                sweep.capture(app, context, &Host::Main, name, caption);
                input::key(app.slint(), Key::Escape);
                wait(app, &format!("{caption} to close"), move |app| {
                    inspect::roots(app.slint()).len() == popups
                });
            }
            close(app, &Host::Main, "Attributes");
            Ok(())
        }),
    );
    isolated(
        app,
        context,
        &prefix,
        "Case",
        Box::new(|app: &mut App, context: &mut Context| {
            panel::select_row(app, left, "alpha.bin")?;
            input::chord(app.slint(), &[Key::Control], Key::F7);
            wait(app, "the Case page", |app| {
                !app.window.get_metadata_token().is_empty()
            });
            sweep.capture(
                app,
                context,
                &Host::Main,
                "case",
                "Change name case (Ctrl+F7)",
            );
            close(app, &Host::Main, "the Case page");
            Ok(())
        }),
    );
    isolated(
        app,
        context,
        &prefix,
        "Select by mask",
        Box::new(|app: &mut App, context: &mut Context| {
            panel::focus_list(app, left)?;
            // Ctrl+= through the window's physical-shortcut entry, as the native key
            // hook delivers it.
            app.window
                .invoke_physical_shortcut("Equal".into(), true, false, false, false);
            wait(app, "Select by mask", |app| {
                has_popup(app) || app.window.get_embedded_modal_open()
            });
            sweep.capture(
                app,
                context,
                &Host::Main,
                "select-mask",
                "Select by mask (Ctrl+=)",
            );
            close(app, &Host::Main, "Select by mask");
            Ok(())
        }),
    );
    isolated(
        app,
        context,
        &prefix,
        "Links",
        Box::new(|app: &mut App, context: &mut Context| {
            panel::select_row(app, left, "alpha.bin")?;
            input::key(app.slint(), Key::F5);
            wait(app, "Copy", |app| {
                !app.window.get_copy_draft_token().is_empty()
            });
            let popups = inspect::roots(app.slint()).len();
            // Scrolled into view first: a small window shows only part of the dialog.
            let links = super::options_support::reveal(app, AccessibleRole::Combobox, "Links")?;
            super::options_support::click_item(app, &links)?;
            wait(app, "the Links list", move |app| {
                inspect::roots(app.slint()).len() > popups
            });
            sweep.capture(
                app,
                context,
                &Host::Main,
                "copy-links",
                "Copy (F5) › the Links list",
            );
            input::key(app.slint(), Key::Escape);
            wait(app, "the Links list to close", move |app| {
                inspect::roots(app.slint()).len() <= popups
            });
            close(app, &Host::Main, "Copy");
            Ok(())
        }),
    );
    isolated(
        app,
        context,
        &prefix,
        "Hot Paths",
        Box::new(|app: &mut App, context: &mut Context| {
            panel::focus_list(app, left)?;
            app.window.invoke_show_bookmarks(left.index() as i32);
            wait(app, "Hot Paths", |app| {
                !app.window
                    .global::<HotPathsPresentation>()
                    .get_token()
                    .is_empty()
            });
            sweep.capture(app, context, &Host::Main, "hot-paths", "Hot Paths");
            close(app, &Host::Main, "Hot Paths");
            Ok(())
        }),
    );
    isolated(
        app,
        context,
        &prefix,
        "About",
        Box::new(|app: &mut App, context: &mut Context| about(app, context, sweep)),
    );
    isolated(
        app,
        context,
        &prefix,
        "Drive information",
        Box::new(|app: &mut App, context: &mut Context| {
            panel::focus_list(app, left)?;
            input::chord(app.slint(), &[Key::Control], Key::F1);
            wait(app, "Drive information", |app| {
                app.window.get_embedded_modal_open() || has_popup(app)
            });
            app.idle(Duration::from_millis(300));
            sweep.capture(
                app,
                context,
                &Host::Main,
                "drive-info",
                "Drive information (Ctrl+F1)",
            );
            close(app, &Host::Main, "Drive information");
            Ok(())
        }),
    );
    isolated(
        app,
        context,
        &prefix,
        "Properties",
        Box::new(|app: &mut App, context: &mut Context| {
            panel::select_row(app, left, "alpha.bin")?;
            input::chord(app.slint(), &[Key::Alt], Key::Return);
            wait(app, "Properties", |app| {
                app.window.get_embedded_modal_open()
                    || has_popup(app)
                    || super::tools_support::shown_notice(app).is_some()
            });
            app.idle(Duration::from_millis(300));
            sweep.capture(
                app,
                context,
                &Host::Main,
                "properties",
                "Properties (Alt+Enter)",
            );
            close(app, &Host::Main, "Properties");
            Ok(())
        }),
    );
    for (name, caption, modifiers, key) in [
        ("find", "Find (Alt+F7)", &[Key::Alt][..], Key::F7),
        (
            "checksums",
            "Checksums (Ctrl+F8)",
            &[Key::Control][..],
            Key::F8,
        ),
        ("help", "Help (F1)", &[][..], Key::F1),
    ] {
        isolated(
            app,
            context,
            &prefix,
            caption,
            Box::new(move |app: &mut App, context: &mut Context| {
                panel::focus_list(app, left)?;
                let before = app.tool_windows().len();
                input::chord(app.slint(), modifiers, key);
                let host = tool(app, sweep, caption, before);
                sweep.capture(app, context, &host, name, caption);
                // A footer too narrow for its actions moves some into More…: its list,
                // closed with Escape (which keeps the window open).
                if let Host::Tool(window) = &host
                    && inspect::find(window.window(), AccessibleRole::Button, "More…").is_some()
                {
                    let popups = inspect::roots(window.window()).len();
                    input::click_button(window.window(), "More…")?;
                    let shown = window.clone();
                    wait(app, &format!("{caption} › More…"), move |_| {
                        inspect::roots(shown.window()).len() > popups
                    });
                    sweep.capture(
                        app,
                        context,
                        &host,
                        &format!("{name}-more"),
                        &format!("{caption} › More…"),
                    );
                    input::key(window.window(), Key::Escape);
                    let shown = window.clone();
                    wait(app, &format!("{caption} › More… to close"), move |_| {
                        inspect::roots(shown.window()).len() <= popups
                    });
                }
                close(app, &host, caption);
                Ok(())
            }),
        );
    }
    isolated(
        app,
        context,
        &prefix,
        "Jobs",
        Box::new(|app: &mut App, context: &mut Context| {
            let before = app.tool_windows().len();
            app.window.invoke_show_operations();
            let host = tool(app, sweep, "Jobs", before);
            sweep.capture(app, context, &host, "jobs", "Jobs");
            close(app, &host, "Jobs");
            Ok(())
        }),
    );
}

/// Sweep `combinations`: start the application once per combination (the first loaded
/// preferences choose theme and density), size and scale it, and capture every surface
/// of the combination's scope.
pub fn sweep(context: &mut Context, combinations: &[Combination]) -> Result<usize, String> {
    // Dialogs are cancelled, never confirmed, but work on a copy all the same.
    let left = context.work.join("review");
    if !left.exists() {
        super::scenarios::transfers::copy_tree(&context.fixture.path("checksums"), &left)?;
    }
    let locked = locked_folder(&left)?;
    let right = context.fixture.root.clone();
    let mut reviewed = 0;
    for combination in combinations {
        let began = Instant::now();
        let (width, height) = combination.main_size();
        let mut config = Config::new(&left, &right);
        config.theme = combination.theme;
        config.density = combination.density;
        config.width = width;
        config.height = height;
        config.settings = Some(seeded_commands(&left));
        let mut app = App::start(config);
        let started = began.elapsed();
        gate::scale_main(&mut app, (width, height), combination.scale);
        let scaled = began.elapsed();
        gate::take_profile();
        let sweep = Sweep {
            combination: *combination,
            prefix: combination.prefix(),
        };
        let caption = format!(
            "Main window · {:?} · {:?} · {width}×{height} · scale {}",
            combination.theme, combination.density, combination.scale
        );
        sweep.capture(&mut app, context, &Host::Main, "main", &caption);
        main_states(&mut app, context, &sweep);
        dialogs(&mut app, context, &sweep, &locked);
        let swept = began.elapsed();
        app.shutdown()?;
        if gate::profiling() {
            let total = began.elapsed();
            let captures = gate::take_profile();
            let surfaces = swept.saturating_sub(scaled);
            context.note(format!(
                "profile {}: total {} ms = App start {} ms + size and scale {} ms + surfaces {} \
                 ms + shutdown {} ms; surfaces = {} captures ({}) + opening and closing them {} ms",
                sweep.prefix,
                total.as_millis(),
                started.as_millis(),
                scaled.saturating_sub(started).as_millis(),
                surfaces.as_millis(),
                total.saturating_sub(swept).as_millis(),
                captures.captures,
                captures.describe(),
                surfaces.saturating_sub(captures.total()).as_millis(),
            ));
        }
        reviewed += 1;
    }
    Ok(reviewed)
}

/// Scenario `ui_review`: the default matrix.
pub fn run(context: &mut Context) -> Result<(), String> {
    let matrix = default_matrix();
    let reviewed = sweep(context, &matrix)?;
    context.note(format!(
        "{reviewed} combinations ({}), {} screenshots",
        matrix
            .iter()
            .map(Combination::prefix)
            .collect::<Vec<_>>()
            .join(", "),
        context.outcome.shots.len()
    ));
    gate::verdict(context, "ui_review")
}

/// Scenario `ui_themes`: every other theme in the colour scope.
pub fn themes(context: &mut Context) -> Result<(), String> {
    let matrix = theme_matrix();
    let reviewed = sweep(context, &matrix)?;
    context.note(format!(
        "{reviewed} themes in the colour scope, {} screenshots",
        context.outcome.shots.len()
    ));
    gate::verdict(context, "ui_themes")
}

/// Scenario `ui_matrix_full`: the full matrix, on request only.
pub fn full(context: &mut Context) -> Result<(), String> {
    if !full_requested(context) {
        return Err(
            "SKIP: the full matrix runs with --all-themes or MIDNIGHT_ACCEPTANCE_UI_MATRIX=full \
             (full:K/N for shard K of N)"
                .to_owned(),
        );
    }
    let (matrix, part) = full_part()?;
    let reviewed = sweep(context, &matrix)?;
    context.note(format!(
        "{reviewed} combinations of the full matrix's {} ({part}: {}), {} screenshots",
        full_matrix().len(),
        matrix
            .iter()
            .map(Combination::prefix)
            .collect::<Vec<_>>()
            .join(", "),
        context.outcome.shots.len()
    ));
    gate::verdict(context, "ui_matrix_full")
}

/// The registrations of the sweep's other scenarios (`scenarios::all`).
pub fn themes_scenario() -> Scenario {
    Scenario {
        name: "ui_themes",
        area: "ui",
        summary: "Polish gate in every theme the default matrix leaves out: main window states, menus, popups, Copy, an error notice and Find.",
        run: themes,
        timeout: Duration::from_secs(40 * 60),
    }
}

pub fn full_scenario() -> Scenario {
    Scenario {
        name: "ui_matrix_full",
        area: "ui",
        summary: "Polish gate over the full matrix (--all-themes or MIDNIGHT_ACCEPTANCE_UI_MATRIX=full, full:K/N for shard K of N): every theme and density, every size class and scale factor of the base themes.",
        run: full,
        timeout: Duration::from_secs(300 * 60),
    }
}
