//! Options › Previews (thumbnail switches and size, seen in the Thumbnails panel view)
//! and Options › Plugins (its sections, empty states without staged plugins, Rescan
//! and the link to Archives); Cancel, OK and a restart. Then, in a staged child with
//! the installed plugin packages beside it (`viewers_stage::run`), the Plugins page
//! lists every package family instead of its empty state, and Rescan reports what the
//! sections list; without packages to stage that part is reported as skipped.
use crate::{
    Context,
    app::{App, Config},
    input, inspect,
    options_support::{self as options, SettingsStore},
    panel, viewers_stage,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use std::{fs, path::Path, sync::Arc, time::Duration};

const PREVIEWS: &str = "Show image and video previews";
const NETWORK: &str = "Allow previews on network locations";
const ARCHIVES: &str = "Allow previews inside archives";
/// The registered timeout of `options_previews_plugins`, which the staged child also
/// needs to know (its deadline counts from the scenario's start).
const TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// A hang guard for the plugin catalogs and a rescan.
const CATALOGS: Duration = Duration::from_secs(30);
/// The Plugins sections: the tab, its list's label, its empty state and the word the
/// rescan report counts it with.
const SECTIONS: [(&str, &str, &str, &str); 4] = [
    (
        "Viewers",
        "Viewer priority",
        "No viewer plugins are installed",
        "viewer",
    ),
    (
        "Filesystems",
        "Filesystem plugins",
        "No filesystem plugins are installed",
        "filesystem",
    ),
    (
        "Archives",
        "Archive plugins",
        "No archive plugins are installed",
        "archive",
    ),
    (
        "Thumbnailers",
        "Thumbnailer plugins",
        "No thumbnailer plugins are installed",
        "thumbnailer",
    ),
];

/// The Plugins page's section tab: "Archives" is also a topic button, and the section
/// tab is the one to its right.
fn section_tab(app: &App, tab: &str) -> Result<inspect::Rect, String> {
    inspect::find_all(app.slint(), AccessibleRole::Button, tab)
        .into_iter()
        .map(|item| inspect::rect(&item))
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .ok_or_else(|| format!("the Plugins page has no {tab} section"))
}

/// The page's texts that speak of plugins, for messages.
fn plugin_texts(app: &App) -> Vec<String> {
    options::texts(app)
        .into_iter()
        .filter(|text| text.contains("plugin") || text.contains("Rescan"))
        .collect()
}

fn config(folder: &Path, store: &Arc<SettingsStore>) -> Config {
    let mut config = Config::new(folder, folder);
    config.settings = Some(store.clone());
    config
}

fn row_height(app: &App) -> f32 {
    app.window.get_left_view_row_height()
}

pub fn previews_plugins(context: &mut Context) -> Result<(), String> {
    if let Some(reason) =
        viewers_stage::run_with_optional_plugins(context, TIMEOUT, previews, plugins_installed)?
    {
        let skipped = format!("SKIP the Plugins page with the installed packages: {reason}");
        context.note(skipped.clone());
        context.outcome.message = skipped;
    }
    Ok(())
}

fn previews(context: &mut Context) -> Result<(), String> {
    let folder = context.work.join("previews");
    fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    for name in ["one.png", "two.txt", "three.bin"] {
        fs::write(folder.join(name), name).map_err(|error| error.to_string())?;
    }
    let store = SettingsStore::new();
    let mut app = App::start(config(&folder, &store));
    // The Thumbnails view (Alt+5): its rows follow the thumbnail size.
    let detailed = row_height(&app);
    panel::focus_list(&mut app, PaneId::Left)?;
    input::chord(app.slint(), &[slint::platform::Key::Alt], "5");
    app.until("the Thumbnails view", move |app| {
        row_height(app) > detailed + 40.0
    });
    let at_96 = row_height(&app);

    options::open(&mut app)?;
    options::page(&mut app, "Previews")?;
    for (label, wanted) in [(PREVIEWS, true), (NETWORK, false), (ARCHIVES, true)] {
        if options::checked(&app, label) != Some(wanted) {
            return Err(format!(
                "{label:?} starts as {:?}",
                options::checked(&app, label)
            ));
        }
    }
    let size = options::value(&app, AccessibleRole::Combobox, "Thumbnail size");
    if size.as_deref() != Some("96 px (default)") {
        return Err(format!("Thumbnail size starts as {size:?}"));
    }
    options::choose(&mut app, "Thumbnail size", "192 px")?;
    app.until("192 px thumbnail rows", move |app| {
        (row_height(app) - (at_96 + 96.0)).abs() < 1.0
    });
    options::set_checked(&mut app, NETWORK, true)?;
    options::set_checked(&mut app, ARCHIVES, false)?;
    options::set_checked(&mut app, PREVIEWS, false)?;
    // Without previews the dependent switches are disabled.
    for label in [NETWORK, ARCHIVES] {
        if options::is_enabled(&app, AccessibleRole::Checkbox, label) {
            return Err(format!("{label:?} stays enabled with previews off"));
        }
    }
    options::review_shot(
        &mut app,
        context,
        "previews-changed",
        "Options › Previews changed",
    );
    if !options::ok(&mut app, &store)? {
        return Err("OK did not save the preview settings".into());
    }
    let saved = store.current();
    if saved.thumbnails_enabled
        || !saved.thumbnails_network
        || saved.thumbnails_archives
        || saved.thumbnail_size != 192
    {
        return Err(format!(
            "saved previews {} network {} archives {} size {}",
            saved.thumbnails_enabled,
            saved.thumbnails_network,
            saved.thumbnails_archives,
            saved.thumbnail_size
        ));
    }
    let at_192 = row_height(&app);

    // Cancel restores a previewed size.
    options::open(&mut app)?;
    options::page(&mut app, "Previews")?;
    options::choose(&mut app, "Thumbnail size", "64 px")?;
    app.until("64 px thumbnail rows", move |app| {
        row_height(app) < at_192 - 100.0
    });
    options::cancel(&mut app)?;
    app.until("Cancel to restore 192 px rows", move |app| {
        (row_height(app) - at_192).abs() < 1.0
    });
    if store.current().thumbnail_size != 192 {
        return Err("Cancel saved the 64 px size".into());
    }

    // Plugins: every section, its empty state here, Rescan and the Archives link.
    options::open(&mut app)?;
    options::page(&mut app, "Plugins")?;
    options::review_shot(
        &mut app,
        context,
        "plugins-viewers",
        "Options › Plugins › Viewers",
    );
    // This process installs no plugin packages: every section must say so.
    let mut empty = Vec::new();
    let mut missing = Vec::new();
    for (name, _, empty_text, _) in SECTIONS {
        let rect = section_tab(&app, name)?;
        options::click_rect(&mut app, rect);
        let shown = options::poll(&mut app, Duration::from_secs(2), |app| {
            !options::texts_with(app, empty_text).is_empty()
        });
        if shown {
            empty.push(name);
        } else {
            missing.push(format!(
                "{name}: {empty_text:?} not shown; texts {:?}",
                options::texts(&app)
                    .into_iter()
                    .filter(|text| text.contains("plugin"))
                    .collect::<Vec<_>>()
            ));
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "Plugins sections without installed plugins do not show their empty state:\n{}",
            missing.join("\n")
        ));
    }
    options::press(&mut app, "Rescan all plugins")?;
    app.until_for("the rescan to finish", Duration::from_secs(30), |app| {
        options::texts_with(app, "Rescanning").is_empty()
    });
    let rect = section_tab(&app, "Archives")?;
    options::click_rect(&mut app, rect);
    options::press(&mut app, "Archive associations…")?;
    app.until("the Archives page", |app| {
        app.window.get_options_topic_index() == 7
    });
    options::cancel(&mut app)?;

    // Restart: the saved preview settings and the Thumbnails view size.
    app.shutdown()?;
    drop(app);
    let mut app = App::start(config(&folder, &store));
    if (row_height(&app) - at_192).abs() > 1.0 {
        return Err(format!(
            "after a restart the Thumbnails rows are {} px, not {at_192}",
            row_height(&app)
        ));
    }
    options::open(&mut app)?;
    options::page(&mut app, "Previews")?;
    for (label, wanted) in [(PREVIEWS, false), (NETWORK, true), (ARCHIVES, false)] {
        if options::checked(&app, label) != Some(wanted) {
            return Err(format!(
                "after a restart {label:?} is {:?}",
                options::checked(&app, label)
            ));
        }
    }
    let size = options::value(&app, AccessibleRole::Combobox, "Thumbnail size");
    if size.as_deref() != Some("192 px") {
        return Err(format!("after a restart Thumbnail size shows {size:?}"));
    }
    options::cancel(&mut app)?;
    context.note(format!(
        "Previews: 192 px grew Thumbnails rows by 96 px, previews off disabled the dependent switches, OK saved all four values, Cancel restored a 64 px preview, a restart kept them. Plugins: sections without staged plugins show their empty state ({empty:?}); Rescan finished; Archive associations… opened the Archives page"
    ));
    app.shutdown()
}

/// Options › Plugins with the installed packages beside this executable (the staged
/// child): every section lists them instead of its empty state, as many rows as there
/// are packages of its family at least, and Rescan reports what the sections list.
fn plugins_installed(context: &mut Context) -> Result<(), String> {
    let folder = context.work.join("plugins-installed");
    fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    let store = SettingsStore::new();
    let mut app = App::start(config(&folder, &store));
    options::open(&mut app)?;
    options::page(&mut app, "Plugins")?;
    let mut listed: Vec<(&str, Vec<String>)> = Vec::new();
    for (name, list, empty_text, _) in SECTIONS {
        let rect = section_tab(&app, name)?;
        options::click_rect(&mut app, rect);
        let ready = options::poll(&mut app, CATALOGS, |app| {
            !options::list_rows(app, list).is_empty()
                && options::texts_with(app, "Refreshing plugin catalogs").is_empty()
        });
        let rows: Vec<String> = options::list_rows(&app, list)
            .into_iter()
            .map(|(label, _)| label)
            .collect();
        if !ready
            || !options::texts_with(&app, empty_text).is_empty()
            || rows
                .iter()
                .any(|row| row.trim().is_empty() || row.contains("Not installed"))
        {
            return Err(format!(
                "with the installed packages the {name} section lists {rows:?}; the page shows {:?}",
                plugin_texts(&app)
            ));
        }
        options::review_shot(
            &mut app,
            context,
            &format!("plugins-installed-{}", name.to_lowercase()),
            &format!("Options › Plugins › {name} with the installed packages"),
        );
        listed.push((name, rows));
    }
    let count = |section: &str| {
        listed
            .iter()
            .find(|(name, _)| *name == section)
            .map_or(0, |(_, rows)| rows.len())
    };
    let packages = [
        ("viewers", viewers_stage::installed_packages("viewers")),
        (
            "filesystems",
            viewers_stage::installed_packages("filesystems"),
        ),
        (
            "thumbnailers",
            viewers_stage::installed_packages("thumbnailers"),
        ),
    ];
    if count("Viewers") < packages[0].1
        || count("Filesystems") + count("Archives") < packages[1].1
        || count("Thumbnailers") < packages[2].1
    {
        return Err(format!(
            "the Plugins page lists {listed:?} for the installed packages {packages:?}"
        ));
    }

    // Rescan reports what the sections list, every package available.
    options::press(&mut app, "Rescan all plugins")?;
    if !options::poll(&mut app, CATALOGS, |app| {
        !options::texts_with(app, "Rescanned").is_empty()
    }) {
        return Err(format!(
            "Rescan all plugins reported nothing; the page shows {:?}",
            plugin_texts(&app)
        ));
    }
    let report = options::texts_with(&app, "Rescanned").join(" ");
    let missing: Vec<String> = SECTIONS
        .into_iter()
        .map(|(name, _, _, kind)| format!(" {} {kind}", count(name)))
        .filter(|part| !report.contains(part.as_str()))
        .collect();
    if !report.starts_with("Rescanned: found")
        || report.contains("unavailable")
        || !missing.is_empty()
    {
        return Err(format!(
            "Rescan all plugins reported {report:?}, without {missing:?} that the sections list ({listed:?})"
        ));
    }
    // The section shown last still lists the same packages.
    let (last, list) = (SECTIONS[3].0, SECTIONS[3].1);
    if options::list_rows(&app, list).len() != count(last) {
        return Err(format!(
            "after Rescan the {last} section lists {:?}",
            options::list_rows(&app, list)
        ));
    }
    options::cancel(&mut app)?;
    context.note(format!(
        "Plugins with the installed packages {packages:?}: every section lists them ({listed:?}), none shows its empty state; Rescan reported {report:?}"
    ));
    app.shutdown()
}
