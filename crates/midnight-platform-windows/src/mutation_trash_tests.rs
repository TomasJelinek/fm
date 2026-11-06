use super::*;
use std::{
    ffi::OsString,
    fs,
    os::windows::ffi::{OsStrExt, OsStringExt},
};
use windows::{
    Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        UI::Shell::{
            FOF_NOERRORUI, FOF_SILENT, FOFX_EARLYFAILURE, FileOperation, IFileOperation,
            SHCreateItemFromParsingName,
        },
    },
    core::PCWSTR,
};

#[test]
#[ignore = "requires Shell access outside the sandbox; recycles and restores only a fresh fixture"]
fn shell_recycles_unicode_and_restores_only_its_returned_item() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
    fs::create_dir_all(&root).unwrap();
    let temp = tempfile::Builder::new()
        .prefix("midnight-recycle-")
        .tempdir_in(fs::canonicalize(root).unwrap())
        .unwrap();
    let parent = temp.path().join("文档-recycle-path");
    fs::create_dir_all(&parent).unwrap();
    let path = parent.join("recycle 🦀 e\u{301}.txt");
    fs::write(&path, b"fixture owned by this test only").unwrap();
    // SAFETY: this test thread initializes an STA; the guard is created only on success.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
    }
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            /* SAFETY: balances successful initialization on this thread. */
            unsafe {
                CoUninitialize();
            }
        }
    }
    let _apartment = Apartment;
    // Hold the real prepared identity handles across recycling, as the engine does.
    let _prepared = midnight_core::MutationService::prepare(
        &crate::WindowsFileSystem::new(None),
        &path,
        &Cancellation::default(),
    )
    .unwrap();
    let recycled = recycle_shell(&path, &Cancellation::default(), || Ok(())).unwrap();
    assert!(!path.exists());
    // This operation restores only the exact IShellItem returned by this test's recycle.
    // It never enumerates or clears any pre-existing Recycle Bin contents.
    restore_item(&recycled, &path);
    assert_eq!(fs::read(&path).unwrap(), b"fixture owned by this test only");
}

fn restore_item(recycled: &IShellItem, path: &Path) {
    let parent_wide = shell_path(path.parent().unwrap());
    let name_wide: Vec<_> = path
        .file_name()
        .unwrap()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: COM is initialized; all interface pointers and terminated UTF-16 buffers are live.
    unsafe {
        let destination: IShellItem =
            SHCreateItemFromParsingName(PCWSTR(parent_wide.as_ptr()), None).unwrap();
        let operation: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER).unwrap();
        operation
            .SetOperationFlags(FOF_NOERRORUI | FOF_SILENT | FOFX_EARLYFAILURE)
            .unwrap();
        operation
            .MoveItem(recycled, &destination, PCWSTR(name_wide.as_ptr()), None)
            .unwrap();
        operation.PerformOperations().unwrap();
        assert!(!operation.GetAnyOperationsAborted().unwrap().as_bool());
    }
}

#[test]
#[ignore = "requires Shell access outside the sandbox; recycles and restores only a fresh junction"]
fn shell_recycles_only_the_junction_and_preserves_its_target() {
    use std::{os::windows::process::CommandExt, process::Command};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
    fs::create_dir_all(&root).unwrap();
    let temp = tempfile::Builder::new()
        .prefix("midnight-recycle-junction-")
        .tempdir_in(fs::canonicalize(root).unwrap())
        .unwrap();
    let target = temp.path().join("outside-target");
    fs::create_dir(&target).unwrap();
    let sentinel = target.join("sentinel");
    fs::write(&sentinel, b"target must stay intact").unwrap();
    let path = temp.path().join("junction 文档");
    // PowerShell writes an invalid reparse target when its -Value starts with \\?\.
    // Use the ordinary native spelling for this short, newly-created fixture target.
    let target_wide = shell_path(&target);
    let junction_target = OsString::from_wide(&target_wide[..target_wide.len() - 1]);
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../midnight-platform/tests/create-junction.ps1");
    let output = Command::new("powershell.exe")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
        .arg(script)
        .arg("-JunctionPath")
        .arg(&path)
        .arg("-TargetPath")
        .arg(junction_target)
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(path.join("sentinel")).unwrap(),
        b"target must stay intact"
    );
    let _apartment = crate::desktop::ComGuard::initialize(
        windows_sys::Win32::System::Com::COINIT_APARTMENTTHREADED as u32,
    )
    .unwrap();
    let prepared = midnight_core::MutationService::prepare(
        &crate::WindowsFileSystem::new(None),
        &path,
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(
        prepared.item().kind,
        midnight_core::EntryKind::DirectoryLink
    );
    let recycled = recycle_shell(&path, &Cancellation::default(), || Ok(())).unwrap();
    let removed_only_link =
        !path.exists() && fs::read(&sentinel).is_ok_and(|data| data == b"target must stay intact");
    // Restore the exact returned item before asserting, even if the observed result is wrong.
    restore_item(&recycled, &path);
    assert!(removed_only_link);
    assert_eq!(fs::read(&sentinel).unwrap(), b"target must stay intact");
    assert_eq!(
        fs::read(path.join("sentinel")).unwrap(),
        b"target must stay intact"
    );
    drop(prepared);
    fs::remove_dir(&path).unwrap();
}

#[test]
#[ignore = "requires Shell access outside the sandbox; creates only fresh long-path fixtures"]
fn unavailable_long_path_recycle_preserves_the_fixture_without_permanent_fallback() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
    fs::create_dir_all(&root).unwrap();
    let temp = tempfile::Builder::new()
        .prefix("midnight-recycle-long-")
        .tempdir_in(fs::canonicalize(root).unwrap())
        .unwrap();
    let mut path = temp.path().to_owned();
    for _ in 0..8 {
        path.push("文档-long-recycle-path-0123456789");
    }
    fs::create_dir_all(&path).unwrap();
    path.push("keep 🦀.txt");
    fs::write(&path, b"must not become permanent deletion").unwrap();
    let prepared = midnight_core::MutationService::prepare(
        &crate::WindowsFileSystem::new(None),
        &path,
        &Cancellation::default(),
    )
    .unwrap();
    let error = prepared
        .delete(
            midnight_core::DeleteMode::Trash,
            &Cancellation::default(),
            &mut |_| {},
        )
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::Unsupported);
    assert!(error.message.contains("path length"));
    assert_eq!(
        fs::read(&path).unwrap(),
        b"must not become permanent deletion"
    );
}
