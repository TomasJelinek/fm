//! The single integration-test binary of `midnight-platform`.
//!
//! Every other `tests/*.rs` file is an independent module of this crate. Linking
//! one executable instead of one per file removes per-binary link and process
//! start-up cost; libtest still runs all tests in parallel. Cargo `autotests` is
//! disabled for this package, so `every_test_file_is_a_module` fails when a new
//! file is not declared below.
// Each module keeps its own copy of the shared `support` files it includes, exactly
// as when every file was a separate test crate, so their types never mix.
#![allow(clippy::duplicate_mod)]

mod linux;
mod local_filesystem;
mod services;
mod unicode;
mod windows;

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

/// Cargo.toml sets `[lib] test = false`, so `cargo test` builds no unit-test harness for
/// the library and a unit test added to it would never run.
#[test]
fn library_has_no_unit_tests() {
    fn check(directory: &std::path::Path) {
        for entry in std::fs::read_dir(directory).expect("source directory") {
            let path = entry.expect("source directory entry").path();
            if path.is_dir() {
                check(&path);
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("library source");
            for line in source.lines().map(str::trim_start) {
                let attribute = line.starts_with("#[") || line.starts_with("#![");
                let names_test = line
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|word| word == "test");
                assert!(
                    !(attribute && names_test),
                    "{}: `{line}`; remove `test = false` from [lib] in Cargo.toml",
                    path.display()
                );
            }
        }
    }
    check(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"));
}
