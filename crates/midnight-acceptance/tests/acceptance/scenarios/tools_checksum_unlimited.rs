//! Checksums without size limits: a large tree and a deep chain are calculated in full,
//! saved, verified and selected.
//!
//! Registered here, not in the registry's list: this module owns its entries, so they
//! change without touching `scenarios/mod.rs`. Not written yet: it reports SKIP.
use crate::{Context, Scenario};
use std::time::Duration;

pub fn scenarios() -> Vec<Scenario> {
    vec![Scenario {
        name: "tools_checksum_unlimited",
        area: "checksums",
        summary: "Not written yet. Checksums without size limits: a large tree and a deep chain calculated, the manifest saved and verified, every row selectable.",
        run: unlimited,
        timeout: Duration::from_secs(5 * 60),
    }]
}

fn unlimited(_: &mut Context) -> Result<(), String> {
    Err("SKIP: not written yet".to_owned())
}
