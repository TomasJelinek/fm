#![cfg(windows)]
mod support;

use midnight_filesystem_plugin_api::*;
use std::{
    ffi::{OsStr, OsString},
    fs::{self, OpenOptions},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::Path,
};
use support::{Client, Temp};

#[test]
fn prepare_close_preserves_namespace_and_pins_current_object() {
    let temp = Temp::new();
    let source = temp.path.join("selected.bin");
    let retained = temp.path.join("retained-original.bin");
    fs::write(&source, b"listed object").unwrap();
    fs::write(temp.path.join("sibling.bin"), b"untouched sibling").unwrap();
    fs::create_dir(temp.path.join("sibling-directory")).unwrap();
    let client = Client::new();
    let (mount, node) = client.resolve(&source);
    let original_namespace = names(&temp.path);

    let original = prepare_node(&client, mount, node.id);
    assert_eq!(original.source.name, name("selected.bin"));
    assert_eq!(original.source.kind, Kind::File);
    assert_eq!(original.source.metadata.size, Some(13));
    assert!(!original.source.identity.is_empty());
    assert!(!original.source.revision.is_empty());
    assert!(!original.parent_identity.is_empty());
    assert_eq!(
        query(&client, original.handle).outcome,
        RenameOutcome::Ready
    );
    assert_eq!(names(&temp.path), original_namespace);
    close(&client, original.handle);
    assert_eq!(names(&temp.path), original_namespace);
    assert_eq!(fs::read(&source).unwrap(), b"listed object");

    // A Browse node is a logical locator. Its current same-kind replacement is
    // authoritative only when this later preparation pins that object.
    fs::rename(&source, &retained).unwrap();
    fs::write(&source, b"current replacement object").unwrap();
    let current_namespace = names(&temp.path);
    let current = prepare_node(&client, mount, node.id);
    assert_ne!(current.handle, original.handle);
    assert_ne!(current.source.identity, original.source.identity);
    assert_eq!(current.parent_identity, original.parent_identity);
    assert_eq!(current.source.name, name("selected.bin"));
    assert_eq!(current.source.metadata.size, Some(26));
    assert_eq!(names(&temp.path), current_namespace);
    close(&client, current.handle);
    assert_eq!(names(&temp.path), current_namespace);
    assert_eq!(fs::read(&source).unwrap(), b"current replacement object");
    assert_eq!(fs::read(&retained).unwrap(), b"listed object");
    assert_eq!(
        fs::read(temp.path.join("sibling.bin")).unwrap(),
        b"untouched sibling"
    );
}

#[test]
fn cancelled_preparation_and_attempt_leave_namespace_unchanged() {
    let temp = Temp::new();
    let source = temp.path.join("source.bin");
    let destination = temp.path.join("renamed.bin");
    fs::write(&source, b"cancel must preserve me").unwrap();
    let client = Client::new();
    let (mount, node) = client.resolve(&source);
    let before = names(&temp.path);
    assert_failed(
        client.cancelled(
            Request::PrepareRename {
                mount,
                node: node.id,
            },
            1,
        ),
        ErrorCode::Cancelled,
    );
    assert_eq!(names(&temp.path), before);

    let prepared = prepare_node(&client, mount, node.id);
    let ready = query(&client, prepared.handle);
    assert_eq!(ready.attempt, 0);
    assert_eq!(ready.outcome, RenameOutcome::Ready);
    let response = client.cancelled(
        Request::Rename {
            prepared: prepared.handle,
            attempt: 1,
            name: name("renamed.bin"),
            policy: RenamePolicy::NoReplace,
        },
        1,
    );
    assert_failed(response, ErrorCode::Cancelled);
    assert_eq!(query(&client, prepared.handle), ready);
    assert_eq!(names(&temp.path), before);
    assert_eq!(fs::read(&source).unwrap(), b"cancel must preserve me");
    assert!(!destination.exists());
    close(&client, prepared.handle);
    assert_eq!(names(&temp.path), before);
    assert_failed(
        client.call(Request::GetRenameState {
            prepared: prepared.handle,
        }),
        ErrorCode::StaleReference,
    );
}

#[test]
fn source_replacement_after_prepare_refuses_substitution() {
    let temp = Temp::new();
    let source = temp.path.join("source.bin");
    let retained = temp.path.join("retained-source.bin");
    fs::write(&source, b"pinned source").unwrap();
    let client = Client::new();
    let prepared = prepare(&client, &source);
    fs::rename(&source, &retained).unwrap();
    fs::write(&source, b"replacement must stay here").unwrap();
    let before = names(&temp.path);

    let state = rename(&client, prepared.handle, OsStr::new("destination.bin"));
    assert_unchanged(&state, ErrorCode::SourceChanged);
    assert_eq!(query(&client, prepared.handle), state);
    assert_eq!(names(&temp.path), before);
    assert_eq!(fs::read(&source).unwrap(), b"replacement must stay here");
    assert_eq!(fs::read(&retained).unwrap(), b"pinned source");
    assert!(!temp.path.join("destination.bin").exists());
    close(&client, prepared.handle);
    assert_eq!(names(&temp.path), before);
}

#[test]
fn parent_replacement_after_prepare_refuses_substitution() {
    let temp = Temp::new();
    let parent = temp.path.join("parent");
    let retained_parent = temp.path.join("retained-parent");
    fs::create_dir(&retained_parent).unwrap();
    junction(&parent, &retained_parent);
    let source = parent.join("source.bin");
    fs::write(&source, b"pinned parent and source").unwrap();
    let parent_identity = native_identity(&retained_parent);
    let client = Client::new();
    let prepared = prepare(&client, &source);
    // Windows refuses to rename a directory containing pinned children. Replace
    // only its junction alias while the actual parent and source pins stay open.
    fs::remove_dir(&parent).unwrap();
    fs::create_dir(&parent).unwrap();
    fs::write(&source, b"new parent source").unwrap();
    assert_ne!(native_identity(&parent), parent_identity);
    assert_eq!(native_identity(&retained_parent), parent_identity);
    let root_before = names(&temp.path);
    let parent_before = names(&parent);
    let retained_before = names(&retained_parent);

    let state = rename(&client, prepared.handle, OsStr::new("destination.bin"));
    assert_unchanged(&state, ErrorCode::SourceChanged);
    assert_eq!(query(&client, prepared.handle), state);
    assert_eq!(names(&temp.path), root_before);
    assert_eq!(names(&parent), parent_before);
    assert_eq!(names(&retained_parent), retained_before);
    assert_eq!(fs::read(&source).unwrap(), b"new parent source");
    assert_eq!(
        fs::read(retained_parent.join("source.bin")).unwrap(),
        b"pinned parent and source"
    );
    assert!(!parent.join("destination.bin").exists());
    assert!(!retained_parent.join("destination.bin").exists());
    close(&client, prepared.handle);
}

#[test]
fn replacing_only_parent_refuses_new_name_and_original_name() {
    let temp = Temp::new();
    let parent = temp.path.join("parent");
    let retained_parent = temp.path.join("retained-parent");
    fs::create_dir(&retained_parent).unwrap();
    junction(&parent, &retained_parent);
    let source = parent.join("source.bin");
    fs::write(&source, b"original source object").unwrap();
    fs::write(
        parent.join("parent-sentinel.bin"),
        b"original parent object",
    )
    .unwrap();
    let source_identity = native_identity(&source);
    // The no-follow identity helper would identify the alias, not its referent.
    let parent_identity = native_identity(&retained_parent);
    let client = Client::new();
    let new_name_prepared = prepare(&client, &source);
    let original_name_prepared = prepare(&client, &source);
    assert_eq!(
        new_name_prepared.source.identity,
        original_name_prepared.source.identity
    );
    assert_eq!(
        new_name_prepared.parent_identity,
        original_name_prepared.parent_identity
    );

    // Swap the alias while both preparations retain their real parent/source pins.
    fs::remove_dir(&parent).unwrap();
    fs::create_dir(&parent).unwrap();
    // Move the original object back to the same full path. A source-ID-only
    // check passes; the retained parent identity must still reject this binding.
    fs::rename(retained_parent.join("source.bin"), &source).unwrap();
    assert_eq!(native_identity(&source), source_identity);
    assert_ne!(native_identity(&parent), parent_identity);
    assert_eq!(native_identity(&retained_parent), parent_identity);
    let root_before = names(&temp.path);
    let parent_before = names(&parent);
    let retained_before = names(&retained_parent);

    let new_name = rename(
        &client,
        new_name_prepared.handle,
        OsStr::new("destination.bin"),
    );
    assert_unchanged(&new_name, ErrorCode::SourceChanged);
    assert_eq!(query(&client, new_name_prepared.handle), new_name);
    // An exact-name shortcut must validate the original parent binding too.
    let original_name = rename(
        &client,
        original_name_prepared.handle,
        OsStr::new("source.bin"),
    );
    assert_unchanged(&original_name, ErrorCode::SourceChanged);
    assert_eq!(query(&client, original_name_prepared.handle), original_name);
    assert_eq!(names(&temp.path), root_before);
    assert_eq!(names(&parent), parent_before);
    assert_eq!(names(&retained_parent), retained_before);
    assert_eq!(native_identity(&source), source_identity);
    assert_eq!(fs::read(&source).unwrap(), b"original source object");
    assert_eq!(
        fs::read(retained_parent.join("parent-sentinel.bin")).unwrap(),
        b"original parent object"
    );
    assert!(!parent.join("destination.bin").exists());
    assert!(!retained_parent.join("destination.bin").exists());
    close(&client, new_name_prepared.handle);
    close(&client, original_name_prepared.handle);
    assert_eq!(names(&parent), parent_before);
    assert_eq!(names(&retained_parent), retained_before);
}

#[test]
fn changing_an_unrelated_sibling_does_not_replace_parent_identity() {
    let temp = Temp::new();
    let source = temp.path.join("source.bin");
    fs::write(&source, b"selected").unwrap();
    let client = Client::new();
    let prepared = prepare(&client, &source);
    fs::write(temp.path.join("new-sibling.bin"), b"unrelated").unwrap();

    let state = rename(&client, prepared.handle, OsStr::new("destination.bin"));
    assert_renamed(&state, OsStr::new("destination.bin"));
    assert!(!source.exists());
    assert_eq!(
        fs::read(temp.path.join("destination.bin")).unwrap(),
        b"selected"
    );
    assert_eq!(
        fs::read(temp.path.join("new-sibling.bin")).unwrap(),
        b"unrelated"
    );
    close(&client, prepared.handle);
}

#[test]
fn exact_same_name_validates_without_namespace_writes() {
    let temp = Temp::new();
    let source = temp.path.join("same-name.bin");
    fs::write(&source, b"same entry").unwrap();
    let client = Client::new();
    let prepared = prepare(&client, &source);
    let before = names(&temp.path);
    let request = Request::Rename {
        prepared: prepared.handle,
        attempt: 1,
        name: name("same-name.bin"),
        policy: RenamePolicy::NoReplace,
    };
    let result = state(client.call(request.clone()), prepared.handle, 1);
    assert!(assert_renamed(&result, OsStr::new("same-name.bin")));
    assert_eq!(query(&client, prepared.handle), result);
    assert_eq!(client.call(request), Response::RenameState(result));
    assert_eq!(names(&temp.path), before);
    assert_eq!(fs::read(&source).unwrap(), b"same entry");
    close(&client, prepared.handle);
    assert_eq!(names(&temp.path), before);
}

#[test]
fn case_only_rename_preserves_actual_requested_spelling() {
    let temp = Temp::new();
    let source = temp.path.join("CaseOnly.TXT");
    fs::write(&source, b"case-only bytes").unwrap();
    let client = Client::new();
    let prepared = prepare(&client, &source);
    let state = rename(&client, prepared.handle, OsStr::new("caseonly.txt"));
    assert_renamed(&state, OsStr::new("caseonly.txt"));
    // exists() cannot distinguish these spellings on a case-insensitive volume.
    assert_eq!(names(&temp.path), vec![wide(OsStr::new("caseonly.txt"))]);
    assert_eq!(
        fs::read(temp.path.join("caseonly.txt")).unwrap(),
        b"case-only bytes"
    );
    close(&client, prepared.handle);
}

#[test]
fn ordinary_directory_rename_preserves_descendants() {
    let temp = Temp::new();
    let source = temp.path.join("selected-directory");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("nested/child.bin"), b"descendant bytes").unwrap();
    fs::write(source.join("sibling.bin"), b"sibling bytes").unwrap();
    let source_identity = native_identity(&source);
    let client = Client::new();
    let prepared = prepare(&client, &source);
    assert_eq!(prepared.source.kind, Kind::Directory);
    assert_eq!(prepared.source.metadata.size, None);
    assert!(!prepared.recoverable_replace);
    let children = names(&source);

    let state = rename(&client, prepared.handle, OsStr::new("renamed-directory"));
    assert_renamed(&state, OsStr::new("renamed-directory"));
    let destination = temp.path.join("renamed-directory");
    assert_eq!(native_identity(&destination), source_identity);
    assert!(!source.exists());
    assert_eq!(names(&destination), children);
    assert_eq!(
        fs::read(destination.join("nested/child.bin")).unwrap(),
        b"descendant bytes"
    );
    assert_eq!(
        fs::read(destination.join("sibling.bin")).unwrap(),
        b"sibling bytes"
    );
    close(&client, prepared.handle);
}

#[test]
fn selected_junction_rename_preserves_referent() {
    let temp = Temp::new();
    let target = temp.path.join("referent");
    let source = temp.path.join("selected-junction");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("sentinel.bin"), b"referent must stay unchanged").unwrap();
    junction(&source, &target);
    let source_identity = native_identity(&source);
    let target_identity = native_identity(&target);
    assert_ne!(source_identity, target_identity);
    let link_target = fs::read_link(&source).unwrap();
    let target_namespace = names(&target);
    let client = Client::new();
    let prepared = prepare(&client, &source);
    assert!(matches!(
        prepared.source.kind,
        Kind::DirectoryLink | Kind::Symlink
    ));
    assert!(!prepared.recoverable_replace);

    let state = rename(&client, prepared.handle, OsStr::new("renamed-junction"));
    assert_renamed(&state, OsStr::new("renamed-junction"));
    let destination = temp.path.join("renamed-junction");
    assert_eq!(native_identity(&destination), source_identity);
    assert_eq!(native_identity(&target), target_identity);
    assert!(fs::symlink_metadata(&source).is_err());
    assert_eq!(fs::read_link(&destination).unwrap(), link_target);
    assert_ne!(
        fs::symlink_metadata(&destination)
            .unwrap()
            .file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT,
        0
    );
    assert_eq!(names(&target), target_namespace);
    assert_eq!(
        fs::read(target.join("sentinel.bin")).unwrap(),
        b"referent must stay unchanged"
    );
    close(&client, prepared.handle);
}

#[test]
fn unicode_names_are_preserved_without_normalization() {
    let temp = Temp::new();
    let source = temp.path.join("selected e\u{301} 文档.bin");
    let destination_name = OsStr::new("cafe\u{301} 文档.bin");
    let composed_sibling = temp.path.join("café 文档.bin");
    fs::write(&source, b"decomposed selected name").unwrap();
    fs::write(&composed_sibling, b"composed sibling").unwrap();
    let client = Client::new();
    let prepared = prepare(&client, &source);
    assert_eq!(prepared.source.name, name("selected e\u{301} 文档.bin"));

    let state = rename(&client, prepared.handle, destination_name);
    assert_renamed(&state, destination_name);
    assert_eq!(
        fs::read(temp.path.join(destination_name)).unwrap(),
        b"decomposed selected name"
    );
    assert_eq!(fs::read(&composed_sibling).unwrap(), b"composed sibling");
    let mut expected = vec![wide(destination_name), wide(OsStr::new("café 文档.bin"))];
    expected.sort();
    assert_eq!(names(&temp.path), expected);
    close(&client, prepared.handle);
}

#[test]
fn unpaired_utf16_source_and_destination_remain_lossless() {
    let temp = Temp::new();
    let source_name = OsString::from_wide(&[0x0073, 0xD800, 0x002E, 0x0062]);
    let destination_name = OsString::from_wide(&[0x0064, 0xDC00, 0x002E, 0x0062]);
    let source = temp.path.join(&source_name);
    fs::write(&source, b"raw UTF-16 bytes").unwrap();
    let client = Client::new();
    let prepared = prepare(&client, &source);
    assert_eq!(prepared.source.name, name(&source_name));

    let state = rename(&client, prepared.handle, &destination_name);
    assert_renamed(&state, &destination_name);
    assert_eq!(names(&temp.path), vec![wide(&destination_name)]);
    assert_eq!(
        fs::read(temp.path.join(&destination_name)).unwrap(),
        b"raw UTF-16 bytes"
    );
    close(&client, prepared.handle);
}

#[test]
fn paths_beyond_max_path_are_not_truncated() {
    let temp = Temp::new();
    let mut parent = temp.path.canonicalize().unwrap();
    for _ in 0..12 {
        parent = parent.join("long native directory 文档");
    }
    fs::create_dir_all(&parent).unwrap();
    let source = parent.join("selected 文档.bin");
    let destination_name = OsStr::new("renamed 文档.bin");
    fs::write(&source, b"beyond MAX_PATH").unwrap();
    assert!(source.as_os_str().encode_wide().count() > 260);
    let client = Client::new();
    let prepared = prepare(&client, &source);
    assert_eq!(prepared.source.name, name("selected 文档.bin"));

    let state = rename(&client, prepared.handle, destination_name);
    assert_renamed(&state, destination_name);
    assert!(!source.exists());
    assert_eq!(names(&parent), vec![wide(destination_name)]);
    assert_eq!(
        fs::read(parent.join(destination_name)).unwrap(),
        b"beyond MAX_PATH"
    );
    close(&client, prepared.handle);
}

#[test]
fn observed_extended_sources_rename_to_strict_destination_names() {
    let temp = Temp::new();
    // Canonicalization gives an extended absolute path. Every unusual source is
    // created beneath this owned fixture, never through a Win32 device spelling.
    let parent = temp.path.canonicalize().unwrap();
    assert!(
        parent
            .as_os_str()
            .encode_wide()
            .take(4)
            .eq("\\\\?\\".encode_utf16())
    );
    let client = Client::new();
    for (source_name, destination_name) in [
        ("observed.", "formerly-trailing-dot.bin"),
        ("observed ", "formerly-trailing-space.bin"),
        ("CON.fixture", "formerly-dos-name.bin"),
    ] {
        let source = parent.join(source_name);
        fs::write(&source, b"extended existing source").unwrap();
        assert!(names(&parent).contains(&wide(OsStr::new(source_name))));
        let prepared = prepare(&client, &source);
        assert_eq!(prepared.source.name, name(source_name));
        let state = rename(&client, prepared.handle, OsStr::new(destination_name));
        assert_renamed(&state, OsStr::new(destination_name));
        assert!(!names(&parent).contains(&wide(OsStr::new(source_name))));
        assert_eq!(
            fs::read(parent.join(destination_name)).unwrap(),
            b"extended existing source"
        );
        close(&client, prepared.handle);
    }
}

fn name(value: impl AsRef<OsStr>) -> NativeText {
    NativeText {
        encoding: NameEncoding::WindowsUtf16Le,
        bytes: value
            .as_ref()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect(),
    }
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().collect()
}

#[allow(unsafe_code)]
fn native_identity(path: &Path) -> (u64, [u8; 16]) {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdInfo,
        GetFileInformationByHandleEx,
    };
    // Metadata-only access: directories are supported and a selected reparse
    // point is opened as the object itself, without following its referent.
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .unwrap();
    let mut info = FILE_ID_INFO::default();
    // SAFETY: the owned handle remains live and the exact FILE_ID_INFO output
    // buffer is aligned and writable until this synchronous call returns.
    assert_ne!(
        unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                (&mut info as *mut FILE_ID_INFO).cast(),
                size_of::<FILE_ID_INFO>() as u32,
            )
        },
        0,
        "fixture identity query failed: {}",
        std::io::Error::last_os_error()
    );
    (info.VolumeSerialNumber, info.FileId.Identifier)
}

fn names(parent: &Path) -> Vec<Vec<u16>> {
    let mut entries: Vec<_> = fs::read_dir(parent)
        .unwrap()
        .map(|entry| wide(&entry.unwrap().file_name()))
        .collect();
    entries.sort();
    entries
}

fn prepare(client: &Client, path: &Path) -> PreparedRename {
    let (mount, node) = client.resolve(path);
    prepare_node(client, mount, node.id)
}

fn prepare_node(client: &Client, mount: u64, node: u64) -> PreparedRename {
    let response = client.call(Request::PrepareRename { mount, node });
    let Response::RenamePrepared(prepared) = response else {
        panic!("native PrepareRename must pin the selected object: {response:?}");
    };
    prepared
}

fn state(response: Response, prepared: u64, attempt: u64) -> RenameState {
    let Response::RenameState(state) = response else {
        panic!("native Rename must return its retained attempt result: {response:?}");
    };
    assert_eq!(state.prepared, prepared);
    assert_eq!(state.attempt, attempt);
    state
}

fn rename(client: &Client, prepared: u64, destination: &OsStr) -> RenameState {
    state(
        client.call(Request::Rename {
            prepared,
            attempt: 1,
            name: name(destination),
            policy: RenamePolicy::NoReplace,
        }),
        prepared,
        1,
    )
}

fn query(client: &Client, prepared: u64) -> RenameState {
    let response = client.cancelled(Request::GetRenameState { prepared }, 1);
    let Response::RenameState(state) = response else {
        panic!("query must preserve the prepared receipt despite cancellation: {response:?}");
    };
    assert_eq!(state.prepared, prepared);
    state
}

fn close(client: &Client, prepared: u64) {
    assert_eq!(
        client.cancelled(Request::CloseRename { prepared }, 1),
        Response::Unit
    );
}

fn assert_unchanged(state: &RenameState, expected: ErrorCode) {
    let RenameOutcome::Unchanged(failure) = &state.outcome else {
        panic!("must refuse before changing the namespace: {state:?}");
    };
    assert_eq!(failure.code, expected);
}

fn assert_renamed(state: &RenameState, destination: &OsStr) -> bool {
    let RenameOutcome::Renamed {
        name: actual,
        same_entry,
        recovery,
        ..
    } = &state.outcome
    else {
        panic!("qualified same-parent Rename must succeed: {state:?}");
    };
    assert_eq!(*actual, name(destination));
    assert!(
        recovery.is_empty(),
        "ordinary Rename left recovery records: {state:?}"
    );
    *same_entry
}

fn assert_failed(response: Response, expected: ErrorCode) {
    let Response::Failed(failure) = response else {
        panic!("request must fail without namespace writes: {response:?}");
    };
    assert_eq!(failure.code, expected);
}

fn junction(path: &Path, target: &Path) {
    // As in cloud_content.rs, mklink /J needs no symlink privilege. Only this
    // fresh fixture is created through cmd; Rust owns all cleanup.
    let output = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(path.to_str().unwrap().replace('/', "\\"))
        .arg(target.to_str().unwrap().replace('/', "\\"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
