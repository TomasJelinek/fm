#![cfg(windows)]
use midnight_core::*;
use midnight_platform_windows::WindowsFileSystem;
use std::{
    ffi::OsStr,
    fs,
    time::{Duration, UNIX_EPOCH},
};

#[allow(
    clippy::permissions_set_readonly_false,
    reason = "Windows-only fixture cleanup changes a file attribute, not Unix permission bits"
)]
fn clear_fixture_readonly(path: &std::path::Path) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions).unwrap();
}

#[test]
fn combined_case_only_rename_and_time_keep_other_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("CASE.txt");
    fs::write(&path, b"unchanged contents").unwrap();
    let cancel = Cancellation::default();
    let item = WindowsFileSystem::new(None)
        .prepare(&path, &cancel)
        .unwrap();
    let before = fs::metadata(&path).unwrap().created().unwrap();
    let time = UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    let patch = MetadataPatch {
        modified: TimeEdit::Set(time),
        windows_mask: EDIT_HIDDEN,
        windows_value: EDIT_HIDDEN,
        ..Default::default()
    };
    let result = item.edit_metadata(&patch, Some(OsStr::new("case.txt")), &cancel);
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(result.changes.renamed && result.changes.modified && result.changes.attributes);
    assert_eq!(
        fs::read(&result.current_path).unwrap(),
        b"unchanged contents"
    );
    let actual = fs::metadata(&result.current_path).unwrap();
    assert_eq!(actual.created().unwrap(), before);
    assert_eq!(actual.modified().unwrap(), time);
    assert_eq!(
        fs::read_dir(dir.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name(),
        "case.txt"
    );
}

#[test]
fn collision_or_cancel_cannot_apply_later_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("SOURCE");
    let target = dir.path().join("target");
    fs::write(&source, b"source").unwrap();
    fs::write(&target, b"target").unwrap();
    let cancel = Cancellation::default();
    let item = WindowsFileSystem::new(None)
        .prepare(&source, &cancel)
        .unwrap();
    let before = fs::metadata(&source).unwrap().modified().unwrap();
    let patch = MetadataPatch {
        modified: TimeEdit::Set(UNIX_EPOCH + Duration::from_secs(1_000_000)),
        ..Default::default()
    };
    let collision = item.edit_metadata(&patch, Some(OsStr::new("target")), &cancel);
    assert_eq!(
        collision.error.unwrap().kind,
        std::io::ErrorKind::AlreadyExists
    );
    assert!(!collision.changes.any());
    assert_eq!(fs::metadata(&source).unwrap().modified().unwrap(), before);
    cancel.cancel();
    assert_eq!(
        item.edit_metadata(&patch, None, &cancel)
            .error
            .unwrap()
            .kind,
        std::io::ErrorKind::Interrupted
    );
    assert_eq!(fs::read(&target).unwrap(), b"target");
}

#[test]
fn another_hardlink_name_is_a_preview_collision() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("FIRST");
    let other = dir.path().join("other");
    fs::write(&path, b"shared contents").unwrap();
    fs::hard_link(&path, &other).unwrap();
    let cancel = Cancellation::default();
    let item = WindowsFileSystem::new(None)
        .prepare(&path, &cancel)
        .unwrap();
    let conflict = item.metadata_name_available(OsStr::new("other"), &cancel);
    assert_eq!(
        conflict.unwrap_err().kind,
        std::io::ErrorKind::AlreadyExists
    );
    let result = item.edit_metadata(
        &MetadataPatch::default(),
        Some(OsStr::new("other")),
        &cancel,
    );
    assert_eq!(
        result.error.unwrap().kind,
        std::io::ErrorKind::AlreadyExists
    );
    assert!(!result.changes.any());
    assert!(path.exists() && other.exists());
}

#[test]
fn metadata_only_preserves_raw_utf16_and_long_native_paths() {
    use std::os::windows::ffi::OsStringExt;
    let dir = tempfile::tempdir().unwrap();
    let mut parent = dir.path().to_owned();
    for index in 0..5 {
        parent.push(format!("{index}-{}", "x".repeat(60)));
        fs::create_dir(&parent).unwrap();
    }
    let raw = std::ffi::OsString::from_wide(&[0xd800, 0x2e, 0x54, 0x58, 0x54]);
    let path = parent.join(&raw);
    fs::write(&path, b"native raw name").unwrap();
    let cancel = Cancellation::default();
    let item = WindowsFileSystem::new(None)
        .prepare(&path, &cancel)
        .unwrap();
    let time = UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    let result = item.edit_metadata(
        &MetadataPatch {
            modified: TimeEdit::Set(time),
            ..Default::default()
        },
        None,
        &cancel,
    );
    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.current_path, path);
    assert_eq!(
        fs::read_dir(&parent)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name(),
        raw
    );
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), time);
}

#[test]
fn compression_round_trip_uses_native_capability_and_preserves_contents() {
    use std::os::windows::fs::MetadataExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("compressible.txt");
    let contents = vec![42u8; 131_072];
    fs::write(&path, &contents).unwrap();
    let cancel = Cancellation::default();
    let item = WindowsFileSystem::new(None)
        .prepare(&path, &cancel)
        .unwrap();
    let caps = item.metadata_snapshot(&cancel).unwrap().capabilities;
    let patch = MetadataPatch {
        compressed: Some(true),
        ..Default::default()
    };
    let result = item.edit_metadata(&patch, None, &cancel);
    if caps.compressed {
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_ne!(fs::metadata(&path).unwrap().file_attributes() & 0x800, 0);
        assert_eq!(fs::read(&path).unwrap(), contents);
        let result = item.edit_metadata(
            &MetadataPatch {
                compressed: Some(false),
                ..Default::default()
            },
            None,
            &cancel,
        );
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(fs::metadata(&path).unwrap().file_attributes() & 0x800, 0);
    } else {
        assert_eq!(result.error.unwrap().kind, std::io::ErrorKind::Unsupported);
    }
    assert_eq!(fs::read(&path).unwrap(), contents);
}

fn clear_readonly_with_compression(compressed: bool, name: Option<&OsStr>) {
    use std::os::windows::fs::MetadataExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("readonly.txt");
    let contents = vec![43u8; 131_072];
    fs::write(&path, &contents).unwrap();
    let cancel = Cancellation::default();
    let item = WindowsFileSystem::new(None)
        .prepare(&path, &cancel)
        .unwrap();
    if !item
        .metadata_snapshot(&cancel)
        .unwrap()
        .capabilities
        .compressed
    {
        return;
    }
    if !compressed {
        let initial = item.edit_metadata(
            &MetadataPatch {
                compressed: Some(true),
                ..Default::default()
            },
            None,
            &cancel,
        );
        assert!(initial.error.is_none(), "{:?}", initial.error);
    }
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();
    let patch = MetadataPatch {
        windows_mask: EDIT_READ_ONLY,
        windows_value: 0,
        compressed: Some(compressed),
        ..Default::default()
    };
    let result = item.edit_metadata(&patch, name, &cancel);
    let actual_path = &result.current_path;
    let attrs = fs::metadata(actual_path).unwrap().file_attributes();
    // Always restore the fresh fixture's cleanup permission, including on the RED path.
    clear_fixture_readonly(actual_path);
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(result.changes.attributes && result.changes.compressed);
    assert_eq!(result.changes.renamed, name.is_some());
    if let Some(name) = name {
        assert_eq!(actual_path.file_name().unwrap(), name);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    assert_eq!(attrs & EDIT_READ_ONLY, 0);
    assert_eq!(attrs & 0x800 != 0, compressed);
    assert_eq!(fs::read(actual_path).unwrap(), contents);
}

#[test]
fn clear_readonly_and_compress_in_one_patch() {
    clear_readonly_with_compression(true, None);
}

#[test]
fn clear_readonly_and_uncompress_in_one_patch() {
    clear_readonly_with_compression(false, None);
}

#[test]
fn clear_readonly_and_compress_after_rename() {
    clear_readonly_with_compression(true, Some(OsStr::new("RENAMED.txt")));
}

#[test]
fn compression_keep_readonly_does_not_change_attributes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("readonly.txt");
    fs::write(&path, b"source").unwrap();
    let cancel = Cancellation::default();
    let item = WindowsFileSystem::new(None)
        .prepare(&path, &cancel)
        .unwrap();
    if !item
        .metadata_snapshot(&cancel)
        .unwrap()
        .capabilities
        .compressed
    {
        return;
    }
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();
    let result = item.edit_metadata(
        &MetadataPatch {
            compressed: Some(true),
            ..Default::default()
        },
        None,
        &cancel,
    );
    let readonly = fs::metadata(&path).unwrap().permissions().readonly();
    clear_fixture_readonly(&path);
    assert_eq!(
        result.error.unwrap().kind,
        std::io::ErrorKind::PermissionDenied
    );
    assert!(!result.changes.any());
    assert!(readonly);
    assert_eq!(fs::read(&path).unwrap(), b"source");
}

#[test]
fn compression_failure_reports_applied_readonly_and_hidden_changes() {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_DELETE, FILE_SHARE_READ,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("readonly.txt");
    fs::write(&path, b"source").unwrap();
    let cancel = Cancellation::default();
    let item = WindowsFileSystem::new(None)
        .prepare(&path, &cancel)
        .unwrap();
    if !item
        .metadata_snapshot(&cancel)
        .unwrap()
        .capabilities
        .compressed
    {
        return;
    }
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();
    // Attribute writes do not require write sharing; the later compression data handle does.
    let guard = fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_READ_DATA)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
        .open(&path)
        .unwrap();
    let result = item.edit_metadata(
        &MetadataPatch {
            windows_mask: EDIT_READ_ONLY | EDIT_HIDDEN,
            windows_value: EDIT_HIDDEN,
            compressed: Some(true),
            ..Default::default()
        },
        None,
        &cancel,
    );
    let attrs = fs::metadata(&path).unwrap().file_attributes();
    drop(guard);
    clear_fixture_readonly(&path);
    assert_eq!(
        result
            .error
            .as_ref()
            .expect("compression needs write sharing")
            .operation,
        "open compression target"
    );
    assert!(result.changes.attributes);
    assert!(!(result.changes.compressed || result.changes.renamed || result.changes.modified));
    assert_eq!(attrs & (EDIT_READ_ONLY | EDIT_HIDDEN), EDIT_HIDDEN);
    assert_eq!(attrs & 0x800, 0);
    assert_eq!(fs::read(&path).unwrap(), b"source");
}

#[test]
fn compression_collision_keeps_requested_readonly_clear_unapplied() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("readonly.txt");
    let target = dir.path().join("target.txt");
    fs::write(&path, b"source").unwrap();
    fs::write(&target, b"target").unwrap();
    let cancel = Cancellation::default();
    let item = WindowsFileSystem::new(None)
        .prepare(&path, &cancel)
        .unwrap();
    if !item
        .metadata_snapshot(&cancel)
        .unwrap()
        .capabilities
        .compressed
    {
        return;
    }
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();
    let result = item.edit_metadata(
        &MetadataPatch {
            windows_mask: EDIT_READ_ONLY,
            windows_value: 0,
            compressed: Some(true),
            ..Default::default()
        },
        Some(OsStr::new("target.txt")),
        &cancel,
    );
    let readonly = fs::metadata(&path).unwrap().permissions().readonly();
    clear_fixture_readonly(&path);
    assert_eq!(
        result.error.unwrap().kind,
        std::io::ErrorKind::AlreadyExists
    );
    assert!(!result.changes.any());
    assert!(readonly);
    assert_eq!(fs::read(&path).unwrap(), b"source");
    assert_eq!(fs::read(&target).unwrap(), b"target");
}
