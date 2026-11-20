//! Options › Appearance: theme, spacing, font family, desktop and touch font sizes and
//! text rendering, each with its visible effect; Cancel restores the previewed theme,
//! OK saves, a restart shows the saved appearance and Reset to defaults restores it.
use crate::{
    Context,
    app::{App, Config},
    options_support::{self as options, Frame, SettingsStore},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{GlyphFitting, PaneId, PreferenceTheme, UiDensity};
use std::{path::Path, sync::Arc};

const SPACING_DESKTOP: &str = "Desktop / mouse · compact";
const SPACING_TOUCH: &str = "Touch · larger controls";
/// The theme ids `Theme.scheme` uses (PreferenceTheme order).
const BLUE: i32 = 0;
const DARK: i32 = 1;
const LIGHT: i32 = 2;

fn config(folder: &Path, store: &Arc<SettingsStore>) -> Config {
    let mut config = Config::new(folder, folder);
    config.settings = Some(store.clone());
    config
}

fn row_height(app: &App) -> f32 {
    app.window.get_left_view_row_height()
}

pub fn appearance(context: &mut Context) -> Result<(), String> {
    let folder = context.fixture.path("code");
    let store = SettingsStore::new();
    let mut app = App::start(config(&folder, &store));
    if app.window.get_theme_index() != BLUE {
        return Err(format!(
            "the default theme is {} instead of Blue",
            app.window.get_theme_index()
        ));
    }
    let blue_luma = Frame::of(&mut app).mean_luma();

    // Theme preview, then Cancel: the previous theme returns and nothing is saved.
    options::open(&mut app)?;
    options::page(&mut app, "Appearance")?;
    options::choose(&mut app, "Theme", "Light")?;
    app.until("the Light theme preview", |app| {
        app.window.get_theme_index() == LIGHT
    });
    options::review_shot(
        &mut app,
        context,
        "appearance-light-preview",
        "Options › Appearance previewing the Light theme",
    );
    options::cancel(&mut app)?;
    app.until("Cancel to restore the Blue theme", |app| {
        app.window.get_theme_index() == BLUE
    });
    app.idle(std::time::Duration::from_millis(300));
    let restored = Frame::of(&mut app).mean_luma();
    if (restored - blue_luma).abs() > 2.0 || store.saves() != 0 {
        return Err(format!(
            "Cancel did not restore the Blue theme exactly: mean luma {blue_luma:.1} before, {restored:.1} after; {} saves",
            store.saves()
        ));
    }

    // Theme: the panels follow the previewed theme (the right list is mostly outside
    // the dialog); Light is much brighter than Dark.
    options::open(&mut app)?;
    options::page(&mut app, "Appearance")?;
    options::choose(&mut app, "Theme", "Light")?;
    app.until("the Light theme", |app| {
        app.window.get_theme_index() == LIGHT
    });
    app.idle(std::time::Duration::from_millis(300));
    let light_list = Frame::of(&mut app).luma(options::list_area(&app, PaneId::Right));
    options::choose(&mut app, "Theme", "Dark")?;
    app.until("the Dark theme", |app| app.window.get_theme_index() == DARK);
    app.idle(std::time::Duration::from_millis(300));
    let dark_list = Frame::of(&mut app).luma(options::list_area(&app, PaneId::Right));
    if light_list < dark_list + 60.0 {
        return Err(format!(
            "the theme preview is not visible: panel luma {light_list:.1} in Light, {dark_list:.1} in Dark"
        ));
    }

    // Desktop font size: rows grow with the text.
    let desktop_rows = row_height(&app);
    options::choose(&mut app, "Desktop font", "16")?;
    app.until("larger desktop rows", move |app| {
        row_height(app) > desktop_rows
    });
    let larger_desktop_rows = row_height(&app);

    // Font family: the panel text renders with the chosen face.
    app.until("the installed font list", |app| {
        app.window.get_font_list_ready()
    });
    let list = options::list_area(&app, PaneId::Right);
    let before_font = Frame::of(&mut app).digest(list);
    options::choose(&mut app, "Font family", "DejaVu Sans Mono")?;
    app.idle(std::time::Duration::from_millis(300));
    let after_font = Frame::of(&mut app).digest(list);
    let mono_installed = ["/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"]
        .iter()
        .any(|path| Path::new(path).is_file());
    if before_font == after_font {
        if mono_installed {
            return Err("choosing DejaVu Sans Mono did not change the panel text".into());
        }
        context.note("DejaVu Sans Mono is not installed here: the font change is verified by selection and storage only");
    }

    // Spacing: touch rows are taller; the touch font size applies to them.
    options::choose(&mut app, "Spacing", SPACING_TOUCH)?;
    app.until("touch spacing", move |app| {
        app.window.get_density_index() == 1 && row_height(app) > larger_desktop_rows
    });
    // Touch rows keep their 40 px minimum up to 28 px text; the text itself grows.
    let list = options::list_area(&app, PaneId::Right);
    let before_touch = Frame::of(&mut app).digest(list);
    options::choose(&mut app, "Touch font", "18")?;
    app.until("the 18 px touch font", |app| {
        app.window.get_touch_font_size() == 18
    });
    app.idle(std::time::Duration::from_millis(300));
    if Frame::of(&mut app).digest(list) == before_touch {
        return Err("the 18 px touch font did not change the panel text".into());
    }
    let final_rows = row_height(&app);

    // Text rendering takes effect after a restart; the choice itself is stored.
    options::choose(&mut app, "Text rendering", "Smooth (subpixel)")?;
    if app.window.get_glyph_fitting_index() != 2 {
        return Err(format!(
            "Text rendering shows index {} after choosing Smooth",
            app.window.get_glyph_fitting_index()
        ));
    }
    options::review_shot(
        &mut app,
        context,
        "appearance-changed",
        "Options › Appearance: Dark, touch spacing, DejaVu Sans Mono, 16/18 px, smooth text",
    );
    if !options::ok(&mut app, &store)? {
        return Err("OK did not save the appearance".into());
    }
    let saved = store.current();
    if saved.theme != PreferenceTheme::Dark
        || saved.density != UiDensity::Touch
        || saved.font_family != "DejaVu Sans Mono"
        || saved.desktop_font_size != 16
        || saved.touch_font_size != 18
        || saved.glyph_fitting != GlyphFitting::Subpixel
    {
        return Err(format!(
            "OK saved theme {:?}, density {:?}, font {:?}, sizes {}/{}, rendering {:?}",
            saved.theme,
            saved.density,
            saved.font_family,
            saved.desktop_font_size,
            saved.touch_font_size,
            saved.glyph_fitting
        ));
    }
    app.screenshot(
        context,
        "appearance-saved",
        "Main window after OK: Dark, touch spacing, DejaVu Sans Mono",
    );

    // Restart on the saved settings: the same appearance and row height.
    app.shutdown()?;
    drop(app);
    let mut app = App::start(config(&folder, &store));
    app.idle(std::time::Duration::from_millis(300));
    if app.window.get_theme_index() != DARK
        || app.window.get_density_index() != 1
        || (row_height(&app) - final_rows).abs() > 0.5
    {
        return Err(format!(
            "after a restart: theme {}, density {}, row height {} (was {final_rows})",
            app.window.get_theme_index(),
            app.window.get_density_index(),
            row_height(&app)
        ));
    }
    options::open(&mut app)?;
    options::page(&mut app, "Appearance")?;
    app.until("the installed font list", |app| {
        app.window.get_font_list_ready()
    });
    for (label, wanted) in [
        ("Theme", "Dark"),
        ("Spacing", SPACING_TOUCH),
        ("Font family", "DejaVu Sans Mono"),
        ("Desktop font", "16"),
        ("Touch font", "18"),
        ("Text rendering", "Smooth (subpixel)"),
    ] {
        let shown = options::value(&app, AccessibleRole::Combobox, label);
        if shown.as_deref() != Some(wanted) {
            return Err(format!(
                "after a restart {label} shows {shown:?} instead of {wanted:?}"
            ));
        }
    }
    options::review_shot(
        &mut app,
        context,
        "appearance-restarted",
        "Options › Appearance after a restart",
    );

    // Reset to defaults restores the default appearance.
    options::press(&mut app, "Reset to defaults")?;
    app.until("the Blue theme after Reset to defaults", |app| {
        app.window.get_theme_index() == BLUE && app.window.get_density_index() == 0
    });
    for (label, wanted) in [
        ("Theme", "Blue"),
        ("Spacing", SPACING_DESKTOP),
        ("Font family", "Application default"),
        ("Desktop font", "12"),
        ("Touch font", "15"),
        ("Text rendering", "Sharp (fitted)"),
    ] {
        let shown = options::value(&app, AccessibleRole::Combobox, label);
        if shown.as_deref() != Some(wanted) {
            return Err(format!(
                "after Reset to defaults {label} shows {shown:?} instead of {wanted:?}"
            ));
        }
    }
    if !options::ok(&mut app, &store)? {
        return Err("OK after Reset to defaults saved nothing".into());
    }
    let saved = store.current();
    if saved.theme != PreferenceTheme::Blue
        || saved.density != UiDensity::Desktop
        || !saved.font_family.is_empty()
        || saved.desktop_font_size != 12
        || saved.touch_font_size != 15
        || saved.glyph_fitting != GlyphFitting::Fitted
    {
        return Err(format!(
            "Reset to defaults saved theme {:?}, density {:?}, font {:?}, sizes {}/{}, rendering {:?}",
            saved.theme,
            saved.density,
            saved.font_family,
            saved.desktop_font_size,
            saved.touch_font_size,
            saved.glyph_fitting
        ));
    }
    context.note("Appearance: Cancel restored the previewed theme; Light/Dark, 16 px desktop and 18 px touch fonts, touch spacing and the font face changed the panels; OK saved them with smooth text rendering; a restart showed them; Reset to defaults restored Blue/Desktop/12/15/Sharp. Text rendering configures the FemtoVG renderer at startup; the software renderer used here ignores it, so it is verified by storage and its display after the restart");
    app.shutdown()
}
