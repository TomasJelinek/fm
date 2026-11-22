//! Office previews through the installed HTML viewer: DOCX, PPTX and XLSX render, a
//! password-protected document is refused, and the preview names the missing viewer.
//!
//! Registered here, not in the registry's list: this module owns its entries, so they
//! change without touching `scenarios/mod.rs`. Not written yet: it reports SKIP.
use crate::{Context, Scenario};
use std::time::Duration;

pub fn scenarios() -> Vec<Scenario> {
    vec![Scenario {
        name: "viewers_office",
        area: "viewers",
        summary: "Not written yet. Office previews: DOCX, PPTX and XLSX render and can be searched, a password-protected file is refused, a missing HTML viewer is named.",
        run: office,
        timeout: Duration::from_secs(5 * 60),
    }]
}

fn office(_: &mut Context) -> Result<(), String> {
    Err("SKIP: not written yet".to_owned())
}
