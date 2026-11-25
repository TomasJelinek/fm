//! Exercise both Appearance pickers through their accessible popup lists. Check the
//! rendered result in each host mode, inactive edits, persistence, Cancel and narrow touch.
//! The narrow and compact topic pickers also support keyboard navigation through every page.
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
    let list = options::list_area(&app, PaneId::Right);
    let mut seen = Vec::new();
    for (label, light) in [("Dark theme", false), ("Light theme", true)] {
        app.set_host_dark(!light);
        let names = offered(&mut app, label)?;
        let count = PreferenceTheme::ALL
            .iter()
            .filter(|theme| theme.is_light() == light)
            .count();
        if names.len() != count || !app.window.get_options_open() {
            return Err(format!(
                "{label} expected {count} entries in Options, got {names:?}"
            ));
        }
        for name in &names {
            options::choose(&mut app, label, name)?;
            app.idle(Duration::from_millis(300));
            let index = app.window.get_theme_index();
            let choice = PreferenceTheme::from_index(index)
                .ok_or_else(|| format!("{label} {name:?} applied unknown ID {index}"))?;
            let frame = Frame::of(&mut app);
            let digest = frame.digest(list);
            let luma = frame.luma(list);
            if (light && luma < 140.0) || (!light && luma > 110.0) {
                return Err(format!(
                    "{label} {name:?} renders the panel with luma {luma:.0}"
                ));
            }
            if choice.is_light() != light {
                return Err(format!(
                    "{label} {name:?} applied the wrong appearance ({choice:?})"
                ));
            }
            if seen
                .iter()
                .any(|(other, pixels)| *other == choice || *pixels == digest)
            {
                return Err(format!(
                    "{label} {name:?} duplicates a previous theme ({choice:?})"
                ));
            }
            seen.push((choice, digest));
        }
    }

    // On a light host each pick is previewed, the dark one too; OK then shows the
    // choice that matches the host, not the last preview.
    app.set_host_dark(false);
    options::choose(&mut app, "Light theme", "Iris")?;
    if app.window.get_theme_index() != PreferenceTheme::Iris.index() {
        return Err("the Iris pick was not previewed".into());
    }
    options::choose(&mut app, "Dark theme", "Plum Smoke")?;
    if app.window.get_theme_index() != PreferenceTheme::PlumSmoke.index() || store.saves() != 0 {
        return Err(
            "the Plum Smoke pick on a light host was not previewed, or settings were written"
                .into(),
        );
    }
    options::review_shot(
        &mut app,
        context,
        "theme-pair",
        "Plum Smoke and Iris in Options",
    );
    if !options::ok(&mut app, &store)? {
        return Err("OK did not save both theme choices".into());
    }
    if app.window.get_theme_index() != PreferenceTheme::Iris.index() {
        return Err("OK on a light host did not show the light choice, Iris".into());
    }
    app.set_host_dark(true);
    if app.window.get_theme_index() != PreferenceTheme::PlumSmoke.index() {
        return Err("the dark host did not pick the saved Plum Smoke palette".into());
    }
    let saved = store.current();
    if (saved.dark_theme, saved.light_theme) != (PreferenceTheme::PlumSmoke, PreferenceTheme::Iris)
    {
        return Err(format!(
            "OK saved {:?} and {:?}",
            saved.dark_theme, saved.light_theme
        ));
    }
    app.shutdown()?;
    drop(app);

    let mut restart = config(&folder, &store);
    restart.theme = PreferenceTheme::DEFAULT_LIGHT;
    let mut app = App::start(restart);
    if app.window.get_theme_index() != PreferenceTheme::Iris.index() {
        return Err("a restart on a light host did not open with Iris".into());
    }
    options::open(&mut app)?;
    options::page(&mut app, "Appearance")?;
    for (label, expected) in [("Dark theme", "Plum Smoke"), ("Light theme", "Iris")] {
        if options::value(&app, AccessibleRole::Combobox, label).as_deref() != Some(expected) {
            return Err(format!("after restart {label} did not show {expected:?}"));
        }
    }
    let saves = store.saves();
    options::choose(&mut app, "Dark theme", "Warm Ash")?;
    options::choose(&mut app, "Light theme", "Faience")?;
    app.set_host_dark(true);
    if app.window.get_theme_index() != PreferenceTheme::Faience.index() {
        return Err("a host change replaced the preview of the last pick, Faience".into());
    }
    options::cancel(&mut app)?;
    if app.window.get_theme_index() != PreferenceTheme::PlumSmoke.index() {
        return Err("Cancel did not restore the saved dark theme after a host change".into());
    }
    app.set_host_dark(false);
    if app.window.get_theme_index() != PreferenceTheme::Iris.index() || store.saves() != saves {
        return Err(
            "Cancel or a host change altered the saved light theme or wrote settings".into(),
        );
    }
    app.shutdown()?;
    drop(app);
    context.note(format!(
        "{} distinct themes previewed across both appearance menus",
        seen.len()
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
    if options::find_any(&app, AccessibleRole::Combobox, TOPIC_PICKER).is_none() {
        failures.push("a narrow window's Options shows no topic picker".into());
    }
    if options::find_any(&app, AccessibleRole::Button, "Appearance").is_some() {
        failures.push("a narrow window's Options still shows the page list".into());
    }
    failures.extend(narrow_picker(&mut app, &store)?);
    options::page(&mut app, "Appearance")?;
    options::choose(&mut app, "Light theme", "Light")?;
    app.set_host_dark(false);
    options::review_shot(
        &mut app,
        context,
        "narrow-light",
        "Options in a narrow touch window: Appearance chosen with the topic picker",
    );
    if !options::ok(&mut app, &store)? || store.current().light_theme != PreferenceTheme::Light {
        failures.push(format!(
            "OK in the narrow layout saved {:?}",
            store.current().light_theme
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
    drop(app);

    // A window so low that Options is compact: the topic picker moves into the header.
    let store = SettingsStore::new();
    let mut compact = config(&folder, &store);
    compact.width = 560;
    compact.height = 330;
    let mut app = App::start(compact);
    options::open(&mut app)?;
    options::review_shot(
        &mut app,
        context,
        "compact-header-picker",
        "Options in a low narrow window: the topic picker in the header",
    );
    failures.extend(header_picker(&mut app, &store)?);
    app.shutdown()?;
    context.note("Topic pickers: the narrow combo box lists the pages in the displayed order, Shift+Tab from OK reaches it, Down and Up walk every page and stop at the ends with the focus kept; the compact header picker lists the same order, opens with Down and chooses with Enter; nothing was saved");
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

const TOPIC_PICKER: &str = "Options topic";

fn titles() -> Vec<String> {
    options::PAGES
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect()
}

/// Whether the topic picker has the keyboard focus.
fn picker_focused(app: &App) -> bool {
    options::focus(app)
        .is_some_and(|(role, label)| role == AccessibleRole::Combobox && label == TOPIC_PICKER)
}

/// The narrow layout's topic combo box: its list in the displayed order, reached from OK
/// with Shift+Tab, Down and Up walking the pages (and stopping at the ends) with the
/// focus kept on the picker. Options stays open on General and nothing is saved.
fn narrow_picker(app: &mut App, store: &SettingsStore) -> Result<Vec<String>, String> {
    let mut failures = Vec::new();
    let saves = store.saves();
    let listed = offered(app, TOPIC_PICKER)?;
    if listed != titles() {
        failures.push(format!(
            "the narrow topic picker lists {listed:?} instead of {:?}",
            titles()
        ));
    }
    if !app.window.get_options_open() || app.window.get_options_topic_index() != 0 {
        return Err(format!(
            "reading the topic picker's list closed Options ({}) or changed the page to {}",
            !app.window.get_options_open(),
            app.window.get_options_topic_index()
        ));
    }
    // From OK, as Options opens.
    input::key(app.slint(), Key::Escape);
    app.until("Escape to close Options", |app| {
        !app.window.get_options_open()
    });
    options::open(app)?;
    options::tab_until(app, true, 60, "the topic picker", &|role, label| {
        role == AccessibleRole::Combobox && label == TOPIC_PICKER
    })?;
    let walk = |app: &mut App, key: Key, topic: i32, failures: &mut Vec<String>| {
        input::key(app.slint(), key);
        let reached = options::poll(app, Duration::from_secs(2), |app| {
            app.window.get_options_topic_index() == topic && picker_focused(app)
        });
        if !reached {
            failures.push(format!(
                "{key:?} on the narrow topic picker should show topic {topic} and keep the focus; topic {}, focus {:?}",
                app.window.get_options_topic_index(),
                options::focus(app)
            ));
        }
    };
    let order: Vec<i32> = options::PAGES.iter().map(|(_, topic)| *topic).collect();
    for topic in order.iter().skip(1) {
        walk(app, Key::DownArrow, *topic, &mut failures);
    }
    walk(app, Key::DownArrow, order[order.len() - 1], &mut failures);
    for topic in order.iter().rev().skip(1) {
        walk(app, Key::UpArrow, *topic, &mut failures);
    }
    walk(app, Key::UpArrow, order[0], &mut failures);
    if !app.window.get_options_open() || store.saves() != saves {
        return Err("walking the narrow topic picker closed or saved Options".into());
    }
    Ok(failures)
}

/// The compact layout's header picker: its list in the displayed order; reached from
/// OK with Shift+Tab, Down opens it on the shown page, Down moves and Enter chooses;
/// Escape then closes Options without saving.
fn header_picker(app: &mut App, store: &SettingsStore) -> Result<Vec<String>, String> {
    let mut failures = Vec::new();
    let saves = store.saves();
    if options::find_any(app, AccessibleRole::Button, "Appearance").is_some() {
        failures.push("a compact Options window still shows the page list".into());
    }
    options::tab_until(
        app,
        true,
        60,
        "the header's topic picker",
        &|role, label| role == AccessibleRole::Combobox && label == TOPIC_PICKER,
    )?;
    let popups = inspect::roots(app.slint()).len();
    input::key(app.slint(), Key::DownArrow);
    if !options::poll(app, Duration::from_secs(2), |app| {
        inspect::roots(app.slint()).len() > popups
    }) {
        return Err(format!(
            "Down on the header's topic picker opened no list; focus {:?}",
            options::focus(app)
        ));
    }
    let mut rows: Vec<(String, f32)> = Vec::new();
    if let Some(popup) = inspect::roots(app.slint()).last() {
        inspect::walk(popup, &mut |item| {
            if item.accessible_role() == AccessibleRole::Button
                && shown(item)
                && let Some(name) = inspect::label(item)
            {
                rows.push((name, inspect::rect(item).1));
            }
        });
    }
    rows.sort_by(|a, b| a.1.total_cmp(&b.1));
    let listed: Vec<String> = rows.into_iter().map(|(name, _)| name).collect();
    // The list is virtualised: the rows it shows must be the pages' first ones, in order.
    let expected = titles();
    if listed.len() < 3 || !expected.starts_with(&listed) {
        failures.push(format!(
            "the header's topic picker lists {listed:?}, not a prefix of {expected:?}"
        ));
    }
    // Down from the shown page (General) to Keyboard, then Enter.
    let (keyboard_index, keyboard_topic) = options::PAGES
        .iter()
        .enumerate()
        .find(|(_, (name, _))| *name == "Keyboard")
        .map(|(index, (_, topic))| (index, *topic))
        .expect("Keyboard page");
    for _ in 0..keyboard_index {
        input::key(app.slint(), Key::DownArrow);
    }
    input::key(app.slint(), Key::Return);
    if !options::poll(app, Duration::from_secs(2), |app| {
        app.window.get_options_topic_index() == keyboard_topic
            && inspect::roots(app.slint()).len() == popups
    }) {
        failures.push(format!(
            "Down ×{keyboard_index} and Enter in the header's topic list should show Keyboard (topic {keyboard_topic}) and close the list; topic {}, popups {}",
            app.window.get_options_topic_index(),
            inspect::roots(app.slint()).len()
        ));
    }
    if !app.window.get_options_open() || store.saves() != saves {
        return Err("choosing a page in the header's topic picker closed or saved Options".into());
    }
    if !picker_focused(app) {
        failures.push(format!(
            "after choosing a page the focus is on {:?}, not on the header's topic picker",
            options::focus(app)
        ));
    }
    input::key(app.slint(), Key::Escape);
    app.until("Escape to close Options", |app| {
        !app.window.get_options_open()
    });
    if store.saves() != saves {
        failures.push("Escape after the header's topic picker saved settings".into());
    }
    Ok(failures)
}
