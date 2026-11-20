//! The single integration-test binary of `midnight-core`.
//!
//! Every other `tests/*.rs` file is an independent module of this crate. Linking
//! one executable instead of one per file removes per-binary link and process
//! start-up cost; libtest still runs all tests in parallel. Cargo `autotests` is
//! disabled for this package, so `every_test_file_is_a_module` fails when a new
//! file is not declared below.
// Each module keeps its own copy of the shared `support` files it includes, exactly
// as when every file was a separate test crate, so their types never mix.
#![allow(clippy::duplicate_mod)]

mod archive_associations;
mod archive_bookmark_identity;
mod archive_custom_extensions;
mod archive_custom_extensions_unix;
mod archive_custom_extensions_windows;
mod archive_timestamp;
mod cancellation;
mod checksum_shortcuts;
mod checksum_verification;
mod comparison_contexts;
mod file_clipboard;
mod file_comparators;
mod file_compare_shortcut;
mod file_drag_drop;
mod find;
mod icon_keys;
mod linked_cancellation;
mod listing_performance;
mod locale_sorting;
mod network_bookmark_identity;
mod panel_choice_shortcuts;
mod panel_selected_navigation;
mod preferences;
mod selection_ranges;
mod shell_menu;
mod shortcuts;
mod themes;
mod tree_view;
mod vfs_identity;
mod viewer_shortcuts;

#[test]
fn every_test_file_is_a_module() {
    use std::collections::BTreeSet;
    let declared: BTreeSet<String> = include_str!("integration.rs")
        .lines()
        .filter_map(|line| line.strip_prefix("mod ")?.strip_suffix(';'))
        .map(str::to_owned)
        .collect();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let files: BTreeSet<String> = std::fs::read_dir(directory)
        .expect("tests directory")
        .map(|entry| entry.expect("tests directory entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .filter_map(|path| Some(path.file_stem()?.to_str()?.to_owned()))
        .filter(|stem| stem != "integration")
        .collect();
    assert_eq!(files, declared, "declare every tests/*.rs file as a module");
}
