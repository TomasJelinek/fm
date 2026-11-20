//! UI review: render the main window and every reachable dialog in several themes,
//! both densities and small and large window sizes, and flag layout problems.
//!
//! Dialogs open with their keyboard shortcut where one exists. Options, Hot Paths,
//! Jobs and About have no default shortcut; they open through the same window
//! callbacks the main menu invokes. Viewer and file-comparison windows need installed
//! plugins and are not reviewed yet (docs/developer/ACCEPTANCE.md, "Extending").
use super::{
    Context,
    app::{self, App, Config},
    input, inspect, panel, report,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{PaneId, PreferenceTheme, UiDensity};
use midnight_ui::{FileListPresentation, HotPathsPresentation};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, software_renderer::MinimalSoftwareWindow},
};
use std::{rc::Rc, time::Duration};

/// Where an opened dialog lives.
enum Host {
    /// A popup on the main window's adapter.
    Main,
    /// Its own platform window.
    Tool(Rc<MinimalSoftwareWindow>),
}

struct Combination {
    theme: PreferenceTheme,
    density: UiDensity,
    width: usize,
    height: usize,
    dialogs: bool,
}

fn combinations(all_themes: bool) -> Vec<Combination> {
    let full = |theme, density, width, height| Combination {
        theme,
        density,
        width,
        height,
        dialogs: true,
    };
    let mut list = vec![
        full(PreferenceTheme::Blue, UiDensity::Desktop, 1600, 1000),
        full(PreferenceTheme::Blue, UiDensity::Desktop, 960, 600),
        full(PreferenceTheme::Dark, UiDensity::Touch, 1600, 1000),
        full(PreferenceTheme::Dark, UiDensity::Touch, 960, 600),
        full(PreferenceTheme::Light, UiDensity::Desktop, 1340, 820),
        full(
            PreferenceTheme::HighContrastDark,
            UiDensity::Touch,
            1340,
            820,
        ),
    ];
    if all_themes {
        for index in 0..=25 {
            let theme = theme_from_index(index);
            if list.iter().any(|combination| combination.theme == theme) {
                continue;
            }
            list.push(Combination {
                theme,
                density: UiDensity::Desktop,
                width: 1340,
                height: 820,
                dialogs: false,
            });
        }
    }
    list
}

fn theme_from_index(index: i32) -> PreferenceTheme {
    use PreferenceTheme::*;
    [
        Blue,
        Dark,
        Light,
        Dracula,
        Nord,
        GruvboxDark,
        CatppuccinMocha,
        SolarizedLight,
        CarbonGreen,
        TokyoNight,
        OneDark,
        RosePine,
        NortonCommander,
        AmberTerminal,
        VerdantLight,
        CatppuccinLatte,
        RosePineDawn,
        PaperContrast,
        SandstoneContrast,
        DeepOcean,
        GraphiteContrast,
        HighContrastDark,
        HighContrastLight,
        SlateBlue,
        SlateTeal,
        SlateViolet,
    ][index as usize]
}

/// Record a capture's layout findings against its screenshot.
fn check(context: &mut Context, shot: &str, window: &slint::Window, width: f32, height: f32) {
    let findings = inspect::check_layout(window, width, height);
    if let Some(recorded) = context
        .outcome
        .shots
        .iter_mut()
        .find(|recorded| recorded.file == shot)
    {
        recorded.flags = findings.len();
    }
    for finding in findings {
        context.outcome.flags.push(report::Flag {
            shot: shot.to_owned(),
            kind: finding.kind,
            detail: finding.detail,
        });
    }
}

fn capture(app: &mut App, context: &mut Context, host: &Host, name: &str, caption: &str) {
    // Theme colours animate briefly after changes; let them settle before rendering.
    app.idle(Duration::from_millis(250));
    match host {
        Host::Main => {
            let shot = app.screenshot(context, name, caption);
            let (width, height) = (app.width as f32, app.height as f32);
            check(context, &shot, app.window.window(), width, height);
        }
        Host::Tool(window) => {
            let shot = app.screenshot_window(context, window, name, caption);
            let size = window.window().size();
            check(
                context,
                &shot,
                window.window(),
                size.width as f32,
                size.height as f32,
            );
        }
    }
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
}

/// Wait for a new tool window and size it for this combination.
fn tool(app: &mut App, what: &str, before: usize) -> Host {
    app.until(what, move |app| app.tool_windows().len() > before);
    let window = app.tool_windows().pop().expect("tool window");
    let width = (app.width as f32 * 0.7).clamp(480.0, 1000.0) as u32;
    let height = (app.height as f32 * 0.75).clamp(360.0, 720.0) as u32;
    app::layout_tool(&window, width, height);
    app.settle();
    Host::Tool(window)
}

fn close(app: &mut App, host: &Host, what: &str) {
    match host {
        Host::Main => input::key(app.slint(), Key::Escape),
        Host::Tool(window) => input::key(window.window(), Key::Escape),
    }
    app.until(&format!("{what} to close"), main_idle);
}

fn options_pages(app: &mut App, context: &mut Context, prefix: &str) -> Result<(), String> {
    app.window.invoke_show_options();
    app.until("Options", |app| app.window.get_options_open());
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
            app.until(&format!("the {label} Options page"), move |app| {
                app.window.get_options_topic_index() == topic
            });
        }
        capture(
            app,
            context,
            &Host::Main,
            &format!("{prefix}-options-{topic:02}"),
            &format!("Options › {label}"),
        );
    }
    close(app, &Host::Main, "Options");
    Ok(())
}

type Step<'a> = Box<dyn FnOnce(&mut App, &mut Context) -> Result<(), String> + 'a>;

/// A dialog opened by a shortcut: name, caption, modifiers, key, and "it is open".
type Shortcut = (
    &'static str,
    &'static str,
    &'static [Key],
    Key,
    fn(&App) -> bool,
);

/// Run one dialog step; a failure (or panic) becomes a review flag and the sweep goes on
/// after Escape has closed whatever the step left open.
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
        shot: format!("shots/{}/{prefix}-main.png", app::context_name()),
        kind: "dialog step failed",
        detail: format!("{name}: {first}"),
    });
    context.note(format!("{prefix} {name} failed: {error}"));
    for _ in 0..3 {
        for window in app.tool_windows() {
            input::key(window.window(), Key::Escape);
        }
        input::key(app.slint(), Key::Escape);
        app.idle(Duration::from_millis(100));
    }
}

/// Open, capture and close every dialog for one combination.
fn dialogs(app: &mut App, context: &mut Context, prefix: &str) -> Result<(), String> {
    let left = PaneId::Left;
    panel::select_row(app, left, "alpha.bin")?;
    let p = prefix.to_owned();
    isolated(
        app,
        context,
        prefix,
        "main menu",
        Box::new(move |app: &mut App, context: &mut Context| {
            input::key(app.slint(), Key::F10);
            app.until("the main menu", has_popup);
            capture(
                app,
                context,
                &Host::Main,
                &format!("{p}-main-menu"),
                "Main menu (F10)",
            );
            close(app, &Host::Main, "the main menu");
            Ok(())
        }),
    );
    let p = prefix.to_owned();
    isolated(
        app,
        context,
        prefix,
        "options",
        Box::new(move |app: &mut App, context: &mut Context| options_pages(app, context, &p)),
    );
    dialog_steps(app, context, prefix);
    Ok(())
}

fn dialog_steps(app: &mut App, context: &mut Context, prefix: &str) {
    let left = PaneId::Left;

    let simple: [Shortcut; 9] = [
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
    ];
    for (name, caption, modifiers, key, ready) in simple {
        let shot = format!("{prefix}-{name}");
        isolated(
            app,
            context,
            prefix,
            caption,
            Box::new(move |app: &mut App, context: &mut Context| {
                panel::focus_list(app, left)?;
                input::chord(app.slint(), modifiers, key);
                app.until(caption, ready);
                capture(app, context, &Host::Main, &shot, caption);
                close(app, &Host::Main, caption);
                Ok(())
            }),
        );
    }

    let p = prefix.to_owned();
    isolated(
        app,
        context,
        prefix,
        "Make File List",
        Box::new(move |app: &mut App, context: &mut Context| {
            panel::focus_list(app, left)?;
            input::chord(app.slint(), &[Key::Control], "m");
            app.until("Make File List", |app| {
                app.window.global::<FileListPresentation>().get_active()
            });
            capture(
                app,
                context,
                &Host::Main,
                &format!("{p}-file-list"),
                "Make File List (Ctrl+M)",
            );
            close(app, &Host::Main, "Make File List");
            Ok(())
        }),
    );

    let p = prefix.to_owned();
    isolated(
        app,
        context,
        prefix,
        "Attributes",
        Box::new(move |app: &mut App, context: &mut Context| {
            panel::focus_list(app, left)?;
            input::chord(app.slint(), &[Key::Control], Key::F2);
            app.until("Attributes", |app| {
                !app.window.get_metadata_token().is_empty()
            });
            capture(
                app,
                context,
                &Host::Main,
                &format!("{p}-attributes"),
                "Attributes (Ctrl+F2)",
            );
            input::click_button(app.slint(), "Times")?;
            app.settle();
            capture(
                app,
                context,
                &Host::Main,
                &format!("{p}-times"),
                "Attributes › Times",
            );
            close(app, &Host::Main, "Attributes");
            Ok(())
        }),
    );

    let p = prefix.to_owned();
    isolated(
        app,
        context,
        prefix,
        "Hot Paths",
        Box::new(move |app: &mut App, context: &mut Context| {
            panel::focus_list(app, left)?;
            app.window.invoke_show_bookmarks(left.index() as i32);
            app.until("Hot Paths", |app| {
                !app.window
                    .global::<HotPathsPresentation>()
                    .get_token()
                    .is_empty()
            });
            capture(
                app,
                context,
                &Host::Main,
                &format!("{p}-hot-paths"),
                "Hot Paths",
            );
            close(app, &Host::Main, "Hot Paths");
            Ok(())
        }),
    );

    let p = prefix.to_owned();
    isolated(
        app,
        context,
        prefix,
        "About",
        Box::new(move |app: &mut App, context: &mut Context| {
            app.window.invoke_show_about();
            app.until("About", |app| app.window.get_about_open());
            capture(app, context, &Host::Main, &format!("{p}-about"), "About");
            close(app, &Host::Main, "About");
            Ok(())
        }),
    );

    let p = prefix.to_owned();
    isolated(
        app,
        context,
        prefix,
        "Drive information",
        Box::new(move |app: &mut App, context: &mut Context| {
            panel::focus_list(app, left)?;
            input::chord(app.slint(), &[Key::Control], Key::F1);
            app.until("Drive information", |app| {
                app.window.get_embedded_modal_open() || has_popup(app)
            });
            app.idle(Duration::from_millis(300));
            capture(
                app,
                context,
                &Host::Main,
                &format!("{p}-drive-info"),
                "Drive information (Ctrl+F1)",
            );
            close(app, &Host::Main, "Drive information");
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
        let shot = format!("{prefix}-{name}");
        isolated(
            app,
            context,
            prefix,
            caption,
            Box::new(move |app: &mut App, context: &mut Context| {
                panel::focus_list(app, left)?;
                let before = app.tool_windows().len();
                input::chord(app.slint(), modifiers, key);
                let host = tool(app, caption, before);
                capture(app, context, &host, &shot, caption);
                close(app, &host, caption);
                Ok(())
            }),
        );
    }

    let p = prefix.to_owned();
    isolated(
        app,
        context,
        prefix,
        "Jobs",
        Box::new(move |app: &mut App, context: &mut Context| {
            let before = app.tool_windows().len();
            app.window.invoke_show_operations();
            let host = tool(app, "Jobs", before);
            capture(app, context, &host, &format!("{p}-jobs"), "Jobs");
            close(app, &host, "Jobs");
            Ok(())
        }),
    );
}

pub fn run(context: &mut Context) -> Result<(), String> {
    // Dialogs are cancelled, never confirmed, but work on a copy all the same.
    let left = context.work.join("review");
    super::scenarios::transfers::copy_tree(&context.fixture.path("checksums"), &left)?;
    let right = context.fixture.root.clone();
    let mut reviewed = 0;
    for combination in combinations(context.options.all_themes) {
        let Combination {
            theme,
            density,
            width,
            height,
            dialogs: with_dialogs,
        } = combination;
        let mut config = Config::new(&left, &right);
        config.theme = theme;
        config.density = density;
        config.width = width;
        config.height = height;
        let mut app = App::start(config);
        let prefix = format!("{theme:?}-{density:?}-{width}x{height}");
        capture(
            &mut app,
            context,
            &Host::Main,
            &format!("{prefix}-main"),
            &format!("Main window · {theme:?} · {density:?} · {width}×{height}"),
        );
        if with_dialogs {
            dialogs(&mut app, context, &prefix).map_err(|error| format!("{prefix}: {error}"))?;
        }
        app.shutdown()?;
        reviewed += 1;
    }
    context.note(format!(
        "{reviewed} theme/density/size combinations, {} screenshots; not reviewed: viewers and Compare files (need installed plugins)",
        context.outcome.shots.len()
    ));
    Ok(())
}
