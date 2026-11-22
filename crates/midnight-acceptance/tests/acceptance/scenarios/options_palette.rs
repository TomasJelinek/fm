//! The filename colour palette as rendered: sources, documents and text share one colour,
//! light themes darken it, and the old shipped palette migrates while a customised list
//! is kept.
//!
//! Registered here, not in the registry's list: this module owns its entries, so they
//! change without touching `scenarios/mod.rs`. Not written yet: it reports SKIP.
use crate::{Context, Scenario};
use std::time::Duration;

pub fn scenarios() -> Vec<Scenario> {
    vec![Scenario {
        name: "options_palette",
        area: "options",
        summary: "Not written yet. The filename palette as rendered: one colour for sources, documents and text, darker on light themes, the old palette migrated, a customised list kept.",
        run: palette,
        timeout: Duration::from_secs(5 * 60),
    }]
}

fn palette(_: &mut Context) -> Result<(), String> {
    Err("SKIP: not written yet".to_owned())
}
