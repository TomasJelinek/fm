#![cfg(windows)]

use midnight_core::{Cancellation, DeleteMode, MutationService, PreparedMutation};
use midnight_platform_windows::WindowsFileSystem;
use std::{ffi::OsStr, fs, io, path::Path, sync::Arc};

fn fixture() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("midnight-mutation-")
        .tempdir_in(root)
        .unwrap()
}

fn prepare(path: &Path) -> Arc<dyn PreparedMutation> {
    WindowsFileSystem::new(None)
        .prepare(path, &Cancellation::default())
        .unwrap()
}

#[test]
fn unicode_rename_preserves_bytes_and_refuses_destination_collision() {
    let temp = fixture();
    let source = temp.path().join("文档 🦀.txt");
    let collision = temp.path().join("existing.txt");
    fs::write(&source, b"original").unwrap();
    fs::write(&collision, b"existing").unwrap();
    let item = prepare(&source);
    assert_eq!(item.item().metadata.size, Some(8));
    assert!(
        item.rename(OsStr::new("existing.txt"), &Cancellation::default())
            .is_err()
    );
    assert_eq!(fs::read(&source).unwrap(), b"original");
    assert_eq!(fs::read(&collision).unwrap(), b"existing");
    let renamed = item
        .rename(
            OsStr::new("e\u{301} renamed 🦀.txt"),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(renamed, temp.path().join("e\u{301} renamed 🦀.txt"));
    assert_eq!(fs::read(renamed).unwrap(), b"original");
    assert!(!source.exists());
}

#[test]
fn case_only_rename_changes_the_actual_directory_entry_spelling() {
    let temp = fixture();
    let source = temp.path().join("Case.txt");
    fs::write(&source, b"payload").unwrap();
    prepare(&source)
        .rename(OsStr::new("CASE.txt"), &Cancellation::default())
        .unwrap();
    let names: Vec<_> = fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![OsStr::new("CASE.txt")]);
}

#[test]
fn invalid_names_do_not_mutate_the_source() {
    let temp = fixture();
    let source = temp.path().join("source.txt");
    fs::write(&source, b"payload").unwrap();
    let item = prepare(&source);
    for name in [
        "",
        ".",
        "..",
        "../escape",
        "a\\b",
        "C:escape",
        "stream:ads",
        "a?",
        "a*",
        "a<",
        "a>",
        "a|",
        "a\"",
        "a\0b",
        "a\nb",
        "trailing.",
        "trailing ",
        "CON",
        "con.txt",
        "PRN",
        "AUX",
        "NUL",
        "COM1",
        "lpt9.txt",
        "COM¹",
        "LPT².txt",
        "CONIN$",
        "CONOUT$",
    ] {
        assert_eq!(
            item.rename(OsStr::new(name), &Cancellation::default())
                .unwrap_err()
                .kind,
            io::ErrorKind::InvalidInput,
            "{name:?}"
        );
        assert_eq!(fs::read(&source).unwrap(), b"payload");
    }
}

#[test]
fn roots_parent_components_and_special_namespaces_are_rejected() {
    let service = WindowsFileSystem::new(None);
    for path in [
        "C:\\",
        "C:\\Windows\\..",
        "C:\\Windows\\.",
        "\\\\server\\share",
        "\\\\.\\PhysicalDrive0",
        "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume1",
        "C:\\file:stream",
        "relative.txt",
    ] {
        assert_eq!(
            service
                .prepare(Path::new(path), &Cancellation::default())
                .err()
                .unwrap()
                .kind,
            io::ErrorKind::InvalidInput,
            "{path:?}"
        );
    }
}

#[test]
fn replaced_source_is_rejected_for_rename_and_permanent_delete() {
    let temp = fixture();
    let source = temp.path().join("source");
    fs::write(&source, b"original").unwrap();
    let item = prepare(&source);
    fs::rename(&source, temp.path().join("moved")).unwrap();
    fs::write(&source, b"replacement").unwrap();
    assert!(
        item.rename(OsStr::new("renamed"), &Cancellation::default())
            .is_err()
    );
    assert!(
        item.delete(DeleteMode::Permanent, &Cancellation::default(), &mut |_| {})
            .is_err()
    );
    assert_eq!(fs::read(&source).unwrap(), b"replacement");
    assert_eq!(fs::read(temp.path().join("moved")).unwrap(), b"original");
}

#[test]
fn nested_permanent_delete_reports_entries_and_honors_partial_cancellation() {
    let temp = fixture();
    let root = temp.path().join("tree");
    fs::create_dir_all(root.join("nested")).unwrap();
    for name in ["a", "b", "nested/c"] {
        fs::write(root.join(name), b"payload").unwrap();
    }
    let item = prepare(&root);
    let cancel = Cancellation::default();
    let mut visited = Vec::new();
    let error = item
        .delete(DeleteMode::Permanent, &cancel, &mut |path| {
            visited.push(path.to_owned());
            cancel.cancel();
        })
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::Interrupted);
    assert_eq!(visited.len(), 1);
    assert!(root.exists());
    prepare(&root)
        .delete(DeleteMode::Permanent, &Cancellation::default(), &mut |_| {})
        .unwrap();
    assert!(!root.exists());
}

#[test]
fn cancelled_requests_leave_the_fixture_untouched() {
    let temp = fixture();
    let path = temp.path().join("source");
    fs::write(&path, b"payload").unwrap();
    let item = prepare(&path);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(
        WindowsFileSystem::new(None)
            .prepare(&path, &cancel)
            .is_err()
    );
    assert_eq!(
        item.rename(OsStr::new("new"), &cancel).unwrap_err().kind,
        io::ErrorKind::Interrupted
    );
    for mode in [DeleteMode::Permanent, DeleteMode::Trash] {
        assert_eq!(
            item.delete(mode, &cancel, &mut |_| {}).unwrap_err().kind,
            io::ErrorKind::Interrupted
        );
    }
    assert_eq!(fs::read(path).unwrap(), b"payload");
}

#[test]
fn paths_beyond_max_path_can_be_renamed_and_deleted() {
    let temp = fixture();
    let mut root = fs::canonicalize(temp.path()).unwrap();
    for _ in 0..8 {
        root.push("文档-long-path-component-0123456789");
    }
    fs::create_dir_all(&root).unwrap();
    let source = root.join("source.txt");
    fs::write(&source, b"long").unwrap();
    let renamed = prepare(&source)
        .rename(OsStr::new("renamed 🦀.txt"), &Cancellation::default())
        .unwrap();
    assert_eq!(fs::read(&renamed).unwrap(), b"long");
    prepare(&renamed)
        .delete(DeleteMode::Permanent, &Cancellation::default(), &mut |_| {})
        .unwrap();
    assert!(!renamed.exists());
}

#[test]
fn recursive_delete_removes_a_junction_without_touching_its_target() {
    let temp = fixture();
    let target = temp.path().join("outside");
    let tree = temp.path().join("tree");
    fs::create_dir(&target).unwrap();
    fs::create_dir(&tree).unwrap();
    fs::write(target.join("sentinel"), b"keep me").unwrap();
    let junction = tree.join("junction");
    create_junction(&junction, &target);
    assert_eq!(fs::read(junction.join("sentinel")).unwrap(), b"keep me");
    assert_eq!(
        prepare(&junction).item().kind,
        midnight_core::EntryKind::DirectoryLink
    );
    prepare(&tree)
        .delete(DeleteMode::Permanent, &Cancellation::default(), &mut |_| {})
        .unwrap();
    assert!(!tree.exists());
    assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"keep me");
}

fn create_junction(junction: &Path, target: &Path) {
    use std::{
        ffi::OsString,
        os::windows::{
            ffi::{OsStrExt, OsStringExt},
            process::CommandExt,
        },
        process::Command,
    };
    // PowerShell's junction creator does not normalize an extended target prefix.
    let units: Vec<u16> = target.as_os_str().encode_wide().collect();
    let prefix: Vec<u16> = "\\\\?\\".encode_utf16().collect();
    let target = if units.starts_with(&prefix) {
        OsString::from_wide(&units[4..])
    } else {
        target.as_os_str().to_owned()
    };
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../midnight-platform/tests/create-junction.ps1");
    let output = Command::new("powershell.exe")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
        .arg(script)
        .arg("-JunctionPath")
        .arg(junction)
        .arg("-TargetPath")
        .arg(target)
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn parent_replacement_does_not_retarget_the_prepared_entry() {
    let temp = fixture();
    let parent = temp.path().join("parent");
    let original = temp.path().join("original");
    fs::create_dir(&original).unwrap();
    create_junction(&parent, &original);
    fs::write(parent.join("source"), b"original").unwrap();
    let item = prepare(&parent.join("source"));
    fs::remove_dir(&parent).unwrap();
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("source"), b"replacement").unwrap();
    assert!(
        item.delete(DeleteMode::Permanent, &Cancellation::default(), &mut |_| {})
            .is_err()
    );
    assert_eq!(fs::read(parent.join("source")).unwrap(), b"replacement");
    assert_eq!(fs::read(original.join("source")).unwrap(), b"original");
}

#[test]
fn read_only_delete_errors_without_changing_attributes_or_contents() {
    let temp = fixture();
    let path = temp.path().join("readonly");
    fs::write(&path, b"keep me").unwrap();
    let original = fs::metadata(&path).unwrap().permissions();
    let mut permissions = original.clone();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();
    assert!(
        prepare(&path)
            .delete(DeleteMode::Permanent, &Cancellation::default(), &mut |_| {})
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), b"keep me");
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
    fs::set_permissions(&path, original).unwrap();
}

#[test]
fn create_nested_unicode_directories_keeps_existing_intermediates_and_refuses_final_collision() {
    let temp = fixture();
    fs::create_dir(temp.path().join("existing")).unwrap();
    let base = WindowsFileSystem::new(None)
        .prepare_create(temp.path(), &Cancellation::default())
        .unwrap();
    let created = base
        .create(
            Path::new("existing/文档 🦀/e\u{301}"),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(created, temp.path().join("existing/文档 🦀/e\u{301}"));
    assert!(created.is_dir());
    assert_eq!(
        base.create(
            Path::new("existing/文档 🦀/e\u{301}"),
            &Cancellation::default()
        )
        .unwrap_err()
        .kind,
        io::ErrorKind::AlreadyExists
    );
}

#[test]
fn create_rejects_invalid_names_and_cancelled_requests_without_changes() {
    let temp = fixture();
    let base = WindowsFileSystem::new(None)
        .prepare_create(temp.path(), &Cancellation::default())
        .unwrap();
    for path in [
        "",
        ".",
        "..",
        "good/.",
        "good/..",
        "\\\\.\\PhysicalDrive0",
        "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume1",
        "one/NUL.txt",
        "one/stream:ads",
        "one/trailing.",
    ] {
        assert_eq!(
            base.create(Path::new(path), &Cancellation::default())
                .unwrap_err()
                .kind,
            io::ErrorKind::InvalidInput,
            "{path:?}"
        );
    }
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(
        WindowsFileSystem::new(None)
            .prepare_create(temp.path(), &cancel)
            .is_err()
    );
    assert_eq!(
        base.create(Path::new("one/two"), &cancel).unwrap_err().kind,
        io::ErrorKind::Interrupted
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[test]
fn create_accepts_absolute_and_parent_relative_paths_outside_the_captured_base() {
    let temp = fixture();
    let panel = temp.path().join("panel");
    fs::create_dir(&panel).unwrap();
    let cancel = Cancellation::default();
    let base = WindowsFileSystem::new(None)
        .prepare_create(&panel, &cancel)
        .unwrap();
    let absolute = temp.path().join("absolute/文档 🦀/new");
    assert_eq!(base.create(&absolute, &cancel).unwrap(), absolute);
    assert!(absolute.is_dir());
    let relative = base
        .create(Path::new("../sibling/文档/new"), &cancel)
        .unwrap();
    assert_eq!(
        fs::canonicalize(relative).unwrap(),
        fs::canonicalize(temp.path().join("sibling/文档/new")).unwrap()
    );
    assert_eq!(fs::read_dir(panel).unwrap().count(), 0);
}

#[test]
fn create_walks_parent_components_without_collapsing_missing_directories_or_links() {
    let temp = fixture();
    let cancel = Cancellation::default();
    let base = WindowsFileSystem::new(None)
        .prepare_create(temp.path(), &cancel)
        .unwrap();
    let created = base
        .create(Path::new("first/../second/./leaf"), &cancel)
        .unwrap();
    assert!(temp.path().join("first").is_dir());
    assert_eq!(
        fs::canonicalize(created).unwrap(),
        fs::canonicalize(temp.path().join("second/leaf")).unwrap()
    );
    fs::create_dir(temp.path().join("outside")).unwrap();
    create_junction(&temp.path().join("link"), &temp.path().join("outside"));
    assert!(
        base.create(Path::new("link/../forbidden"), &cancel)
            .is_err()
    );
    assert!(!temp.path().join("forbidden").exists());
    assert_eq!(
        fs::read_dir(temp.path().join("outside")).unwrap().count(),
        0
    );
}

#[test]
fn create_resolves_root_relative_and_same_drive_relative_paths_against_the_panel() {
    let temp = fixture();
    let panel = temp.path().join("panel");
    fs::create_dir(&panel).unwrap();
    let cancel = Cancellation::default();
    let base = WindowsFileSystem::new(None)
        .prepare_create(&panel, &cancel)
        .unwrap();
    let absolute = temp.path().join("root-relative/new");
    let prefix = absolute.components().next().unwrap();
    let root_relative = absolute
        .components()
        .skip(1)
        .collect::<std::path::PathBuf>();
    assert_eq!(base.create(&root_relative, &cancel).unwrap(), absolute);
    let drive_relative = format!("{}same-drive/new", prefix.as_os_str().to_str().unwrap());
    assert_eq!(
        base.create(Path::new(&drive_relative), &cancel).unwrap(),
        panel.join("same-drive/new")
    );
    assert!(absolute.is_dir());
    assert!(panel.join("same-drive/new").is_dir());
}

#[test]
fn create_absolute_targets_still_honor_cancellation_and_replaced_base_identity() {
    let temp = fixture();
    let panel = temp.path().join("panel");
    fs::create_dir(&panel).unwrap();
    let cancel = Cancellation::default();
    let base = WindowsFileSystem::new(None)
        .prepare_create(&panel, &cancel)
        .unwrap();
    let absolute = temp.path().join("outside/new");
    cancel.cancel();
    assert_eq!(
        base.create(&absolute, &cancel).unwrap_err().kind,
        io::ErrorKind::Interrupted
    );
    assert!(!temp.path().join("outside").exists());
    fs::rename(&panel, temp.path().join("moved-panel")).unwrap();
    fs::create_dir(&panel).unwrap();
    assert!(base.create(&absolute, &Cancellation::default()).is_err());
    assert!(!temp.path().join("outside").exists());
}

#[test]
fn create_absolute_long_path_preserves_native_unicode_without_replacing_final_entries() {
    use std::os::windows::ffi::OsStringExt;
    let temp = fixture();
    let panel = temp.path().join("panel");
    fs::create_dir(&panel).unwrap();
    let cancel = Cancellation::default();
    let base = WindowsFileSystem::new(None)
        .prepare_create(&panel, &cancel)
        .unwrap();
    let mut target = temp.path().join("outside");
    for _ in 0..10 {
        target.push("文档-long-directory-0123456789");
    }
    target.push(std::ffi::OsString::from_wide(&[b'r' as u16, 0xd800]));
    assert_eq!(base.create(&target, &cancel).unwrap(), target);
    assert!(target.is_dir());
    assert_eq!(
        base.create(&target, &cancel).unwrap_err().kind,
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read_dir(panel).unwrap().count(), 0);
}

#[test]
fn create_does_not_follow_an_existing_junction_or_replace_a_file() {
    let temp = fixture();
    let outside = temp.path().join("outside");
    let base_path = temp.path().join("base");
    fs::create_dir(&outside).unwrap();
    fs::create_dir(&base_path).unwrap();
    create_junction(&base_path.join("link"), &outside);
    fs::write(base_path.join("file"), b"keep").unwrap();
    let base = WindowsFileSystem::new(None)
        .prepare_create(&base_path, &Cancellation::default())
        .unwrap();
    assert!(
        base.create(Path::new("link/escaped"), &Cancellation::default())
            .is_err()
    );
    assert!(
        base.create(Path::new("file/subdirectory"), &Cancellation::default())
            .is_err()
    );
    assert!(!outside.join("escaped").exists());
    assert_eq!(fs::read(base_path.join("file")).unwrap(), b"keep");
}

#[test]
fn create_supports_paths_beyond_max_path_and_leaves_intermediates_on_os_failure() {
    let temp = fixture();
    let base = WindowsFileSystem::new(None)
        .prepare_create(temp.path(), &Cancellation::default())
        .unwrap();
    let mut relative = std::path::PathBuf::new();
    for _ in 0..10 {
        relative.push("文档-long-directory-0123456789");
    }
    assert!(
        base.create(&relative, &Cancellation::default())
            .unwrap()
            .is_dir()
    );
    assert!(
        base.create(
            &Path::new("partial").join("x".repeat(300)),
            &Cancellation::default()
        )
        .is_err()
    );
    assert!(temp.path().join("partial").is_dir());
}
