#![cfg(windows)]
#![forbid(unsafe_code)]
//! The directory junction fixture that the engine's Windows tests share takes both paths as
//! data: SECURITY.md requires that no shell or command line ever sees a fixture path, where
//! `%NAME%` would be expanded and `&` or `^` would end or escape a command.
use std::fs;

#[path = "support/checksum_verify_links_windows.rs"]
mod links;

#[test]
fn junction_fixture_paths_are_data_never_command_text() {
    let owned = tempfile::tempdir().unwrap();
    let target = owned.path().join("target 100% %PATH% ^& !OS! (x)");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("inside.txt"), b"reached").unwrap();
    let link = owned.path().join("link %OS% & echo ^ ! 'x'");
    links::create(&link, &target);
    // The junction names exactly the target: nothing was expanded or run instead.
    assert_eq!(fs::read(link.join("inside.txt")).unwrap(), b"reached");
    assert_eq!(
        fs::canonicalize(&link).unwrap(),
        fs::canonicalize(&target).unwrap()
    );
    let mut names: Vec<_> = fs::read_dir(owned.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    let mut expected = [link.file_name().unwrap(), target.file_name().unwrap()];
    expected.sort();
    assert_eq!(names, expected);
    // A target that does not exist yet is linked as named, as `mklink /J` links it.
    let missing = owned.path().join("missing 50%");
    let dangling = owned.path().join("dangling ^");
    links::create(&dangling, &missing);
    fs::create_dir(&missing).unwrap();
    assert_eq!(
        fs::canonicalize(&dangling).unwrap(),
        fs::canonicalize(&missing).unwrap()
    );
}
