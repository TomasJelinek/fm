#![cfg(target_os = "linux")]
mod support;
use midnight_filesystem_plugin_api::*;
use std::{
    ffi::OsString,
    fs,
    os::unix::{
        ffi::OsStringExt,
        fs::{MetadataExt, PermissionsExt, symlink},
    },
};
use support::*;

fn native_identity(path: &std::path::Path) -> (u64, u64) {
    let metadata = fs::symlink_metadata(path).unwrap();
    (metadata.dev(), metadata.ino())
}

fn prepare(client: &Client, path: &std::path::Path) -> PreparedRename {
    let (mount, node) = client.resolve(path);
    match client.call(Request::PrepareRename {
        mount,
        node: node.id,
    }) {
        Response::RenamePrepared(value) => value,
        other => panic!("native prepared Rename must succeed: {other:?}"),
    }
}
fn name(bytes: &[u8]) -> NativeText {
    NativeText {
        encoding: NameEncoding::UnixBytes,
        bytes: bytes.to_vec(),
    }
}
fn attempt(client: &Client, prepared: u64, destination: &[u8]) -> RenameOutcome {
    let Response::RenameState(state) = client.call(Request::Rename {
        prepared,
        attempt: 1,
        name: name(destination),
        policy: RenamePolicy::NoReplace,
    }) else {
        panic!("native Rename must return its retained state")
    };
    state.outcome
}
fn changed(outcome: RenameOutcome) {
    assert!(
        matches!(
            outcome,
            RenameOutcome::Unchanged(Failure {
                code: ErrorCode::SourceChanged,
                ..
            })
        ),
        "{outcome:?}"
    );
}

/// Bypass the public encoder's validation to exercise the actual ABI decoder.
#[allow(unsafe_code)]
fn reject_raw_rename_basename(client: &Client, prepared: u64, basename: &[u8]) {
    use midnight_filesystem_plugin_api::abi::*;

    // FSBR v1 + prepared:u64 + attempt:u64 + name encoding:u8 + byte count:u64
    // + at most eight name bytes + NoReplace:u8.
    assert!(basename.len() <= 8);
    let mut bytes = [0u8; 42];
    bytes[..8].copy_from_slice(b"FSBR\x01\0\0\0");
    bytes[8..16].copy_from_slice(&prepared.to_le_bytes());
    bytes[16..24].copy_from_slice(&1u64.to_le_bytes());
    bytes[24] = NameEncoding::UnixBytes as u8;
    bytes[25..33].copy_from_slice(&(basename.len() as u64).to_le_bytes());
    bytes[33..33 + basename.len()].copy_from_slice(basename);
    let length = 34 + basename.len();
    let call = Call {
        struct_size: size_of::<Call>() as u32,
        operation: 21,
        payload: Slice {
            data: bytes.as_ptr(),
            len: length as u64,
        },
        cancel: Cancellation::default(),
    };
    let mut output = Buffer::default();
    // SAFETY: the live client owns this instance and callback table; bounded
    // caller-owned payload/output storage remains mapped for the synchronous call.
    let status = unsafe { client.api.call.unwrap()(client.instance, &call, &mut output) };
    let output_id = output.id;
    let output_length = output.bytes.len;
    let output_is_null = output.bytes.data.is_null();
    if output_id != 0 {
        // SAFETY: release any unexpectedly accepted response using its producing
        // descriptor/instance before asserting, so a failure cannot leak a buffer.
        assert_eq!(
            unsafe { client.api.release_buffer.unwrap()(client.instance, output_id) },
            OK
        );
    }
    assert_eq!(status, INVALID);
    assert_eq!(output_id, 0);
    assert_eq!(output_length, 0);
    assert!(output_is_null);
}

#[test]
fn prepare_close_and_cancel_leave_namespace_untouched() {
    let temp = Temp::new();
    let path = temp.path.join("selected");
    fs::write(&path, b"selected bytes").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o0)).unwrap();
    let before = fs::symlink_metadata(&path).unwrap();
    let client = Client::new();
    let prepared = prepare(&client, &path);
    assert_eq!(prepared.source.name, name(b"selected"));
    assert_eq!(prepared.source.kind, Kind::File);
    assert_eq!(prepared.source.metadata.size, Some(14));
    assert!(!prepared.source.identity.is_empty());
    assert!(!prepared.parent_identity.is_empty());
    assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 1);
    assert_eq!(
        client.cancelled(
            Request::CloseRename {
                prepared: prepared.handle
            },
            1
        ),
        Response::Unit
    );
    assert_eq!(fs::symlink_metadata(&path).unwrap().ino(), before.ino());
    let prepared = prepare(&client, &path);
    assert!(matches!(
        client.cancelled(
            Request::Rename {
                prepared: prepared.handle,
                attempt: 1,
                name: name(b"renamed"),
                policy: RenamePolicy::NoReplace,
            },
            1
        ),
        Response::Failed(Failure {
            code: ErrorCode::Cancelled,
            ..
        })
    ));
    assert_eq!(
        client.cancelled(
            Request::GetRenameState {
                prepared: prepared.handle
            },
            1
        ),
        Response::RenameState(RenameState {
            prepared: prepared.handle,
            attempt: 0,
            outcome: RenameOutcome::Ready,
        })
    );
    assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 1);
    assert_eq!(fs::symlink_metadata(&path).unwrap().ino(), before.ino());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn same_kind_replacement_before_prepare_is_current_prepared_object() {
    let temp = Temp::new();
    let path = temp.path.join("selected");
    fs::write(&path, b"old").unwrap();
    let client = Client::new();
    let (mount, node) = client.resolve(&path);
    fs::rename(&path, temp.path.join("old")).unwrap();
    fs::write(&path, b"current object").unwrap();
    let Response::RenamePrepared(prepared) = client.call(Request::PrepareRename {
        mount,
        node: node.id,
    }) else {
        panic!("Browse is a logical locator; Prepare must pin the current same-kind object")
    };
    assert_eq!(prepared.source.metadata.size, Some(14));
    assert!(matches!(
        attempt(&client, prepared.handle, b"renamed"),
        RenameOutcome::Renamed {
            same_entry: false,
            ..
        }
    ));
    assert_eq!(
        fs::read(temp.path.join("renamed")).unwrap(),
        b"current object"
    );
    assert_eq!(fs::read(temp.path.join("old")).unwrap(), b"old");
}

#[test]
fn source_replacement_after_prepare_is_refused_without_adoption() {
    let temp = Temp::new();
    let path = temp.path.join("selected");
    fs::write(&path, b"original").unwrap();
    let client = Client::new();
    let prepared = prepare(&client, &path);
    fs::rename(&path, temp.path.join("original")).unwrap();
    fs::write(&path, b"substitute").unwrap();
    changed(attempt(&client, prepared.handle, b"renamed"));
    assert_eq!(fs::read(&path).unwrap(), b"substitute");
    assert_eq!(fs::read(temp.path.join("original")).unwrap(), b"original");
    assert!(!temp.path.join("renamed").exists());
    assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 2);
}

#[test]
fn actual_parent_replacement_refuses_even_the_original_inode_and_same_basename() {
    let temp = Temp::new();
    let parent = temp.path.join("parent");
    fs::create_dir(&parent).unwrap();
    let source = parent.join("selected");
    fs::write(&source, b"original").unwrap();
    let source_id = native_identity(&source);
    let parent_id = native_identity(&parent);
    let client = Client::new();
    // Separate leases test the normal and same-name paths without terminal reuse.
    let renamed = prepare(&client, &source);
    let same_name = prepare(&client, &source);
    let old_parent = temp.path.join("old-parent");
    fs::rename(&parent, &old_parent).unwrap();
    fs::create_dir(&parent).unwrap();
    // Keep the selected inode and basename intact; only the parent identity changes.
    fs::rename(old_parent.join("selected"), &source).unwrap();
    assert_eq!(native_identity(&source), source_id);
    assert_ne!(native_identity(&parent), parent_id);
    changed(attempt(&client, renamed.handle, b"renamed"));
    changed(attempt(&client, same_name.handle, b"selected"));
    assert_eq!(native_identity(&source), source_id);
    assert_eq!(fs::read(&source).unwrap(), b"original");
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
    assert_eq!(fs::read_dir(&old_parent).unwrap().count(), 0);
    assert!(!parent.join("renamed").exists());
    assert!(!old_parent.join("renamed").exists());
}

#[test]
fn successful_no_replace_preserves_regular_file_device_and_inode() {
    let temp = Temp::new();
    let source = temp.path.join("source");
    let destination = temp.path.join("renamed");
    fs::write(&source, b"retained inode").unwrap();
    let source_id = native_identity(&source);
    let client = Client::new();
    let prepared = prepare(&client, &source);
    assert!(matches!(
        attempt(&client, prepared.handle, b"renamed"),
        RenameOutcome::Renamed { same_entry: false, recovery, .. } if recovery.is_empty()
    ));
    assert!(!source.exists());
    assert_eq!(native_identity(&destination), source_id);
    assert_eq!(fs::read(&destination).unwrap(), b"retained inode");
    assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 1);
}

#[test]
fn same_name_validates_binding_and_case_only_names_use_linux_semantics() {
    let temp = Temp::new();
    let path = temp.path.join("Selected");
    fs::write(&path, b"source").unwrap();
    let client = Client::new();
    let prepared = prepare(&client, &path);
    assert!(matches!(attempt(&client, prepared.handle, b"Selected"),
        RenameOutcome::Renamed { same_entry: true, recovery, .. } if recovery.is_empty()));
    let prepared = prepare(&client, &path);
    assert!(matches!(attempt(&client, prepared.handle, b"selected"),
        RenameOutcome::Renamed { same_entry: false, recovery, .. } if recovery.is_empty()));
    assert!(!path.exists());
    assert_eq!(fs::read(temp.path.join("selected")).unwrap(), b"source");
}

#[test]
fn raw_unix_name_and_backslash_are_preserved_without_utf8_conversion() {
    let temp = Temp::new();
    let source = OsString::from_vec(vec![b's', 0xff, b'\\']);
    let destination = vec![b'd', 0xfe, b'\\'];
    let path = temp.path.join(&source);
    fs::write(&path, b"raw bytes").unwrap();
    let client = Client::new();
    let prepared = prepare(&client, &path);
    assert_eq!(prepared.source.name, name(&[b's', 0xff, b'\\']));
    assert!(matches!(attempt(&client, prepared.handle, &destination),
        RenameOutcome::Renamed { name: actual, .. } if actual == name(&destination)));
    assert_eq!(
        fs::read(temp.path.join(OsString::from_vec(destination))).unwrap(),
        b"raw bytes"
    );
}

#[test]
fn directory_and_selected_symlink_rename_do_not_touch_descendants_or_referent() {
    let temp = Temp::new();
    let directory = temp.path.join("directory");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("child"), b"descendant").unwrap();
    let referent = temp.path.join("referent");
    fs::write(&referent, b"external sentinel").unwrap();
    let link = temp.path.join("link");
    symlink(&referent, &link).unwrap();
    let link_id = fs::symlink_metadata(&link).unwrap().ino();
    let client = Client::new();
    let prepared = prepare(&client, &directory);
    assert_eq!(prepared.source.kind, Kind::Directory);
    assert!(matches!(
        attempt(&client, prepared.handle, b"moved-directory"),
        RenameOutcome::Renamed { .. }
    ));
    assert_eq!(
        fs::read(temp.path.join("moved-directory/child")).unwrap(),
        b"descendant"
    );
    let prepared = prepare(&client, &link);
    assert_eq!(prepared.source.kind, Kind::Symlink);
    assert!(matches!(
        attempt(&client, prepared.handle, b"moved-link"),
        RenameOutcome::Renamed { .. }
    ));
    assert_eq!(
        fs::symlink_metadata(temp.path.join("moved-link"))
            .unwrap()
            .ino(),
        link_id
    );
    assert_eq!(
        fs::read_link(temp.path.join("moved-link")).unwrap(),
        referent
    );
    assert_eq!(fs::read(&referent).unwrap(), b"external sentinel");
}

#[test]
fn invalid_basename_does_not_execute_or_consume_prepared_resource() {
    let temp = Temp::new();
    let path = temp.path.join("selected");
    fs::write(&path, b"source").unwrap();
    let client = Client::new();
    let prepared = prepare(&client, &path);
    for invalid in [b"".as_slice(), b".", b"..", b"a/b", b"a\0b"] {
        reject_raw_rename_basename(&client, prepared.handle, invalid);
        assert_eq!(
            client.cancelled(
                Request::GetRenameState {
                    prepared: prepared.handle
                },
                1
            ),
            Response::RenameState(RenameState {
                prepared: prepared.handle,
                attempt: 0,
                outcome: RenameOutcome::Ready,
            })
        );
    }
    assert_eq!(fs::read(&path).unwrap(), b"source");
    assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 1);
    assert!(matches!(
        attempt(&client, prepared.handle, b"valid"),
        RenameOutcome::Renamed { .. }
    ));
}
