//! Panel scrolling: rows cut at the list's edges are drawn, Details and Brief come to
//! rest on whole rows after a scroll, Thumbnails keeps its offset, a click on a cut row
//! selects it, reduce-motion aligns at once.
//!
//! Registered here, not in the registry's list: this module owns its entries, so they
//! change without touching `scenarios/mod.rs`. Not written yet: it reports SKIP.
use crate::{Context, Scenario};
use std::time::Duration;

pub fn scenarios() -> Vec<Scenario> {
    vec![Scenario {
        name: "panels_scrolling",
        area: "panels",
        summary: "Not written yet. Panel scrolling: rows cut at the edges drawn, whole-row rest in Details and Brief, Thumbnails keeps its offset, a click on a cut row, reduce-motion.",
        run: scrolling,
        timeout: Duration::from_secs(5 * 60),
    }]
}

fn scrolling(_: &mut Context) -> Result<(), String> {
    Err("SKIP: not written yet".to_owned())
}
