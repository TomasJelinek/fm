//! The crate's integration tests, one binary. Every `tests/*.rs` file is a module here.
mod support;

mod apply;
#[cfg(windows)]
mod apply_windows;
mod archive;
mod client;
mod controller;
mod installer;
mod loopback;
mod release;
mod trust;

#[test]
fn every_test_file_is_a_module() {
    let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let source = std::fs::read_to_string(folder.join("integration.rs")).unwrap();
    for entry in std::fs::read_dir(&folder).unwrap() {
        let path = entry.unwrap().path();
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if path.extension().is_some_and(|extension| extension == "rs") && stem != "integration" {
            assert!(
                source.contains(&format!("mod {stem};")),
                "{stem}.rs is not declared"
            );
        }
    }
}
