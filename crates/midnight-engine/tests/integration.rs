//! The single integration-test binary of `midnight-engine`.
//!
//! Every other `tests/*.rs` file is an independent module of this crate. Linking
//! one executable instead of one per file removes per-binary link and process
//! start-up cost; libtest still runs all tests in parallel. Cargo `autotests` is
//! disabled for this package, so `every_test_file_is_a_module` fails when a new
//! file is not declared below.
// Each module keeps its own copy of the shared `support` files it includes, exactly
// as when every file was a separate test crate, so their types never mix.
#![allow(clippy::duplicate_mod)]

mod archive_selection;
mod checksum_actor;
mod checksum_manifest_verification;
mod checksums_backend;
mod clipboard;
mod desktop;
mod editor;
mod elevation;
mod engine_configuration;
mod file_actions;
mod find_actions;
mod find_actor;
mod find_advanced;
mod find_archives;
mod find_backend;
mod find_content_streaming;
mod find_content_unicode;
mod find_current_directory;
mod find_duplicate_failures;
mod find_extension_sort;
mod find_real_windows;
mod find_sort;
mod find_sort_after_stop;
mod folder_size_warnings;
mod folder_sizes;
mod hidden_entries_unix;
mod locale_datetime;
mod locale_sorting;
mod navigation;
mod navigation_fallback;
mod navigation_not_a_folder;
mod navigation_ordering;
mod operations_real_windows;
mod search;
mod selected_folder_sizes;
mod tree_controller;
mod tree_controller_ack;
mod tree_filters;
mod tree_link_native_linux;
mod tree_link_native_windows;
mod tree_recursive_controller;
mod unicode;
mod user_command_actor;
mod user_command_contracts;
mod user_command_preparation;
mod user_command_process;
mod user_command_templates;
mod user_command_tree_preparation;
mod volumes;
mod watch_native;
mod windows_paths;

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
