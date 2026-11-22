//! Web and mail links for the Shell: the request names the open verb and the plain ASCII
//! address, and nothing else. No test calls `ShellExecuteExW`.
use super::*;

/// The NUL-terminated wide text at `text`, which must not be null.
fn wide(text: *const u16) -> Vec<u16> {
    assert!(!text.is_null());
    let mut units = Vec::new();
    let mut at = text;
    // SAFETY: callers pass a request's own NUL-terminated text or the static verb, which
    // stay alive while they are read here; reading stops at the terminator.
    unsafe {
        while *at != 0 {
            units.push(*at);
            at = at.add(1);
        }
    }
    units
}

#[test]
fn a_link_request_holds_only_the_open_verb_and_the_exact_ascii_address() {
    for link in [
        "https://example.com/a b?q=\"quoted\"&x=%41#top",
        "http://[::1]:8080/文档/",
        "mailto:someone@example.com?subject=Hi there&attach=C:\\secret.txt",
    ] {
        let url = ExternalUrl::parse(link).unwrap();
        let text = url.as_str();
        assert!(text.bytes().all(|byte| byte.is_ascii_graphic()), "{text}");
        let request = UrlOpen::new(&url);
        assert_eq!(
            request.file,
            text.encode_utf16().chain(Some(0)).collect::<Vec<_>>(),
            "the address is the only text, terminated once"
        );
        let info = request.info();
        assert_eq!(info.cbSize as usize, size_of::<SHELLEXECUTEINFOW>());
        assert_eq!(info.fMask, SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC);
        assert_eq!(wide(info.lpVerb), "open".encode_utf16().collect::<Vec<_>>());
        assert_eq!(info.lpFile, request.file.as_ptr());
        assert_eq!(String::from_utf16(&wide(info.lpFile)).unwrap(), text);
        assert_eq!(info.nShow, SW_SHOWNORMAL);
        // Nothing else: no parameters, working folder, class, item list, window or hot key.
        assert!(info.lpParameters.is_null());
        assert!(info.lpDirectory.is_null());
        assert!(info.lpClass.is_null());
        assert!(info.hkeyClass.is_null());
        assert!(info.lpIDList.is_null());
        assert!(info.hwnd.is_null());
        assert!(info.hInstApp.is_null());
        assert!(info.hProcess.is_null());
        assert_eq!(info.dwHotKey, 0);
        // SAFETY: both members of this union are handles; the request leaves it zeroed.
        assert!(unsafe { info.Anonymous.hIcon }.is_null());
    }
}

#[test]
fn a_mail_link_reaches_the_shell_without_the_attachments_it_asked_for() {
    let url = ExternalUrl::parse(
        "mailto:someone@example.com?attach=C:%5Csecret.txt&subject=Report&Attachment=x",
    )
    .unwrap();
    assert!(url.attachments_removed());
    let file = String::from_utf16(&UrlOpen::new(&url).file).unwrap();
    assert_eq!(file, "mailto:someone@example.com?subject=Report\0");
}
