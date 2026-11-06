use super::*;
use midnight_core::{ShellMenuModifiers, ShellMenuPosition, ShellMenuTarget};
use std::{os::windows::ffi::OsStringExt, path::PathBuf};

fn request(paths: Vec<PathBuf>) -> ShellMenuRequest {
    ShellMenuRequest {
        paths,
        owner: 0,
        position: ShellMenuPosition { x: 200, y: 200 },
        modifiers: ShellMenuModifiers::default(),
        target: ShellMenuTarget::Items,
    }
}

#[test]
fn shell_menu_native_paths_preserve_utf16_and_reject_nul() {
    let path = PathBuf::from(std::ffi::OsString::from_wide(&[
        67, 58, 92, 0xd800, 0x6587, 32, 38, 36, 46, 116, 120, 116,
    ]));
    assert_eq!(
        wide_path(&path).unwrap(),
        vec![67, 58, 92, 0xd800, 0x6587, 32, 38, 36, 46, 116, 120, 116, 0]
    );
    assert!(wide_path(Path::new("C:\\file\0.txt")).is_err());
}

#[test]
fn shell_menu_validates_drive_roots_without_resolving_or_truncating_paths() {
    for path in ["C:\\", "Z:\\", "\\\\server\\share\\", "\\\\?\\C:\\"] {
        let mut menu = request(vec![PathBuf::from(path)]);
        menu.target = ShellMenuTarget::Drive;
        assert!(validate_request(&menu).is_ok(), "{path}");
    }
    let mut menu = request(vec![PathBuf::from("C:\\folder")]);
    menu.target = ShellMenuTarget::Drive;
    assert!(validate_request(&menu).is_err());
}

#[test]
fn shell_menu_offsets_do_not_invoke_cancel_or_out_of_range_commands() {
    assert_eq!(command_offset(0, 12), None);
    assert_eq!(command_offset(1, 12), Some(0));
    assert_eq!(command_offset(12, 12), Some(11));
    assert_eq!(command_offset(13, 12), None);
}

#[test]
fn shell_menu_invalid_request_never_initializes_a_native_menu() {
    let filesystem = WindowsFileSystem::new(None);
    assert!(filesystem.show_shell_menu(&request(Vec::new())).is_err());
    assert!(
        filesystem
            .show_shell_menu(&request(vec![PathBuf::from("relative.txt")]))
            .is_err()
    );
}

#[test]
fn shell_menu_real_shell_resolves_unicode_multiselection_without_invoking() {
    let root = tempfile::tempdir().unwrap();
    let paths = vec![
        root.path().join("文档 & one.txt"),
        root.path().join("two.txt"),
    ];
    for path in &paths {
        std::fs::write(path, b"untouched").unwrap();
    }
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let _com = ComGuard::initialize(COINIT_APARTMENTTHREADED as u32).unwrap();
                let menu = bind_context_menu(ptr::null_mut(), &request(paths.clone())).unwrap();
                let popup = PopupMenu::new().unwrap();
                let count = query_menu(&menu, &popup, ShellMenuModifiers::default()).unwrap();
                assert!(count > 0);
                let copy = (0..count)
                    .find(|offset| canonical_verb(&menu, *offset) == Some(ShellMenuVerb::Copy))
                    .expect("the native file selection must expose its canonical Copy command");
                assert_eq!(
                    invoke_command(&menu, ptr::null_mut(), &request(paths.clone()), copy).unwrap(),
                    ShellMenuOutcome::Managed(ShellMenuVerb::Copy)
                );
            })
            .join()
            .unwrap();
    });
    for path in paths {
        assert_eq!(std::fs::read(path).unwrap(), b"untouched");
    }
}

#[test]
#[ignore = "requires an interactive Windows desktop; opens and automatically dismisses a Shell menu"]
fn shell_menu_interactive_native_popup_can_be_dismissed() {
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GUI_INMENUMODE, GUITHREADINFO, GetGUIThreadInfo, GetWindowThreadProcessId,
        PostMessageW, WM_CANCELMODE,
    };
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("Native menu smoke 文档.txt");
    std::fs::write(&path, b"untouched").unwrap();
    let menu_request = request(vec![path.clone()]);
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = sender.send(WindowsFileSystem::new(None).show_shell_menu(&menu_request));
    });
    let class: Vec<u16> = "MidnightExplorerShellMenuOwner\0".encode_utf16().collect();
    let started = Instant::now();
    loop {
        // SAFETY: lookup and thread-info calls inspect this test's native helper only.
        let window = unsafe { FindWindowW(class.as_ptr(), ptr::null()) };
        let mut process = 0;
        let thread = unsafe { GetWindowThreadProcessId(window, &mut process) };
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if !window.is_null()
            && process == unsafe { windows_sys::Win32::System::Threading::GetCurrentProcessId() }
            && unsafe { GetGUIThreadInfo(thread, &mut info) } != 0
            && info.flags & GUI_INMENUMODE != 0
        {
            assert_eq!(info.hwndMenuOwner, window);
            // SAFETY: cancel the visible native menu without selecting any command.
            assert_ne!(unsafe { PostMessageW(window, WM_CANCELMODE, 0, 0) }, 0);
            break;
        }
        if let Ok(result) = receiver.try_recv() {
            panic!("native menu returned before entering the Windows menu loop: {result:?}");
        }
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "native menu did not open"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap(),
        ShellMenuOutcome::Dismissed
    );
    assert_eq!(std::fs::read(path).unwrap(), b"untouched");
}
