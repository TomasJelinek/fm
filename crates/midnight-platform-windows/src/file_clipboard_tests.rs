use super::*;
use std::{ffi::OsString, os::windows::ffi::OsStringExt};

#[test]
fn hdrop_round_trip_keeps_unpaired_utf16_and_has_a_unicode_header() {
    let path = PathBuf::from(OsString::from_wide(&[67, 58, 92, 0xd800, 0x6587, 32, 38]));
    let payload = FileClipboard {
        paths: vec![path.clone()],
        intent: FileClipboardIntent::Copy,
    };
    let bytes = encode_hdrop(&payload).unwrap();
    assert_eq!(
        &bytes[..20],
        &[20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]
    );
    assert_eq!(
        &bytes[20..],
        &[
            67, 0, 58, 0, 92, 0, 0, 216, 135, 101, 32, 0, 38, 0, 0, 0, 0, 0
        ]
    );
    assert_eq!(decode_hdrop(&bytes).unwrap(), vec![path]);
}

#[test]
fn hdrop_rejects_missing_terminators_offsets_ansi_and_relative_paths() {
    let payload = FileClipboard {
        paths: vec!["C:\\file.txt".into()],
        intent: FileClipboardIntent::Copy,
    };
    let bytes = encode_hdrop(&payload).unwrap();
    for changed in [
        bytes[..19].to_vec(),
        bytes[..bytes.len() - 2].to_vec(),
        {
            let mut value = bytes.clone();
            value[0] = 21;
            value
        },
        {
            let mut value = bytes.clone();
            value[0] = 255;
            value
        },
        {
            let mut value = bytes.clone();
            value[16] = 0;
            value
        },
        {
            let mut value = bytes.clone();
            value[20] = 0;
            value[21] = 0;
            value
        },
    ] {
        assert!(decode_hdrop(&changed).is_err());
    }
    assert!(
        encode_hdrop(&FileClipboard {
            paths: vec!["relative.txt".into()],
            intent: FileClipboardIntent::Copy
        })
        .is_err()
    );
    assert!(decode_hdrop(&vec![0; 4 * 1024 * 1024 + 1]).is_err());
}

#[test]
fn preferred_drop_effect_distinguishes_move_and_rejects_unknown_or_ambiguous_effects() {
    assert_eq!(decode_drop_effect(None).unwrap(), FileClipboardIntent::Copy);
    assert_eq!(
        decode_drop_effect(Some(&[1, 0, 0, 0])).unwrap(),
        FileClipboardIntent::Copy
    );
    assert_eq!(
        decode_drop_effect(Some(&[2, 0, 0, 0])).unwrap(),
        FileClipboardIntent::Move
    );
    for bytes in [
        &[][..],
        &[1, 0, 0],
        &[0, 0, 0, 0],
        &[3, 0, 0, 0],
        &[4, 0, 0, 0],
    ] {
        assert!(decode_drop_effect(Some(bytes)).is_err());
    }
}

#[test]
fn file_clipboard_owns_a_real_window_on_the_backend_session_thread() {
    use windows_sys::Win32::{
        System::Threading::GetCurrentThreadId,
        UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow},
    };
    let mut files = Files::new().unwrap();
    let window = files.window;
    assert!(!window.is_null());
    // SAFETY: inspect only the native window just created by this test's session.
    unsafe {
        assert_ne!(IsWindow(window), 0);
        assert_eq!(
            GetWindowThreadProcessId(window, ptr::null_mut()),
            GetCurrentThreadId()
        );
    }
    files.service_pending();
    drop(files);
    // SAFETY: IsWindow accepts a stale handle and reports whether it is still valid.
    assert_eq!(unsafe { IsWindow(window) }, 0);
}
