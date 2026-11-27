//! Opt-in end-to-end acceptance suite for Midnight Explorer.
//!
//! This library is intentionally empty. The suite is the `acceptance` test target
//! (`tests/acceptance.rs`), which needs the `acceptance` feature and runs only through
//! `./build.sh --acceptance` or `scripts/acceptance.sh`. See
//! `docs/developer/acceptance.md` for how to run it, how to add a scenario and the
//! fixture catalogue.
#![forbid(unsafe_code)]
