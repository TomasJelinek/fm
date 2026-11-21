//! Starting with folders on the command line (`--left`, `--right`, both or neither)
//! against the application's own settings store: the preferences are read
//! asynchronously from `$XDG_CONFIG_HOME/midnight-explorer/preferences.json` (Windows:
//! `%LOCALAPPDATA%\Midnight Explorer\preferences.json`), as in production, here in the
//! staged process's private profile seeded with a saved tab
//! session. Each start must finish within production's settings deadline; a panel given
//! a path shows that path in one tab, the other panel restores its saved tabs, and the
//! saved tabs of a panel without a path stay in the profile.
use crate::{
    Context,
    app::{App, Config},
    files_support::{self as support, io_error},
    viewers_stage,
};
use midnight_core::{PaneId, PaneTabPreferences, SettingsService, UserPreferences};
use midnight_platform::LocalFileSystem;
use std::{
    fs,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const TIMEOUT: Duration = Duration::from_secs(8 * 60);

/// Production shows its window with default settings once their read has taken this
/// long (`startup::DEADLINE` in midnight-explorer); a start must be ready before.
const DEADLINE: Duration = Duration::from_secs(10);

const PANES: [PaneId; 2] = [PaneId::Left, PaneId::Right];

pub fn startup_paths(context: &mut Context) -> Result<(), String> {
    viewers_stage::run_private(context, TIMEOUT, run)
}

/// A folder holding one file named after it, so its listing is recognisable.
fn marked(root: &Path, name: &str) -> Result<PathBuf, String> {
    let folder = root.join(name);
    fs::create_dir(&folder).map_err(|error| io_error(&folder, error))?;
    let marker = folder.join(format!("{name}.txt"));
    fs::write(&marker, name).map_err(|error| io_error(&marker, error))?;
    Ok(folder)
}

fn session(paths: &[&Path], active: usize) -> PaneTabPreferences {
    PaneTabPreferences {
        paths: paths.iter().map(|path| path.to_path_buf()).collect(),
        active,
        ..PaneTabPreferences::default()
    }
}

fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_owned()))
        .unwrap_or_else(|| "panicked".to_owned())
}

/// The tabs a panel shows: their folders and which one is active.
fn shown_tabs(app: &App, pane: PaneId) -> (Vec<PathBuf>, Option<usize>) {
    let tabs = app.presenter.borrow().tabs(pane);
    (
        tabs.iter().map(|tab| tab.path.clone()).collect(),
        tabs.iter().position(|tab| tab.active),
    )
}

fn run(context: &mut Context) -> Result<(), String> {
    // Only the staged process has a private profile, beside its run folder
    // (`<work>/staged/{run/<run>,xdg/config}`); never touch the account's settings.
    let staged = context.run_dir.parent().and_then(Path::parent);
    // Unix: `$XDG_CONFIG_HOME/midnight-explorer`. Windows: the LocalAppData known folder
    // (`%USERPROFILE%\AppData\Local`, also `LOCALAPPDATA`)`\Midnight Explorer`.
    #[cfg(unix)]
    let (variables, application) = (["XDG_CONFIG_HOME"], "midnight-explorer");
    #[cfg(not(unix))]
    let (variables, application) = (["LOCALAPPDATA", "USERPROFILE"], "Midnight Explorer");
    let mut private = variables.iter().map(|variable| {
        std::env::var_os(variable)
            .map(PathBuf::from)
            .filter(|folder| staged.is_some_and(|staged| folder.starts_with(staged)))
            .ok_or(format!(
                "startup_paths runs only in its staged process, with a private {variable}"
            ))
    });
    let profile = private.next().ok_or("no profile variable")??;
    for other in private {
        other?;
    }
    context.note(format!(
        "Private settings profile of the staged process: {}",
        profile.join(application).display()
    ));
    let root = support::scratch(context, "startup")?;
    let given = [marked(&root, "given-left")?, marked(&root, "given-right")?];
    let saved_left = [
        marked(&root, "saved-left-1")?,
        marked(&root, "saved-left-2")?,
    ];
    let saved_right = marked(&root, "saved-right")?;
    // The saved tab session: two tabs on the left (the second active), one on the right.
    let saved = [
        session(&[&saved_left[0], &saved_left[1]], 1),
        session(&[&saved_right], 0),
    ];
    let store = LocalFileSystem::new(None);
    let mut failures = Vec::new();
    let mut shot = false;
    for (what, explicit) in [
        ("--left and --right", [true, true]),
        ("--left only", [true, false]),
        ("--right only", [false, true]),
        ("no paths (the saved tabs restored)", [false, false]),
    ] {
        // Every start begins from the same saved session.
        store
            .save_preferences(UserPreferences {
                tabs: saved.clone(),
                ..UserPreferences::default()
            })
            .map_err(|error| format!("seeding the settings profile: {error}"))?;
        let mut config = Config::new(&given[0], &given[1]);
        config.real_settings = true;
        if explicit == [false; 2] {
            config.restore_tabs = true;
        } else {
            config.explicit_paths = Some(explicit);
        }
        let begun = Instant::now();
        let started = catch_unwind(AssertUnwindSafe(|| App::start(config)));
        let took = begun.elapsed();
        let mut app = match started {
            Ok(app) => app,
            Err(panic) => {
                failures.push(format!(
                    "start with {what}: not ready after {:.1} s: {}",
                    took.as_secs_f64(),
                    panic_text(panic.as_ref())
                ));
                continue;
            }
        };
        if took > DEADLINE {
            failures.push(format!(
                "start with {what}: ready only after {:.1} s; production shows default \
                 settings after {DEADLINE:?}",
                took.as_secs_f64()
            ));
        }
        for (slot, pane) in PANES.into_iter().enumerate() {
            let wanted = if explicit[slot] {
                (vec![given[slot].clone()], Some(0))
            } else {
                (saved[slot].paths.clone(), Some(saved[slot].active))
            };
            let shown = shown_tabs(&app, pane);
            if shown != wanted {
                failures.push(format!(
                    "start with {what}: the {pane:?} panel shows tabs {:?} (active {:?}), \
                     expected {:?} (active {:?})",
                    shown.0, shown.1, wanted.0, wanted.1
                ));
            }
            let folder = wanted.1.and_then(|active| wanted.0.get(active).cloned());
            if app.location(pane) != folder {
                failures.push(format!(
                    "start with {what}: the {pane:?} panel lists {:?}, expected {folder:?}",
                    app.location(pane)
                ));
            }
            let marker = folder
                .as_ref()
                .and_then(|folder| folder.file_name())
                .map(|name| format!("{}.txt", name.to_string_lossy()));
            if let Some(marker) = marker
                && !app.names(pane).contains(&marker)
            {
                failures.push(format!(
                    "start with {what}: the {pane:?} panel's listing has no {marker}: {:?}",
                    app.names(pane)
                ));
            }
        }
        let status = app.window.get_desktop_status().to_string();
        context.note(format!(
            "start with {what}: ready after {:.2} s; status line {status:?}",
            took.as_secs_f64()
        ));
        if !shot {
            shot = true;
            app.screenshot(
                context,
                "start-explicit-paths",
                "Started with --left and --right",
            );
        }
        // Let the save that follows the first read finish before the store is read.
        app.idle(Duration::from_millis(800));
        app.shutdown()?;
        match store.load_preferences_for_acceptance() {
            Err(error) => failures.push(format!(
                "start with {what}: the profile cannot be read afterwards: {error}"
            )),
            Ok(stored) => {
                for (slot, pane) in PANES.into_iter().enumerate() {
                    let kept = &stored.tabs[slot];
                    if !explicit[slot]
                        && (kept.paths != saved[slot].paths || kept.active != saved[slot].active)
                    {
                        failures.push(format!(
                            "start with {what}: the {pane:?} panel's saved tabs became {:?} \
                             (active {}) instead of staying {:?} (active {})",
                            kept.paths, kept.active, saved[slot].paths, saved[slot].active
                        ));
                    }
                }
                context.note(format!(
                    "start with {what}: saved tabs afterwards: left {:?} (active {}), right {:?} (active {})",
                    stored.tabs[0].paths, stored.tabs[0].active, stored.tabs[1].paths, stored.tabs[1].active
                ));
            }
        }
    }
    support::report(&failures)
}
