//! The single integration-test binary of `midnight-platform-common`.
//!
//! Every other `tests/*.rs` file is an independent module of this crate. Linking
//! one executable instead of one per file removes per-binary link and process
//! start-up cost; libtest still runs all tests in parallel. Cargo `autotests` is
//! disabled for this package, so `every_test_file_is_a_module` fails when a new
//! file is not declared below.
// Each module keeps its own copy of the shared `support` files it includes, exactly
// as when every file was a separate test crate, so their types never mix.
#![allow(clippy::duplicate_mod)]

mod archive_association_preferences;
mod archive_bookmarks;
mod archive_custom_extension_preferences;
mod archive_preview_preferences;
mod checksum_preferences;
mod comparator_preferences;
mod disk_concurrency_preferences;
mod editor_preferences;
mod file_comparators;
mod filename_colors;
mod filter_expression_language;
mod filter_history;
mod glyph_fitting_preferences;
mod listing_classifier;
mod metadata_case;
mod metadata_case_unix;
mod metadata_case_windows;
mod network_bookmarks;
mod network_tab_preferences;
mod panel_filter;
mod panel_filter_expression;
mod panel_filter_preferences;
mod panel_mode_preferences;
mod panel_preferences;
mod panel_split_preferences;
mod quick_search_preferences;
mod tabs_preferences;
mod theme_preferences;
mod transfer_preferences;
mod user_commands_settings;
mod viewer_preferences;
mod window_geometry;

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
