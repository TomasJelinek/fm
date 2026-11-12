use super::*;
use crate::providers::Manifest;
use std::{fs, path::PathBuf};

#[test]
fn catalog_metadata_preserves_declared_capabilities_and_readable_names() {
    for (id, name, extensions, video) in [
        (
            "midnight-image",
            "Image thumbnailer",
            vec!["png", "jpg"],
            false,
        ),
        (
            "midnight-video",
            "Video thumbnailer",
            vec!["webm", "avi"],
            true,
        ),
        ("custom.provider", "custom.provider", vec!["custom"], false),
    ] {
        let location = PathBuf::from("thumbnailers").join(id);
        let row = metadata(Provider {
            manifest: Manifest {
                protocol: 1,
                id: id.into(),
                version: "decoder-v2".into(),
                executable: id.into(),
                extensions: extensions
                    .iter()
                    .map(|extension| (*extension).into())
                    .collect(),
                video,
            },
            executable: location.clone(),
        });
        assert_eq!(row.id, id);
        assert_eq!(row.name, name);
        assert_eq!(row.version, "decoder-v2");
        assert_eq!(row.extensions, extensions);
        assert_eq!(row.video, video);
        assert_eq!(row.location, location);
        assert_eq!(row.status, "Installed");
    }
}

#[test]
fn absent_thumbnailer_directory_is_an_empty_catalog() {
    let root = tempfile::tempdir().unwrap();
    assert!(catalog(&root.path().join("me.exe")).unwrap().is_empty());
}

fn fixture() -> tempfile::TempDir {
    let base = std::env::var_os("HOST_TEST_INSTALL_ROOT").expect("HOST_TEST_INSTALL_ROOT");
    let root = tempfile::tempdir_in(base).unwrap();
    fs::write(
        root.path().join("me.exe"),
        b"catalog fixture, never executed",
    )
    .unwrap();
    fs::create_dir(root.path().join("thumbnailers")).unwrap();
    root
}

fn provider(root: &Path, package: &str, id: &str, extensions: &[&str], video: bool) -> PathBuf {
    let directory = root.join("thumbnailers");
    let executable = format!("{package}.exe");
    let helper = directory.join(&executable);
    fs::write(&helper, b"not executable, never started").unwrap();
    fs::write(
        directory.join(format!("{package}.json")),
        serde_json::to_vec(&serde_json::json!({
            "protocol": 1,
            "id": id,
            "version": "test-v1",
            "executable": executable,
            "extensions": extensions,
            "video": video,
        }))
        .unwrap(),
    )
    .unwrap();
    helper
}

#[test]
#[ignore = "requires a trusted HOST_TEST_INSTALL_ROOT"]
fn trusted_thumbnailer_catalog_lists_both_workers_without_starting_them() {
    let root = fixture();
    let video = provider(
        root.path(),
        "video",
        "midnight-video",
        &["webm", "avi"],
        true,
    );
    let image = provider(
        root.path(),
        "image",
        "midnight-image",
        &["png", "jpg"],
        false,
    );
    let rows = catalog(&root.path().join("me.exe")).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, "midnight-image");
    assert_eq!(rows[1].id, "midnight-video");
    for (row, helper) in rows.iter().zip([image, video]) {
        assert_eq!(row.version, "test-v1");
        assert_eq!(row.status, "Installed");
        assert_eq!(row.location, helper.canonicalize().unwrap());
        assert_eq!(fs::read(helper).unwrap(), b"not executable, never started");
    }
    assert_eq!(rows[0].extensions, ["png", "jpg"]);
    assert!(!rows[0].video);
    assert_eq!(rows[1].extensions, ["webm", "avi"]);
    assert!(rows[1].video);
}

#[test]
#[ignore = "requires a trusted HOST_TEST_INSTALL_ROOT"]
fn malformed_thumbnailer_manifest_returns_an_error_instead_of_a_partial_catalog() {
    let root = fixture();
    provider(root.path(), "image", "midnight-image", &["png"], false);
    fs::write(root.path().join("thumbnailers/broken.json"), b"not JSON").unwrap();
    assert!(catalog(&root.path().join("me.exe")).is_err());
}

#[test]
#[ignore = "requires a trusted HOST_TEST_INSTALL_ROOT"]
fn missing_thumbnailer_helper_returns_an_error() {
    let root = fixture();
    let helper = provider(root.path(), "image", "midnight-image", &["png"], false);
    fs::remove_file(helper).unwrap();
    assert!(catalog(&root.path().join("me.exe")).is_err());
}

#[test]
#[ignore = "requires a trusted HOST_TEST_INSTALL_ROOT"]
fn duplicate_thumbnailer_identity_returns_an_explicit_error() {
    let root = fixture();
    provider(root.path(), "one", "duplicate", &["png"], false);
    provider(root.path(), "two", "duplicate", &["jpg"], false);
    let error = catalog(&root.path().join("me.exe")).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Duplicate thumbnail provider identifier")
    );
}

#[test]
#[ignore = "requires PLUGIN_CATALOG_TEST_EXECUTABLE in a trusted installed package"]
fn actual_thumbnailer_installation_lists_image_and_video_helpers() {
    let executable = PathBuf::from(
        std::env::var_os("PLUGIN_CATALOG_TEST_EXECUTABLE").expect("PLUGIN_CATALOG_TEST_EXECUTABLE"),
    )
    .canonicalize()
    .unwrap();
    let rows = catalog(&executable).unwrap();
    for (id, video) in [("midnight-image", false), ("midnight-video", true)] {
        let row = rows.iter().find(|row| row.id == id).unwrap();
        assert_eq!(row.video, video);
        assert_eq!(row.status, "Installed");
        assert!(!row.extensions.is_empty());
        assert!(row.location.is_absolute());
    }
}
