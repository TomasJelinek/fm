//! The polish gate's interactive sweep (scenario `ui_polish_dialogs`): every embedded
//! dialog and every tool window is opened again and again from plain browsing, and
//! `ui_interact::check_surface` checks Tab and Shift+Tab, visible focus, Escape, Enter,
//! the effect of every button (each on a fresh opening and a freshly restored scratch
//! folder), unexpected windows and notices after a dismissal, hover feedback and the
//! tooltips of icon-only buttons; the main window gets the hover and tooltip checks.
//! Each surface is also captured once with the static and contrast checks.
//!
//! It runs in a private child process (`viewers_stage::run_private`): the buttons are
//! really pressed, so Delete to Trash moves a scratch file into a private Trash. Windows
//! has no private Recycle Bin, so there the Trash confirmation is never confirmed.
use crate::{
    Context, Scenario,
    app::{self, App, Config},
    files_support, input, inspect, panel,
    scenarios::{
        transfers::copy_tree,
        ui_gate::{self as gate, Target},
        ui_interact::{self as interact, Surface},
    },
    tools_support, viewers_stage,
};
use midnight_core::PaneId;
use midnight_ui::{FileListPresentation, HotPathsPresentation};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter},
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

const TIMEOUT: Duration = Duration::from_secs(60 * 60);
const LEFT: PaneId = PaneId::Left;
const RIGHT: PaneId = PaneId::Right;
/// A folder the panel cannot list: entering it opens the error notice with its path.
const UNREADABLE: &str = "an-unreadable-folder-whose-long-name-shows-how-the-notice-handles-paths";

/// The scratch folders the dialogs act on, restored before every opening.
struct Scratch {
    template: PathBuf,
    left: PathBuf,
    right: PathBuf,
}

/// Set `path`'s permission bits (on Windows their equivalent, `tools_support::set_mode`).
fn set_mode(path: &Path, mode: u32) {
    let _ = crate::tools_support::set_mode(path, mode);
}

/// Empty `folder`, giving its entries owner permissions back first.
fn empty(folder: &Path) -> Result<(), String> {
    for entry in fs::read_dir(folder)
        .map_err(|error| error.to_string())?
        .flatten()
    {
        let path = entry.path();
        let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if is_dir {
            set_mode(&path, 0o755);
            crate::fixtures::restore_owner_write(&path);
            fs::remove_dir_all(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        } else {
            fs::remove_file(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        }
    }
    Ok(())
}

impl Scratch {
    fn new(context: &Context) -> Result<Self, String> {
        let root = context.work.join("dialogs");
        let scratch = Self {
            template: root.join("template"),
            left: root.join("left"),
            right: root.join("right"),
        };
        copy_tree(&context.fixture.path("checksums"), &scratch.template)?;
        fs::create_dir_all(scratch.template.join("folder")).map_err(|error| error.to_string())?;
        fs::write(scratch.template.join("folder/inside.txt"), "inside\n")
            .map_err(|error| error.to_string())?;
        for folder in [&scratch.left, &scratch.right] {
            fs::create_dir_all(folder).map_err(|error| error.to_string())?;
        }
        scratch.reset_disk()?;
        Ok(scratch)
    }

    fn reset_disk(&self) -> Result<(), String> {
        empty(&self.left)?;
        empty(&self.right)?;
        for entry in fs::read_dir(&self.template)
            .map_err(|error| error.to_string())?
            .flatten()
        {
            copy_tree(&entry.path(), &self.left.join(entry.file_name()))?;
        }
        let locked = self.left.join(UNREADABLE);
        fs::create_dir_all(&locked).map_err(|error| error.to_string())?;
        set_mode(&locked, 0o000);
        Ok(())
    }
}

fn listed(app: &App, pane: PaneId, folder: &Path) -> bool {
    let mut shown: Vec<String> = app
        .names(pane)
        .into_iter()
        .filter(|name| name != "..")
        .collect();
    shown.sort();
    app.location(pane).as_deref() == Some(folder)
        && !app.loading(pane)
        && shown == panel::disk_names(folder)
}

/// Type a folder into the panel's path editor (Ctrl+G) and wait until it is shown.
fn go_to(app: &mut App, pane: PaneId, folder: &Path) -> Result<(), String> {
    panel::focus_list(app, pane)?;
    input::chord(app.slint(), &[Key::Control], "g");
    app.settle();
    input::replace_text(app.slint(), &folder.to_string_lossy());
    input::key(app.slint(), Key::Return);
    let wanted = folder.to_path_buf();
    app.until_for(
        &format!("the {pane:?} panel to show {}", folder.display()),
        Duration::from_secs(20),
        move |app| app.location(pane).as_ref() == Some(&wanted) && !app.loading(pane),
    );
    Ok(())
}

/// Back to plain browsing on freshly restored scratch folders.
fn restore(app: &mut App, scratch: &Scratch) {
    files_support::dismiss_all(app);
    app.idle(Duration::from_millis(150));
    files_support::dismiss_all(app);
    if let Err(error) = scratch.reset_disk() {
        eprintln!("ui_polish_dialogs: restoring the scratch folders: {error}");
    }
    for (pane, folder) in [(LEFT, &scratch.left), (RIGHT, &scratch.right)] {
        if app.location(pane).as_deref() != Some(folder.as_path()) {
            let _ = go_to(app, pane, folder);
        }
        if !listed(app, pane, folder) && panel::focus_list(app, pane).is_ok() {
            input::chord(app.slint(), &[Key::Control], "r");
        }
    }
    let (left, right) = (scratch.left.clone(), scratch.right.clone());
    app.until_for(
        "the panels to list the restored scratch folders",
        Duration::from_secs(20),
        move |app| listed(app, LEFT, &left) && listed(app, RIGHT, &right),
    );
    let _ = panel::focus_list(app, LEFT);
}

fn embedded_open(app: &App) -> bool {
    inspect::roots(app.slint()).len() > 1 || app.window.get_embedded_modal_open()
}

/// The newest tool window, sized to its preferred size.
fn new_tool(app: &mut App, what: &str, before: usize) -> Result<Target, String> {
    app.until(what, move |app| app.tool_windows().len() > before);
    let window = app
        .tool_windows()
        .pop()
        .ok_or_else(|| format!("{what}: no window"))?;
    let (_, preferred) = gate::window_sizes(window.window());
    gate::size_tool(&window, preferred, 1.0);
    app.settle();
    Ok(Target::Tool(window))
}

type Open = fn(&mut App) -> Result<Target, String>;
type IsOpen = fn(&App, &Target) -> bool;

fn key_dialog(
    app: &mut App,
    row: &str,
    modifiers: &[Key],
    key: Key,
    what: &str,
    ready: fn(&App) -> bool,
) -> Result<Target, String> {
    panel::select_row(app, LEFT, row)?;
    input::chord(app.slint(), modifiers, key);
    app.until(what, ready);
    Ok(Target::Main)
}

fn tool_open(_: &App, target: &Target) -> bool {
    match target {
        Target::Tool(window) => window.window().is_visible(),
        Target::Main => false,
    }
}

/// Every surface of the sweep: name, opener, open test, buttons that are not clicked
/// (with the reason), and whether Enter and the buttons are checked.
struct Spec {
    name: &'static str,
    open: Open,
    is_open: IsOpen,
    skip: &'static [(&'static str, &'static str)],
    enter: bool,
    buttons: bool,
    /// Typed into the focused name field before Enter is pressed.
    enter_text: Option<&'static str>,
}

const SPECS: &[Spec] = &[
    Spec {
        name: "copy",
        open: |app| {
            key_dialog(app, "alpha.bin", &[], Key::F5, "Copy", |app| {
                !app.window.get_copy_draft_token().is_empty()
            })
        },
        is_open: |app, _| !app.window.get_copy_draft_token().is_empty(),
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "move",
        open: |app| {
            key_dialog(app, "alpha.bin", &[], Key::F6, "Move", |app| {
                !app.window.get_copy_draft_token().is_empty()
            })
        },
        is_open: |app, _| !app.window.get_copy_draft_token().is_empty(),
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "trash",
        open: |app| {
            key_dialog(app, "alpha.bin", &[], Key::F8, "Delete to Trash", |app| {
                !app.window.get_operation_token().is_empty()
            })
        },
        is_open: |app, _| !app.window.get_operation_token().is_empty(),
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "delete",
        open: |app| {
            key_dialog(app, "alpha.bin", &[Key::Shift], Key::F8, "Delete", |app| {
                !app.window.get_operation_token().is_empty()
            })
        },
        is_open: |app, _| !app.window.get_operation_token().is_empty(),
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "rename",
        open: |app| {
            key_dialog(app, "alpha.bin", &[], Key::F2, "Rename", |app| {
                !app.window.get_operation_token().is_empty()
            })
        },
        is_open: |app, _| !app.window.get_operation_token().is_empty(),
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "mkdir",
        open: |app| {
            key_dialog(app, "alpha.bin", &[], Key::F7, "Create directory", |app| {
                !app.window.get_operation_token().is_empty()
            })
        },
        is_open: |app, _| !app.window.get_operation_token().is_empty(),
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: Some("polish folder"),
    },
    Spec {
        name: "newfile",
        open: |app| {
            key_dialog(
                app,
                "alpha.bin",
                &[Key::Shift],
                Key::F4,
                "New file",
                |app| !app.window.get_operation_token().is_empty(),
            )
        },
        is_open: |app, _| !app.window.get_operation_token().is_empty(),
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: Some("polish file.txt"),
    },
    Spec {
        name: "compare-dirs",
        open: |app| {
            key_dialog(
                app,
                "alpha.bin",
                &[Key::Control],
                Key::F10,
                "Compare",
                |app| app.window.get_comparison_view().open,
            )
        },
        is_open: |app, _| app.window.get_comparison_view().open,
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "panel-filter",
        open: |app| {
            key_dialog(
                app,
                "alpha.bin",
                &[Key::Control],
                Key::F12,
                "Filter",
                |app| app.window.get_panel_filter_view().open,
            )
        },
        is_open: |app, _| app.window.get_panel_filter_view().open,
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "file-list",
        open: |app| {
            panel::select_row(app, LEFT, "alpha.bin")?;
            input::chord(app.slint(), &[Key::Control], "m");
            app.until("Make File List", |app| {
                app.window.global::<FileListPresentation>().get_active()
            });
            Ok(Target::Main)
        },
        is_open: |app, _| app.window.global::<FileListPresentation>().get_active(),
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "attributes",
        open: |app| {
            key_dialog(
                app,
                "alpha.bin",
                &[Key::Control],
                Key::F2,
                "Attributes",
                |app| !app.window.get_metadata_token().is_empty(),
            )
        },
        is_open: |app, _| !app.window.get_metadata_token().is_empty(),
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "hot-paths",
        open: |app| {
            panel::focus_list(app, LEFT)?;
            app.window.invoke_show_bookmarks(LEFT.index() as i32);
            app.until("Hot Paths", |app| {
                !app.window
                    .global::<HotPathsPresentation>()
                    .get_token()
                    .is_empty()
            });
            Ok(Target::Main)
        },
        is_open: |app, _| {
            !app.window
                .global::<HotPathsPresentation>()
                .get_token()
                .is_empty()
        },
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "about",
        open: |app| {
            app.window.invoke_show_about();
            app.until("About", |app| app.window.get_about_open());
            Ok(Target::Main)
        },
        is_open: |app, _| app.window.get_about_open(),
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "drive-info",
        open: |app| {
            panel::focus_list(app, LEFT)?;
            input::chord(app.slint(), &[Key::Control], Key::F1);
            app.until("Drive information", embedded_open);
            app.idle(Duration::from_millis(300));
            Ok(Target::Main)
        },
        is_open: |app, _| embedded_open(app),
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "properties",
        open: |app| {
            panel::select_row(app, LEFT, "alpha.bin")?;
            input::chord(app.slint(), &[Key::Alt], Key::Return);
            app.until("Properties", embedded_open);
            app.idle(Duration::from_millis(300));
            Ok(Target::Main)
        },
        is_open: |app, _| embedded_open(app),
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "error-notice",
        open: |app| {
            panel::select_row(app, LEFT, UNREADABLE)?;
            input::key(app.slint(), Key::Return);
            app.until("the error notice", |app| {
                tools_support::shown_notice(app).is_some()
            });
            Ok(Target::Main)
        },
        is_open: |app, _| tools_support::shown_notice(app).is_some(),
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "options",
        open: |app| {
            app.window.invoke_show_options();
            app.until("Options", |app| app.window.get_options_open());
            Ok(Target::Main)
        },
        is_open: |app, _| app.window.get_options_open(),
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "find",
        open: |app| {
            panel::focus_list(app, LEFT)?;
            let before = app.tool_windows().len();
            input::chord(app.slint(), &[Key::Alt], Key::F7);
            new_tool(app, "Find", before)
        },
        is_open: tool_open,
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "checksums",
        open: |app| {
            panel::select_row(app, LEFT, "alpha.bin")?;
            let before = app.tool_windows().len();
            input::chord(app.slint(), &[Key::Control], Key::F8);
            new_tool(app, "Checksums", before)
        },
        is_open: tool_open,
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "help",
        open: |app| {
            panel::focus_list(app, LEFT)?;
            let before = app.tool_windows().len();
            input::key(app.slint(), Key::F1);
            new_tool(app, "Help", before)
        },
        is_open: tool_open,
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
    Spec {
        name: "jobs",
        open: |app| {
            let before = app.tool_windows().len();
            app.window.invoke_show_operations();
            new_tool(app, "Jobs", before)
        },
        is_open: tool_open,
        skip: &[],
        enter: true,
        buttons: true,
        enter_text: None,
    },
];

/// The names of the sweep's surfaces, for the coverage record.
pub fn surface_names() -> Vec<&'static str> {
    SPECS.iter().map(|spec| spec.name).collect()
}

fn body(context: &mut Context) -> Result<(), String> {
    let scratch = Scratch::new(context)?;
    let mut config = Config::new(&scratch.left, &scratch.right);
    config.clipboard = true;
    let mut app = App::start(config);
    restore(&mut app, &scratch);
    // The main window: hover feedback and the tooltips of icon-only buttons.
    let shot_main = {
        gate::capture(
            &mut app,
            context,
            &Target::Main,
            "main",
            "dialogs-main",
            "Main window (interactive checks)",
        );
        format!("shots/{}/dialogs-main.png", app::context_name())
    };
    let mut issues = interact::hover_checks(&mut app, &Target::Main);
    issues.extend(interact::tooltip_checks(&mut app, &Target::Main));
    gate::record(context, &shot_main, "main", issues);
    for spec in SPECS {
        restore(&mut app, &scratch);
        // A screenshot with the static and contrast checks first.
        let shot = format!("dialogs-{}", spec.name);
        let opened = (spec.open)(&mut app);
        match opened {
            Ok(target) => {
                gate::capture(&mut app, context, &target, spec.name, &shot, spec.name);
            }
            Err(error) => {
                gate::record(
                    context,
                    &format!("shots/{}/{shot}.png", app::context_name()),
                    spec.name,
                    vec![gate::Issue::new("surface did not open", error)],
                );
                continue;
            }
        }
        restore(&mut app, &scratch);
        let scratch_ref = &scratch;
        let open = spec.open;
        let is_open = spec.is_open;
        let mut surface = Surface::new(
            spec.name,
            move |app: &mut App| open(app),
            move |app: &App, target: &Target| is_open(app, target),
            move |app: &mut App| restore(app, scratch_ref),
        );
        surface.skip = spec
            .skip
            .iter()
            .map(|(label, reason)| ((*label).to_owned(), *reason))
            .collect();
        surface.folders = vec![scratch.left.clone(), scratch.right.clone()];
        surface.enter = spec.enter;
        surface.enter_text = spec.enter_text;
        surface.buttons = spec.buttons;
        // Windows recycles into the account's own Recycle Bin, which no variable makes
        // private: the Trash confirmation is never confirmed there (Enter, its button).
        if cfg!(windows) && spec.name == "trash" {
            surface.enter = false;
            surface.skip.push((
                "Move to Trash".to_owned(),
                "on Windows it would recycle into the account's own Recycle Bin",
            ));
            context.note(
                "trash: Enter and Move to Trash not pressed on Windows (the account's own \
                 Recycle Bin cannot be made private)",
            );
        }
        let outcome = interact::check_surface(&mut app, &mut surface);
        interact::record(
            context,
            spec.name,
            &format!("shots/{}/{shot}.png", app::context_name()),
            outcome,
        );
    }
    restore(&mut app, &scratch);
    app.shutdown()?;
    // The unreadable folder must not keep the work folder from being deleted.
    set_mode(&scratch.left.join(UNREADABLE), 0o755);
    context.note(format!(
        "{} surfaces checked interactively: {}",
        SPECS.len(),
        surface_names().join(", ")
    ));
    gate::verdict(context, "ui_polish_dialogs")
}

pub fn dialogs(context: &mut Context) -> Result<(), String> {
    viewers_stage::run_private(context, TIMEOUT, body)
}

/// The registration of `ui_polish_dialogs` (`scenarios::all`).
pub fn scenario() -> Scenario {
    Scenario {
        name: "ui_polish_dialogs",
        area: "ui",
        summary: "Polish gate, interactive: every dialog and tool window — Tab/Shift+Tab, visible focus, Escape, Enter, every button's effect, no extra window or notice, hover feedback and tooltips.",
        run: dialogs,
        timeout: TIMEOUT,
    }
}
