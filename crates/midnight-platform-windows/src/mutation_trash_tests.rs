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
            COPYENGINE_E_ACCESS_DENIED_DEST, FOF_NOERRORUI, FOF_SILENT, FOFX_EARLYFAILURE,
            FileOperation, IFileOperation, SHCreateItemFromParsingName,
        },
    },
    core::PCWSTR,
};

#[test]
fn only_unsupported_recycling_is_a_recycle_bin_refusal() {
    let path = Path::new(r"D:\test.txt");
    for value in [ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_NOT_FOUND] {
        let code = windows::core::HRESULT::from_win32(value.0);
        assert!(
            recycle_destination_error(path, windows::core::Error::from_hresult(code))
                .is_trash_unavailable()
        );
        assert!(
            !recycle_error(path, windows::core::Error::from_hresult(code)).is_trash_unavailable()
        );
    }
    for code in [
        COPYENGINE_E_RECYCLE_BIN_NOT_FOUND,
        COPYENGINE_E_RECYCLE_FORCE_NUKE,
        COPYENGINE_E_RECYCLE_PATH_TOO_LONG,
        COPYENGINE_E_RECYCLE_SIZE_TOO_BIG,
        E_NOINTERFACE,
        E_NOTIMPL,
    ] {
        assert!(
            recycle_error(path, windows::core::Error::from_hresult(code)).is_trash_unavailable()
        );
    }
    for code in [
        COPYENGINE_E_ACCESS_DENIED_SRC,
        COPYENGINE_E_SHARING_VIOLATION_SRC,
        COPYENGINE_E_ACCESS_DENIED_DEST,
    ] {
        assert!(
            !recycle_error(path, windows::core::Error::from_hresult(code)).is_trash_unavailable()
        );
    }
}

#[test]
fn in_use_recycle_refusals_are_readable_retryable_and_keep_the_path() {
    let path = Path::new("D:\\work\\文档 🦀\\in use e\u{301}");
    // The Shell also reports other refused access with 0x80270021, which its message names.
    for (code, hex, names_denied_access) in [
        (COPYENGINE_E_ACCESS_DENIED_SRC, "0x80270021", true),
        (COPYENGINE_E_SHARING_VIOLATION_SRC, "0x80270027", false),
    ] {
        let error = recycle_error(path, windows::core::Error::from_hresult(code));
        assert_eq!(error.kind, io::ErrorKind::ResourceBusy, "{error}");
        assert_eq!((error.operation, error.path.as_path()), ("recycle", path));
        let message = &error.message;
        // This application's own panels, expanded folders and viewers keep entries open
        // too, so the message names them instead of blaming only another program.
        assert!(
            message.starts_with(
                "the item or something inside it is open in a program, which can be this \
                 application (a panel, an expanded folder or a viewer)"
            ),
            "{error}"
        );
        assert!(!message.contains("another program"), "{error}");
        assert!(
            message.contains("; retry when it is no longer open; "),
            "{error}"
        );
        assert!(message.contains("not permanently deleted"), "{error}");
        assert!(message.contains(hex), "{error}");
        assert_eq!(
            message.contains("access to it is denied"),
            names_denied_access,
            "{error}"
        );
        let shown = error.to_string();
        assert!(shown.ends_with(&format!("({})", path.display())), "{shown}");
    }
    // Any other code, such as a refusal by the Recycle Bin destination, keeps its own code.
    let error = recycle_error(
        path,
        windows::core::Error::from_hresult(COPYENGINE_E_ACCESS_DENIED_DEST),
    );
    assert_eq!(error.kind, io::ErrorKind::Other, "{error}");
    assert!(error.message.contains("0x80270022"), "{error}");
    assert_eq!(error.path, path);
}

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

/// A folder on a volume whose Recycle Bin is set to remove files immediately ("Don't
/// move files to the Recycle Bin", `NukeOnDelete`): Trash must refuse in words and keep
/// the file, never delete it permanently on the Shell's behalf.
#[test]
#[ignore = "requires Shell access and MIDNIGHT_TEST_NUKE_DIR: a folder the test owns on a volume whose Recycle Bin removes files immediately"]
fn trash_on_a_volume_that_removes_files_immediately_refuses_and_keeps_the_file() {
    let Some(root) = std::env::var_os("MIDNIGHT_TEST_NUKE_DIR") else {
        return;
    };
    let temp = tempfile::Builder::new()
        .prefix("midnight-recycle-nuke-")
        .tempdir_in(root)
        .unwrap();
    let path = temp.path().join("Screenshot 2023-03-31 105053.png");
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
    eprintln!("refusal: {error:?}");
    assert!(error.is_trash_unavailable(), "{error:?}");
    assert_eq!(
        fs::read(&path).unwrap(),
        b"must not become permanent deletion"
    );
}
