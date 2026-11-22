//! The Links choice on special destinations: FAT, removable and network folders (named
//! by the gated `MIDNIGHT_ACCEPTANCE_*_DIR` variables) get their default, reason and
//! warning, and the first OK holds where links would be lost.
//!
//! Registered here, not in the registry's list: this module owns its entries, so they
//! change without touching `scenarios/mod.rs`. Not written yet: it reports SKIP.
use crate::{Context, Scenario};
use std::time::Duration;

pub fn scenarios() -> Vec<Scenario> {
    vec![Scenario {
        name: "files_links_volumes",
        area: "transfers",
        summary: "Not written yet. The Links choice on FAT, removable and network destinations: default, reason and warning lines, the first OK's hold.",
        run: links_volumes,
        timeout: Duration::from_secs(5 * 60),
    }]
}

fn links_volumes(_: &mut Context) -> Result<(), String> {
    Err("SKIP: not written yet".to_owned())
}
