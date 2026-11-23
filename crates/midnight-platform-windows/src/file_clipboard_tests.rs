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
fn hdrop_accepts_any_nonzero_unicode_flag_as_dotnet_programs_write_it() {
    // The header WinForms' DataObject.SetFileDropList put on a drag into the app (seen
    // on Windows 11): pFiles 20, fWide 0xFFFFFFFF (BOOL TRUE as -1), then the list.
    let mut bytes = vec![
        0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff,
    ];
    for unit in r"D:\probe\incoming.txt".encode_utf16().chain([0, 0]) {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    assert_eq!(
        decode_hdrop(&bytes).unwrap(),
        vec![PathBuf::from(r"D:\probe\incoming.txt")]
    );
    // fWide FALSE is an ANSI list: still refused, never read as UTF-16.
    bytes[16..20].copy_from_slice(&[0; 4]);
    assert!(decode_hdrop(&bytes).is_err());
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
    // 1 = COPY (Midnight's own Copy), 5 = COPY | LINK (Explorer's Copy) and a
    // trailing byte beyond the DWORD do not change the decoded intent.
    for bytes in [&[1, 0, 0, 0][..], &[5, 0, 0, 0], &[5, 0, 0, 0, 9]] {
        assert_eq!(
            decode_drop_effect(Some(bytes)).unwrap(),
            FileClipboardIntent::Copy,
            "{bytes:?}"
        );
    }
    // 2 = MOVE (Cut) and 6 = MOVE | LINK select Move.
    for value in [2u8, 6] {
        assert_eq!(
            decode_drop_effect(Some(&[value, 0, 0, 0])).unwrap(),
            FileClipboardIntent::Move,
            "{value}"
        );
    }
    // Truncated, empty and link-only effects are rejected (links are never created),
    // and so are the ambiguous COPY | MOVE (3) and COPY | MOVE | LINK (7): a destructive
    // Move is never guessed. Unknown high bits are rejected too.
    for bytes in [
        &[][..],
        &[1, 0, 0],
        &[0, 0, 0, 0],
        &[3, 0, 0, 0],
        &[4, 0, 0, 0],
        &[7, 0, 0, 0],
        &[0, 0, 0, 0x80],
        &[1, 0, 0, 0x80],
    ] {
        assert!(decode_drop_effect(Some(bytes)).is_err(), "{bytes:?}");
    }
}

#[test]
fn clipboard_open_retries_only_a_briefly_held_clipboard_within_its_bound() {
    let denied = || io::Error::from_raw_os_error(ERROR_ACCESS_DENIED as i32);
    // Held twice by another process, then released: the third attempt succeeds.
    let (mut attempts, mut pauses) = (0, 0);
    let opened = retry_open(
        || {
            attempts += 1;
            if attempts <= 2 { Err(denied()) } else { Ok(()) }
        },
        || pauses += 1,
    );
    assert!(opened.is_ok());
    assert_eq!((attempts, pauses), (3, 2));
    // Held for the whole bound: the last native error is reported, without a final pause.
    let (mut attempts, mut pauses) = (0, 0);
    let error = retry_open(
        || {
            attempts += 1;
            Err(denied())
        },
        || pauses += 1,
    )
    .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(ERROR_ACCESS_DENIED as i32));
    assert_eq!((attempts, pauses), (OPEN_ATTEMPTS, OPEN_ATTEMPTS - 1));
    // Other failures are not contention and are returned immediately.
    let (mut attempts, mut pauses) = (0, 0);
    let error = retry_open(
        || {
            attempts += 1;
            Err(io::Error::from_raw_os_error(1400))
        },
        || pauses += 1,
    )
    .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(1400));
    assert_eq!((attempts, pauses), (1, 0));
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
