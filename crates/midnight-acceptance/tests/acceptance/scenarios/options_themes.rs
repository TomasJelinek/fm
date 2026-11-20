//! Options › Appearance › Theme, every theme the picker offers: each choice previews at
//! once (the theme changes and the panels behind Options render differently from every
//! other theme; themes named Light are bright and Dark ones dark), the picker shows it,
//! and OK saves the last one, which a restart keeps. In a narrow touch window the topic
//! picker replaces the page list: a page chosen there, a theme changed and saved, and a
//! cancelled check box all behave as in the wide layout.
use crate::{
    Context,
    app::{App, Config},
    input, inspect,
    options_support::{self as options, Frame, SettingsStore},
};
use i_slint_core::{
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Clip},
};
use midnight_core::{PaneId, PreferenceTheme, SizeDisplay, UiDensity};
use slint::platform::Key;
use std::{path::Path, sync::Arc, time::Duration};

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

fn config(folder: &Path, store: &Arc<SettingsStore>) -> Config {
    let mut config = Config::new(folder, folder);
    config.settings = Some(store.clone());
    config
}

/// The items the picker labelled `label` offers, read from its open popup, which is
/// then closed by choosing the shown item again.
fn offered(app: &mut App, label: &str) -> Result<Vec<String>, String> {
    let current = options::value(app, AccessibleRole::Combobox, label)
        .ok_or_else(|| format!("no {label:?} picker"))?;
    let picker = options::reveal(app, AccessibleRole::Combobox, label)?;
    let popups = inspect::roots(app.slint()).len();
    options::click_item(app, &picker)?;
    if inspect::roots(app.slint()).len() <= popups {
        return Err(format!("the {label:?} picker opened no list"));
    }
    let mut items = Vec::new();
    if let Some(popup) = inspect::roots(app.slint()).last() {
        inspect::walk(popup, &mut |item| {
            if item.accessible_role() == AccessibleRole::ListItem
                && shown(item)
                && let Some(name) = inspect::label(item)
            {
                items.push(name);
            }
        });
    }
    input::key(app.slint(), Key::Escape);
    app.idle(Duration::from_millis(100));
    if inspect::roots(app.slint()).len() > popups {
        options::choose(app, label, &current)?;
    }
    Ok(items)
}

pub fn themes(context: &mut Context) -> Result<(), String> {
    let folder = context.fixture.path("code");
    let store = SettingsStore::new();
    let mut app = App::start(config(&folder, &store));
    let mut failures: Vec<String> = Vec::new();
    options::open(&mut app)?;
    options::page(&mut app, "Appearance")?;
    if !app.window.get_options_open() {
        return Err("Options closed while reading the theme list".into());
    }
    let names = offered(&mut app, "Theme")?;
    if names.len() < 20 {
        return Err(format!("the Theme picker offers only {names:?}"));
    }
    if !app.window.get_options_open() {
        return Err("closing the theme list closed Options".into());
    }
    let list = options::list_area(&app, PaneId::Right);
    let mut seen: Vec<(String, i32, u64, f32)> = Vec::new();
    for name in &names {
        let before = app.window.get_theme_index();
        if let Err(error) = options::choose(&mut app, "Theme", name) {
            failures.push(format!("choosing {name:?}: {error}"));
            continue;
        }
        let first = seen.is_empty();
        for _ in 0..40 {
            if app.window.get_theme_index() != before || first {
                break;
            }
            app.idle(Duration::from_millis(25));
        }
        // Theme colours animate briefly.
        app.idle(Duration::from_millis(300));
        let frame = Frame::of(&mut app);
        let index = app.window.get_theme_index();
        let digest = frame.digest(list);
        let luma = frame.luma(list);
        if let Some((other, _, _, _)) = seen
            .iter()
            .find(|(_, known, pixels, _)| *known == index || *pixels == digest)
        {
            failures.push(format!("{name:?} previews like {other:?} (theme {index})"));
        }
        let lower = name.to_lowercase();
        if lower.contains("light") && luma < 140.0 {
            failures.push(format!("{name:?} renders dark panels (luma {luma:.0})"));
        }
        if lower.contains("dark") && luma > 110.0 {
            failures.push(format!("{name:?} renders bright panels (luma {luma:.0})"));
        }
        seen.push((name.clone(), index, digest, luma));
    }
    options::review_shot(
        &mut app,
        context,
        "last-theme",
        "Options › Appearance after previewing every theme",
    );
    let last = names.last().cloned().unwrap_or_default();
    let last_index = app.window.get_theme_index();
    if !options::ok(&mut app, &store)? {
        failures.push("OK did not save the theme".into());
    }
    let saved = store.current().theme;
    if saved as i32 != last_index {
        failures.push(format!(
            "OK saved {saved:?} ({}) while {last:?} ({last_index}) was shown",
            saved as i32
        ));
    }
    app.shutdown()?;
    drop(app);
    let mut app = App::start(config(&folder, &store));
    app.idle(Duration::from_millis(300));
    if app.window.get_theme_index() != last_index {
        failures.push(format!(
            "after a restart the theme is {} instead of {last:?} ({last_index})",
            app.window.get_theme_index()
        ));
    }
    options::open(&mut app)?;
    options::page(&mut app, "Appearance")?;
    let shown_name = options::value(&app, AccessibleRole::Combobox, "Theme");
    if shown_name.as_deref() != Some(last.as_str()) {
        failures.push(format!(
            "after a restart the Theme picker shows {shown_name:?}, not {last:?}"
        ));
    }
    options::cancel(&mut app)?;
    app.shutdown()?;
    drop(app);
    context.note(format!(
        "{} themes previewed, each distinct; lumas {:?}",
        seen.len(),
        seen.iter()
            .map(|(name, _, _, luma)| format!("{name} {luma:.0}"))
            .collect::<Vec<_>>()
    ));

    // A narrow touch window: the topic picker replaces the list of pages.
    let store = SettingsStore::new();
    let mut narrow = config(&folder, &store);
    narrow.width = 720;
    narrow.height = 560;
    narrow.density = UiDensity::Touch;
    narrow.theme = PreferenceTheme::Dark;
    let mut app = App::start(narrow);
    options::open(&mut app)?;
    if options::find_any(&app, AccessibleRole::Combobox, "Options topic").is_none() {
        failures.push("a narrow window's Options shows no topic picker".into());
    }
    if options::find_any(&app, AccessibleRole::Button, "Appearance").is_some() {
        failures.push("a narrow window's Options still shows the page list".into());
    }
    options::page(&mut app, "Appearance")?;
    options::choose(&mut app, "Theme", "Light")?;
    options::review_shot(
        &mut app,
        context,
        "narrow-light",
        "Options in a narrow touch window: Appearance chosen with the topic picker",
    );
    if !options::ok(&mut app, &store)? || store.current().theme != PreferenceTheme::Light {
        failures.push(format!(
            "OK in the narrow layout saved {:?}",
            store.current().theme
        ));
    }
    options::open(&mut app)?;
    options::page(&mut app, "General")?;
    options::set_checked(&mut app, "Always show file sizes in bytes", true)?;
    let saves = store.saves();
    options::cancel(&mut app)?;
    if store.saves() != saves || store.current().size_display != SizeDisplay::Readable {
        failures.push("Cancel in the narrow layout saved the cancelled check box".into());
    }
    app.shutdown()?;
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}
