//! Staging: extraction refuses escaping or linked entries, and the staged copy must
//! match its own BUILD-INFO.json.
use crate::support::{pack, platform, release_tree};
use midnight_update::archive::{StageError, extract, verify_staged};
use semver::Version;
use std::path::Path;

fn staged(
    tree: &[(String, Vec<u8>)],
    version: &str,
) -> (tempfile::TempDir, Result<(), StageError>) {
    let platform = platform();
    let parsed = Version::parse(version).unwrap();
    let folder = tempfile::tempdir().unwrap();
    let archive = folder.path().join(platform.archive_name(&parsed));
    std::fs::write(&archive, pack(&platform.archive_root(&parsed), tree)).unwrap();
    let result = extract(
        std::fs::File::open(&archive).unwrap(),
        &platform,
        &platform.archive_root(&parsed),
        &folder.path().join("staged"),
    );
    (folder, result)
}

#[test]
fn a_release_extracts_and_verifies() {
    let tree = release_tree("0.5.1", "me.exe", &[("viewers/v/plugin.dll", b"plugin")]);
    let (folder, result) = staged(&tree, "0.5.1");
    result.unwrap();
    let staged = folder.path().join("staged");
    assert_eq!(
        std::fs::read(staged.join("viewers/v/plugin.dll")).unwrap(),
        b"plugin"
    );
    let info = verify_staged(&staged, &Version::parse("0.5.1").unwrap(), &platform()).unwrap();
    assert_eq!(info.binary, "me.exe");
}

#[test]
fn the_renamed_executable_is_read_from_build_info() {
    let tree = release_tree("0.5.1", "midnight-explorer.exe", &[]);
    let (folder, result) = staged(&tree, "0.5.1");
    result.unwrap();
    let info = verify_staged(
        &folder.path().join("staged"),
        &Version::parse("0.5.1").unwrap(),
        &platform(),
    )
    .unwrap();
    assert_eq!(info.binary, "midnight-explorer.exe");
}

#[test]
fn build_info_of_another_version_is_refused() {
    let tree = release_tree("0.5.0", "me.exe", &[]);
    let (folder, result) = staged(&tree, "0.5.1");
    result.unwrap();
    let error = verify_staged(
        &folder.path().join("staged"),
        &Version::parse("0.5.1").unwrap(),
        &platform(),
    )
    .unwrap_err();
    assert!(matches!(error, StageError::WrongBuild { .. }), "{error:?}");
    assert!(error.to_string().contains("0.5.0"));
}

#[test]
fn a_changed_module_file_is_refused() {
    let mut tree = release_tree("0.5.1", "me.exe", &[("viewers/v/plugin.dll", b"plugin")]);
    for (path, content) in &mut tree {
        if path == "viewers/v/plugin.dll" {
            *content = b"plugix".to_vec();
        }
    }
    let (folder, result) = staged(&tree, "0.5.1");
    result.unwrap();
    assert_eq!(
        verify_staged(
            &folder.path().join("staged"),
            &Version::parse("0.5.1").unwrap(),
            &platform()
        )
        .unwrap_err(),
        StageError::FileMismatch("viewers/v/plugin.dll".into())
    );
}

#[test]
fn a_changed_executable_is_refused() {
    let mut tree = release_tree("0.5.1", "me.exe", &[]);
    tree[0].1 = b"another program".to_vec();
    let (folder, result) = staged(&tree, "0.5.1");
    result.unwrap();
    assert_eq!(
        verify_staged(
            &folder.path().join("staged"),
            &Version::parse("0.5.1").unwrap(),
            &platform()
        )
        .unwrap_err(),
        StageError::FileMismatch("me.exe".into())
    );
}

#[test]
fn entries_outside_the_release_folder_are_refused() {
    for path in [
        "../escape.txt",
        "a/../../escape.txt",
        "a\\b.txt",
        "a/c:d.txt",
    ] {
        let tree = vec![(path.to_owned(), b"x".to_vec())];
        let (folder, result) = staged(&tree, "0.5.1");
        assert!(
            matches!(
                result,
                Err(StageError::BadEntry(_)) | Err(StageError::Unreadable(_))
            ),
            "{path}: {result:?}"
        );
        assert!(!folder.path().join("escape.txt").exists());
    }
}

#[test]
fn an_archive_without_build_info_is_refused() {
    let tree = vec![("me.exe".to_owned(), b"x".to_vec())];
    let (folder, result) = staged(&tree, "0.5.1");
    result.unwrap();
    assert!(matches!(
        verify_staged(
            &folder.path().join("staged"),
            &Version::parse("0.5.1").unwrap(),
            &platform()
        ),
        Err(StageError::NoBuildInfo(_))
    ));
}

#[test]
fn a_damaged_archive_is_refused() {
    let platform = platform();
    let folder = tempfile::tempdir().unwrap();
    let archive = folder.path().join("a");
    std::fs::write(&archive, b"not an archive").unwrap();
    let result = extract(
        std::fs::File::open(&archive).unwrap(),
        &platform,
        "root",
        &folder.path().join("out"),
    );
    assert!(
        matches!(result, Err(StageError::Unreadable(_))),
        "{result:?}"
    );
    assert!(Path::new(&folder.path().join("out")).is_dir());
}
