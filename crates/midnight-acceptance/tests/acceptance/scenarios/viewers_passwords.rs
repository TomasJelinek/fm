//! Password-protected archives through the installed plugins: the listing's encrypted
//! rows, F3 on an encrypted member, unlocking, the password's lifetime, and that the
//! password never leaks.
//!
//! Registered here, not in the registry's list: this module owns its entries, so they
//! change without touching `scenarios/mod.rs`. Not written yet: it reports SKIP.
use crate::{Context, Scenario};
use std::time::Duration;

pub fn scenarios() -> Vec<Scenario> {
    vec![Scenario {
        name: "viewers_passwords",
        area: "viewers",
        summary: "Not written yet. Password-protected ZIP and 7z archives: encrypted rows, F3 asks for the password, wrong and empty answers, unlocking, the password's lifetime, no leaks.",
        run: passwords,
        timeout: Duration::from_secs(5 * 60),
    }]
}

fn passwords(_: &mut Context) -> Result<(), String> {
    Err("SKIP: not written yet".to_owned())
}
