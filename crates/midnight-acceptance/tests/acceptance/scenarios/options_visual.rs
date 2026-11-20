//! Every Options page in a light and a dark theme, and in a narrow touch layout where
//! a picker replaces the topic list, each capture run through the UI polish gate's
//! checks (`scenarios::ui_gate`); any flag fails the scenario.
use crate::{
    Context,
    app::{App, Config},
    options_support::{self as options, PAGES, SettingsStore},
    scenarios::ui_gate::{self as gate, Target},
};
use midnight_core::{PreferenceTheme, UiDensity};

pub fn visual(context: &mut Context) -> Result<(), String> {
    let folder = context.fixture.path("docs");
    let mut pages = 0;
    for (theme, density, width, height) in [
        (PreferenceTheme::Light, UiDensity::Desktop, 1340, 820),
        (PreferenceTheme::Dark, UiDensity::Desktop, 1340, 820),
        (PreferenceTheme::Dark, UiDensity::Touch, 720, 560),
    ] {
        let store = SettingsStore::new();
        let mut config = Config::new(&folder, &folder);
        config.theme = theme;
        config.density = density;
        config.width = width;
        config.height = height;
        config.settings = Some(store.clone());
        let mut app = App::start(config);
        let prefix = format!("{theme:?}-{density:?}-{width}x{height}");
        options::open(&mut app)?;
        for (title, _) in PAGES {
            options::page(&mut app, title)
                .map_err(|error| format!("{prefix}: {title}: {error}"))?;
            let name = format!("{prefix}-{}", title.to_lowercase().replace(' ', "-"));
            gate::capture(
                &mut app,
                context,
                &Target::Main,
                &format!("{prefix}/options/{title}"),
                &name,
                &format!("Options › {title} · {theme:?} · {density:?} · {width}×{height}"),
            );
            pages += 1;
        }
        options::cancel(&mut app)?;
        if store.saves() != 0 {
            return Err(format!("{prefix}: browsing the pages saved settings"));
        }
        app.shutdown()?;
    }
    context.note(format!(
        "{pages} Options pages captured (Light and Dark at 1340×820, Dark touch at 720×560 with the topic picker), each through the UI polish gate"
    ));
    gate::verdict(context, "options_visual")
}
