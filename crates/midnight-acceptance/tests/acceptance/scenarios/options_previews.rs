//! Options › Previews (thumbnail switches and size, seen in the Thumbnails panel view)
//! and Options › Plugins (its sections, empty states without staged plugins, Rescan
//! and the link to Archives); Cancel, OK and a restart.
use crate::{
    Context,
    app::{App, Config},
    input, inspect,
    options_support::{self as options, SettingsStore},
    panel,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use std::{fs, path::Path, sync::Arc, time::Duration};

const PREVIEWS: &str = "Show image and video previews";
const NETWORK: &str = "Allow previews on network locations";
const ARCHIVES: &str = "Allow previews inside archives";

fn config(folder: &Path, store: &Arc<SettingsStore>) -> Config {
    let mut config = Config::new(folder, folder);
    config.settings = Some(store.clone());
    config
}

fn row_height(app: &App) -> f32 {
    app.window.get_left_view_row_height()
}

pub fn previews_plugins(context: &mut Context) -> Result<(), String> {
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
    // "Archives" is also a topic button; the section tab is the one to its right.
    let tab = |app: &App, tab: &str| -> Result<inspect::Rect, String> {
        inspect::find_all(app.slint(), AccessibleRole::Button, tab)
            .into_iter()
            .map(|item| inspect::rect(&item))
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .ok_or_else(|| format!("the Plugins page has no {tab} section"))
    };
    let mut empty = Vec::new();
    for (name, empty_text) in [
        ("Viewers", "No viewer plugins are installed"),
        ("Filesystems", "No filesystem plugins are installed"),
        ("Archives", "No archive plugins are installed"),
        ("Thumbnailers", "No thumbnailer plugins are installed"),
    ] {
        let rect = tab(&app, name)?;
        options::click_rect(&mut app, rect);
        if !options::texts_with(&app, empty_text).is_empty() {
            empty.push(name);
        }
    }
    options::press(&mut app, "Rescan all plugins")?;
    app.until_for("the rescan to finish", Duration::from_secs(30), |app| {
        options::texts_with(app, "Rescanning").is_empty()
    });
    let rect = tab(&app, "Archives")?;
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
