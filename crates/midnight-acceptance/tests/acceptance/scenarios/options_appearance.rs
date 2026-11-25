//! Options › Appearance: theme, application icon, spacing, font family, desktop and touch
//! font sizes and text rendering, each with its visible effect; Cancel restores previews,
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
const ICON_PICKER: &str = "Application icon";
/// The theme ids `Theme.scheme` uses (PreferenceTheme order).
const DEFAULT_DARK: i32 = PreferenceTheme::DEFAULT_DARK.index();
const DARK: i32 = PreferenceTheme::Dark.index();
const LIGHT: i32 = PreferenceTheme::Light.index();

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
    if app.window.get_theme_index() != DEFAULT_DARK {
        return Err(format!(
            "the default dark theme is {} instead of Pencil Lead",
            app.window.get_theme_index()
        ));
    }
    let initial_luma = Frame::of(&mut app).mean_luma();
    let moon = app.window.get_current_app_icon();

    // Preview a theme and icon, then Cancel to restore the previous appearance.
    options::open(&mut app)?;
    options::page(&mut app, "Appearance")?;
    if options::value(&app, AccessibleRole::Combobox, ICON_PICKER).as_deref()
        != Some("Midnight Moon")
    {
        return Err("the default application icon is not Midnight Moon".into());
    }
    options::choose(&mut app, ICON_PICKER, "Harvest Navigator")?;
    let harvest = app.window.get_current_app_icon();
    if harvest == moon {
        return Err("Harvest Navigator did not replace the window artwork".into());
    }
    options::choose(&mut app, "Dark theme", "Carbon Black")?;
    app.until("the Carbon Black theme preview", |app| {
        app.window.get_theme_index() == PreferenceTheme::CarbonBlack.index()
    });
    options::review_shot(
        &mut app,
        context,
        "appearance-dark-preview",
        "Options › Appearance previewing Carbon Black",
    );
    options::cancel(&mut app)?;
    app.until("Cancel to restore Pencil Lead", |app| {
        app.window.get_theme_index() == DEFAULT_DARK
    });
    if app.window.get_current_app_icon() != moon {
        return Err("Cancel did not restore the Midnight Moon artwork".into());
    }
    app.idle(std::time::Duration::from_millis(300));
    let restored = Frame::of(&mut app).mean_luma();
    if (restored - initial_luma).abs() > 2.0 || store.saves() != 0 {
        return Err(format!(
            "Cancel did not restore Pencil Lead: mean luma {initial_luma:.1} before, {restored:.1} after; {} saves",
            store.saves()
        ));
    }

    // Theme: the panels follow the previewed theme (the right list is mostly outside
    // the dialog); Light is much brighter than Dark.
    options::open(&mut app)?;
    options::page(&mut app, "Appearance")?;
    options::choose(&mut app, "Light theme", "Light")?;
    app.set_host_dark(false);
    app.until("the Light theme", |app| {
        app.window.get_theme_index() == LIGHT
    });
    app.idle(std::time::Duration::from_millis(300));
    let light_list = Frame::of(&mut app).luma(options::list_area(&app, PaneId::Right));
    options::choose(&mut app, "Dark theme", "Dark")?;
    app.set_host_dark(true);
    app.until("the Dark theme", |app| app.window.get_theme_index() == DARK);
    app.idle(std::time::Duration::from_millis(300));
    let dark_list = Frame::of(&mut app).luma(options::list_area(&app, PaneId::Right));
    if light_list < dark_list + 60.0 {
        return Err(format!(
            "the theme preview is not visible: panel luma {light_list:.1} in Light, {dark_list:.1} in Dark"
        ));
    }

    options::choose(&mut app, ICON_PICKER, "Origami Trail")?;
    let origami = app.window.get_current_app_icon();
    if origami == moon || origami == harvest {
        return Err("Origami Trail repeated a previous window icon".into());
    }
    options::choose(&mut app, ICON_PICKER, "Interleave")?;
    let interleave = app.window.get_current_app_icon();
    if interleave == moon || interleave == harvest || interleave == origami {
        return Err("Interleave repeated a previous window icon".into());
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
    if saved.app_icon.storage_name() != "interleave" {
        return Err(format!("OK saved application icon {:?}", saved.app_icon));
    }
    if saved.dark_theme != PreferenceTheme::Dark
        || saved.light_theme != PreferenceTheme::Light
        || saved.density != UiDensity::Touch
        || saved.font_family != "DejaVu Sans Mono"
        || saved.desktop_font_size != 16
        || saved.touch_font_size != 18
        || saved.glyph_fitting != GlyphFitting::Subpixel
    {
        return Err(format!(
            "OK saved themes {:?}/{:?}, density {:?}, font {:?}, sizes {}/{}, rendering {:?}",
            saved.dark_theme,
            saved.light_theme,
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
    if app.window.get_current_app_icon() != interleave {
        return Err("the restarted window did not restore the Interleave artwork".into());
    }
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
        ("Dark theme", "Dark"),
        ("Light theme", "Light"),
        (ICON_PICKER, "Interleave"),
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
    app.until("Pencil Lead after Reset to defaults", |app| {
        app.window.get_theme_index() == DEFAULT_DARK && app.window.get_density_index() == 0
    });
    for (label, wanted) in [
        ("Dark theme", "Pencil Lead"),
        ("Light theme", "Letterhead"),
        (ICON_PICKER, "Midnight Moon"),
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
    if app.window.get_current_app_icon() != moon {
        return Err("Reset to defaults did not preview the Midnight Moon artwork".into());
    }
    let before_cancel = store.saves();
    options::cancel(&mut app)?;
    if app.window.get_current_app_icon() != interleave || store.saves() != before_cancel {
        return Err(
            "cancelling Reset did not restore the saved Interleave icon without a write".into(),
        );
    }
    options::open(&mut app)?;
    options::press(&mut app, "Reset to defaults")?;
    if !options::ok(&mut app, &store)? {
        return Err("OK after Reset to defaults saved nothing".into());
    }
    let saved = store.current();
    if saved.app_icon.storage_name() != "moon" {
        return Err(format!(
            "Reset to defaults saved application icon {:?}",
            saved.app_icon
        ));
    }
    if saved.dark_theme != PreferenceTheme::DEFAULT_DARK
        || saved.light_theme != PreferenceTheme::DEFAULT_LIGHT
        || saved.density != UiDensity::Desktop
        || !saved.font_family.is_empty()
        || saved.desktop_font_size != 12
        || saved.touch_font_size != 15
        || saved.glyph_fitting != GlyphFitting::Fitted
    {
        return Err(format!(
            "Reset to defaults saved themes {:?}/{:?}, density {:?}, font {:?}, sizes {}/{}, rendering {:?}",
            saved.dark_theme,
            saved.light_theme,
            saved.density,
            saved.font_family,
            saved.desktop_font_size,
            saved.touch_font_size,
            saved.glyph_fitting
        ));
    }
    context.note("All four icon choices displayed distinct window artwork. OK saved Interleave, restart restored it, cancelling Reset kept it, and accepting Reset saved Midnight Moon.");
    context.note("Appearance: Cancel restored the previewed theme. The Light and Dark selections followed the host appearance. The 16 px desktop and 18 px touch fonts, touch spacing and font face changed the panels. OK saved them with smooth text rendering and a restart restored them. Reset selected Pencil Lead and Letterhead with desktop spacing, 12/15 px fonts and sharp text. Text rendering configures FemtoVG at startup and is verified here through persistence.");
    app.shutdown()
}
