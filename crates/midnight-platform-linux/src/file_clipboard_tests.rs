use super::*;
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[test]
fn file_uris_preserve_raw_unix_bytes_spaces_and_shell_syntax() {
    let path = PathBuf::from(OsString::from_vec(b"/tmp/a-\xff &$x".to_vec()));
    let payload = FileClipboard {
        paths: vec![path.clone()],
        intent: FileClipboardIntent::Copy,
    };
    assert_eq!(
        encode_uris(&payload).unwrap(),
        b"file:///tmp/a-%FF%20%26%24x\r\n"
    );
    assert_eq!(
        decode_uris(b"file:///tmp/a-%FF%20%26%24x\r\n").unwrap(),
        vec![path]
    );
}

#[test]
fn file_uris_accept_localhost_and_encoded_newlines_but_reject_remote_or_malformed_lists() {
    assert_eq!(
        decode_uris(b"# comment\r\nfile://localhost/tmp/a%0Ab\r\n").unwrap(),
        vec![PathBuf::from("/tmp/a\nb")]
    );
    for bytes in [
        &b""[..],
        b"file://server/tmp/a",
        b"https://example/a",
        b"file:///tmp/a%00b",
        b"file:///tmp/%",
        b"file:///tmp/%Q1",
        b"file:///tmp/a?query",
        b"file:///tmp/a#fragment",
        b"file:///tmp/a\nfile://remote/tmp/b",
        b"file://localhost/../tmp/a\ninvalid",
    ] {
        assert!(decode_uris(bytes).is_err(), "{bytes:?}");
    }
}

#[test]
fn gnome_clipboard_intent_is_explicit_and_unknown_headers_are_rejected() {
    assert_eq!(
        decode_gnome(b"copy\nfile:///tmp/a\n").unwrap().intent,
        FileClipboardIntent::Copy
    );
    assert_eq!(
        decode_gnome(b"cut\nfile:///tmp/a\n").unwrap().intent,
        FileClipboardIntent::Move
    );
    assert!(decode_gnome(b"move\nfile:///tmp/a\n").is_err());
    assert!(decode_gnome(b"file:///tmp/a\n").is_err());
}

#[test]
fn clipboard_backend_prefers_a_working_x11_bridge_over_wayland() {
    let selected = select_backend(true, true, || Ok(23)).unwrap();
    assert!(matches!(selected, ClipboardBackend::X11(23)));
}

#[test]
fn clipboard_backend_without_display_uses_read_only_wayland_and_rejects_copy() {
    let selected: ClipboardBackend<()> = select_backend(false, true, || {
        panic!("an absent DISPLAY must not initiate an X11 connection")
    })
    .unwrap();
    assert!(matches!(selected, ClipboardBackend::WaylandReadOnly));
    let mut files = Files::Wayland;
    let payload = FileClipboard {
        paths: vec!["/tmp/source".into()],
        intent: FileClipboardIntent::Copy,
    };
    assert_eq!(
        files.copy(&payload).unwrap_err().kind,
        io::ErrorKind::Unsupported
    );
}

#[test]
fn clipboard_backend_retains_read_only_fallback_and_reports_missing_displays() {
    let selected: ClipboardBackend<()> =
        select_backend(true, true, || Err(error("X11 refused"))).unwrap();
    assert!(matches!(selected, ClipboardBackend::WaylandReadOnly));
    let missing: Result<ClipboardBackend<()>, _> = select_backend(false, false, || {
        panic!("an absent DISPLAY must not initiate an X11 connection")
    });
    assert_eq!(missing.unwrap_err().kind, io::ErrorKind::Unsupported);
}
