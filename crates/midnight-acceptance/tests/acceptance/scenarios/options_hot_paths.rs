//! Hot Paths management: add three folders from the manager, reorder their numbered
//! shortcuts (an occupied number is exchanged), remove one, open the others with
//! Ctrl+digit, and keep them across a restart.
use crate::{
    Context,
    app::{App, Config},
    input, inspect,
    options_support::{self as options, SettingsStore},
    panel,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use midnight_ui::HotPathsPresentation;
use slint::{ComponentHandle, platform::Key};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

fn config(folder: &Path, store: &Arc<SettingsStore>) -> Config {
    let mut config = Config::new(folder, folder);
    config.settings = Some(store.clone());
    config
}

fn manager_open(app: &App) -> bool {
    !app.window
        .global::<HotPathsPresentation>()
        .get_token()
        .is_empty()
}

/// Open the manager from the toolbar's Hot Paths picker.
fn open_manager(app: &mut App) -> Result<(), String> {
    panel::focus_list(app, PaneId::Left)?;
    options::pick(app, "Hot Paths", "Manage Hot Paths…")?;
    app.until("the Hot Paths manager", manager_open);
    app.idle(Duration::from_millis(60));
    Ok(())
}

fn close_manager(app: &mut App) -> Result<(), String> {
    options::press(app, "Close")?;
    app.until("the Hot Paths manager to close", |app| !manager_open(app));
    panel::wait_input(app);
    Ok(())
}

/// The manager's rows: (path, shortcut line) from each row button's label.
fn rows(app: &App) -> Vec<(String, String)> {
    let mut rows: Vec<(f32, String, String)> =
        inspect::controls(app.slint(), AccessibleRole::Button)
            .into_iter()
            .filter(|(label, _)| label.starts_with('/') || label.contains(":\\"))
            .map(|(label, item)| {
                let mut lines = label.splitn(2, '\n');
                let path = lines.next().unwrap_or_default().to_owned();
                let slot = lines.next().unwrap_or_default().to_owned();
                (inspect::rect(&item).1, path, slot)
            })
            .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter()
        .map(|(_, path, slot)| (path, slot))
        .collect()
}

fn slot_of(app: &App, path: &Path) -> Option<String> {
    let wanted = path.to_string_lossy();
    rows(app)
        .into_iter()
        .find(|(row, _)| *row == wanted)
        .map(|(_, slot)| slot)
}

/// Saved hot paths: (path, slot), in saved order.
fn saved(store: &SettingsStore) -> Vec<(PathBuf, Option<u8>)> {
    store
        .current()
        .bookmarks
        .iter()
        .map(|bookmark| (bookmark.path.clone(), bookmark.slot))
        .collect()
}

fn go(app: &mut App, path: &Path) -> Result<(), String> {
    let parent = path.parent().ok_or("no parent")?.to_path_buf();
    if app.location(PaneId::Left).as_deref() != Some(parent.as_path()) {
        panel::focus_list(app, PaneId::Left)?;
        input::key(app.slint(), Key::Backspace);
        panel::wait_location(app, PaneId::Left, &parent);
    }
    let name = path
        .file_name()
        .ok_or("no name")?
        .to_string_lossy()
        .into_owned();
    panel::enter(app, PaneId::Left, &name)?;
    Ok(())
}

/// Ctrl+digit in the left panel; return where the panel went.
fn hot_key(app: &mut App, digit: &str, home: &Path) -> Result<Option<PathBuf>, String> {
    panel::focus_list(app, PaneId::Left)?;
    input::chord(app.slint(), &[Key::Control], digit);
    for _ in 0..40 {
        app.idle(Duration::from_millis(25));
        if app.location(PaneId::Left).as_deref() != Some(home) && !app.loading(PaneId::Left) {
            return Ok(app.location(PaneId::Left));
        }
    }
    Ok(None)
}

pub fn hot_paths(context: &mut Context) -> Result<(), String> {
    let root = context.work.join("hot");
    let [alpha, beta, gamma] = ["alpha", "beta", "gamma"].map(|name| root.join(name));
    for folder in [&alpha, &beta, &gamma] {
        fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    let store = SettingsStore::new();
    let mut app = App::start(config(&root, &store));

    // Add each folder from the manager's Add current: numbers 1, 2 and 3.
    for folder in [&alpha, &beta, &gamma] {
        go(&mut app, folder)?;
        open_manager(&mut app)?;
        options::press(&mut app, "Add current")?;
        let path = folder.clone();
        app.until("the new Hot Path row", move |app| {
            slot_of(app, &path).is_some()
        });
        close_manager(&mut app)?;
    }
    let expected_first = vec![
        (alpha.clone(), Some(1)),
        (beta.clone(), Some(2)),
        (gamma.clone(), Some(3)),
    ];
    // Hot Path edits save at once, without Options.
    let watched = store.clone();
    app.until("three saved Hot Paths", move |_| {
        watched.current().bookmarks.len() == 3
    });
    if saved(&store) != expected_first {
        return Err(format!("the saved Hot Paths are {:?}", saved(&store)));
    }

    // Reorder: give gamma number 1; alpha takes gamma's old number.
    open_manager(&mut app)?;
    let gamma_text = gamma.to_string_lossy().into_owned();
    let row = inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .find(|(label, _)| label.split('\n').next() == Some(gamma_text.as_str()))
        .map(|(_, item)| inspect::rect(&item))
        .ok_or("the gamma row is not shown")?;
    options::click_rect(&mut app, row);
    options::pick(&mut app, "Hot Path shortcut", "Hot Path 1")?;
    options::press(&mut app, "Apply shortcut")?;
    let (watched, moved) = (store.clone(), gamma.clone());
    app.until("gamma to take Hot Path 1", move |_| {
        watched
            .current()
            .bookmarks
            .iter()
            .any(|bookmark| bookmark.path == moved && bookmark.slot == Some(1))
    });
    let shown = slot_of(&app, &gamma).unwrap_or_default();
    if !shown.starts_with("Hot Path 1") {
        return Err(format!("the manager shows gamma's number as {shown:?}"));
    }
    options::review_shot(
        &mut app,
        context,
        "hot-paths-reordered",
        "Hot Paths after exchanging numbers",
    );

    // Remove beta with its row's remove button.
    let beta_text = beta.to_string_lossy().into_owned();
    let remove = format!("Remove Hot Path: {beta_text}\nThe folder is kept.");
    input::click_button(app.slint(), &remove)?;
    let beta_gone = beta.clone();
    app.until("beta's row to go", move |app| {
        slot_of(app, &beta_gone).is_none()
    });
    close_manager(&mut app)?;
    let watched = store.clone();
    app.until("two saved Hot Paths", move |_| {
        watched.current().bookmarks.len() == 2
    });
    if !beta.is_dir() {
        return Err("removing the Hot Path removed its folder".into());
    }
    let after = saved(&store);
    let slot = |path: &Path| {
        after
            .iter()
            .find(|(saved, _)| saved == path)
            .map(|(_, slot)| *slot)
    };
    if slot(&gamma) != Some(Some(1)) || slot(&alpha) != Some(Some(3)) || slot(&beta).is_some() {
        return Err(format!(
            "after reordering and removing the saved Hot Paths are {after:?}"
        ));
    }

    // Ctrl+digit opens them; the removed number does nothing.
    let home = root.clone();
    go_home(&mut app, &home)?;
    if hot_key(&mut app, "1", &home)?.as_deref() != Some(gamma.as_path()) {
        return Err("Ctrl+1 did not open gamma".into());
    }
    go_home(&mut app, &home)?;
    if hot_key(&mut app, "3", &home)?.as_deref() != Some(alpha.as_path()) {
        return Err("Ctrl+3 did not open alpha".into());
    }
    go_home(&mut app, &home)?;
    if let Some(went) = hot_key(&mut app, "2", &home)? {
        return Err(format!("Ctrl+2 (removed) opened {}", went.display()));
    }

    // Restart: the numbers remain.
    app.shutdown()?;
    drop(app);
    let mut app = App::start(config(&root, &store));
    go_home(&mut app, &home)?;
    if hot_key(&mut app, "1", &home)?.as_deref() != Some(gamma.as_path()) {
        return Err("after a restart Ctrl+1 did not open gamma".into());
    }
    context.note("Hot Paths: Add current saved alpha, beta, gamma as 1–3; giving gamma number 1 exchanged it with alpha (3); removing beta kept the folder; Ctrl+1/Ctrl+3 opened gamma/alpha, Ctrl+2 nothing, also after a restart");
    app.shutdown()
}

fn go_home(app: &mut App, home: &Path) -> Result<(), String> {
    if app.location(PaneId::Left).as_deref() == Some(home) {
        return Ok(());
    }
    panel::focus_list(app, PaneId::Left)?;
    input::key(app.slint(), Key::Backspace);
    panel::wait_location(app, PaneId::Left, home);
    Ok(())
}
