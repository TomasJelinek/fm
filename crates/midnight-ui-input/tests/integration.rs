//! The single integration-test binary of `midnight-ui-input`.
//!
//! Cargo autotests are disabled; the guard below keeps every test file included.

mod placement;
mod post_window_event;

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
