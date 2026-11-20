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
use midnight_core::{PaneId, PreferenceTheme, UiDensity};
use midnight_ui::{FileListPresentation, HotPathsPresentation};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, software_renderer::MinimalSoftwareWindow},
};
use std::{path::Path, rc::Rc, time::Duration};

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

/// Whether the full matrix was asked for (`--all-themes`, or
/// `MIDNIGHT_ACCEPTANCE_UI_MATRIX=full`).
pub fn full_requested(context: &Context) -> bool {
    context.options.all_themes
        || std::env::var("MIDNIGHT_ACCEPTANCE_UI_MATRIX").is_ok_and(|value| value == "full")
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
    app.window.invoke_show_options();
    wait(app, "Options", |app| app.window.get_options_open());
    for (topic, label) in [
        (0, "General"),
        (1, "Appearance"),
        (3, "Keyboard"),
        (4, "Editors"),
        (10, "Comparators"),
        (5, "Plugins"),
        (6, "Previews"),
        (7, "Archives"),
        (8, "Colors"),
        (9, "Custom Commands"),
    ] {
        if app.window.get_options_topic_index() != topic {
            if inspect::find(app.slint(), AccessibleRole::Button, label).is_some() {
                input::click_button(app.slint(), label)?;
            } else {
                // Narrow layouts replace the topic list with a combo box: open it and
                // move through the topics in their displayed order.
                const ORDER: [i32; 10] = [0, 1, 3, 4, 10, 5, 6, 7, 8, 9];
                let position = |topic: i32| ORDER.iter().position(|&id| id == topic).unwrap_or(0);
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
    }
    close(app, &Host::Main, "Options");
    Ok(())
}

type Step<'a> = Box<dyn FnOnce(&mut App, &mut Context) -> Result<(), String> + 'a>;

/// Run one step; a failure (or panic) becomes a review flag and the sweep goes on after
/// Escape has closed whatever the step left open.
fn isolated(app: &mut App, context: &mut Context, prefix: &str, name: &str, step: Step<'_>) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| step(app, context)));
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
}

/// An unreadable folder with a long name inside the review folder: entering it shows
/// the error notice that names a long path.
fn locked_folder(left: &Path) -> Result<String, String> {
    let name = "a-folder-whose-long-name-makes-the-error-path-long-enough-to-wrap-in-a-notice";
    let path = left.join(name);
    std::fs::create_dir_all(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000))
            .map_err(|error| format!("{}: {error}", path.display()))?;
    }
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
    if inspect::find(app.slint(), AccessibleRole::Button, "Details").is_some() {
        input::click_button(app.slint(), "Details")?;
        app.settle();
        sweep.capture(
            app,
            context,
            &Host::Main,
            "notice-details",
            "Error notice, details shown",
        );
    }
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
            close(app, &Host::Main, "Attributes");
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
        let (width, height) = combination.main_size();
        let mut config = Config::new(&left, &right);
        config.theme = combination.theme;
        config.density = combination.density;
        config.width = width;
        config.height = height;
        let mut app = App::start(config);
        gate::scale_main(&mut app, (width, height), combination.scale);
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
        app.shutdown()?;
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
            "SKIP: the full matrix runs with --all-themes or MIDNIGHT_ACCEPTANCE_UI_MATRIX=full"
                .to_owned(),
        );
    }
    let matrix = full_matrix();
    let reviewed = sweep(context, &matrix)?;
    context.note(format!(
        "{reviewed} combinations of the full matrix, {} screenshots",
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
        summary: "Polish gate over the full matrix (--all-themes or MIDNIGHT_ACCEPTANCE_UI_MATRIX=full): every theme and density, every size class and scale factor of the base themes.",
        run: full,
        timeout: Duration::from_secs(300 * 60),
    }
}
