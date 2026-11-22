//! Options' Export and Import of the configuration file: the exported file, the import
//! preview, Cancel and OK, refused files, and paths from the other operating system.
//!
//! Registered here, not in the registry's list: this module owns its entries, so they
//! change without touching `scenarios/mod.rs`. Not written yet: it reports SKIP.
use crate::{Context, Scenario};
use std::time::Duration;

pub fn scenarios() -> Vec<Scenario> {
    vec![Scenario {
        name: "options_settings_file",
        area: "options",
        summary: "Not written yet. Export and Import of the settings file: export and replace, the import preview, Cancel and OK, refused files, paths from the other OS.",
        run: settings_file,
        timeout: Duration::from_secs(5 * 60),
    }]
}

fn settings_file(_: &mut Context) -> Result<(), String> {
    Err("SKIP: not written yet".to_owned())
}
