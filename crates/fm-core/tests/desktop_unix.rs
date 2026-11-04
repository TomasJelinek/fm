#![cfg(unix)]

use fm_core::{ShellFlavor, quote_shell_path};
use std::{
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[test]
fn shell_insertion_rejects_non_utf8_instead_of_retargeting() {
    let path = PathBuf::from(OsString::from_vec(vec![b'/', 0xff]));
    assert!(quote_shell_path(&path, ShellFlavor::Posix).is_err());
}

#[test]
fn posix_argument_context_preserves_exact_bytes_without_expansion_or_word_splitting() {
    const SENTINEL: &str = "__FM_SENTINEL_9A81__";
    let cases = [
        "",
        "plain",
        "001",
        "0x10",
        "1e3",
        ".001",
        "-literal",
        "two words",
        " \tleading and trailing\t ",
        "line\r\nbreak",
        "no\u{a0}break\u{202f}space",
        "Žluťoučký_中文_日本語_العربية_עברית_👩🏽‍💻_e\u{301}",
        "\u{2018}",
        "\u{2019}",
        "\u{201a}",
        "\u{201b}",
        "\u{201c}",
        "\u{201d}",
        "\u{201e}",
        "\u{201f}",
        "name‘’‚‛“”„‟tail",
        "$HOME",
        "$(printf INJECTED)",
        "`printf INJECTED`",
        "a; printf INJECTED",
        "literal, @{} [] * ? #",
        "a\"b'c",
        "back\\slash",
        "C:\\with space\\tail\\",
        "back\\slash\"quote\\",
    ];
    for value in cases {
        let token = quote_shell_path(Path::new(value), ShellFlavor::Posix).unwrap();
        let script = format!("set -- {token} {SENTINEL}; printf '%s\\0' \"$#\" \"$@\"");
        let output = Command::new("/bin/sh")
            .args(["-c", &script])
            .env("LC_ALL", "C")
            .env_remove("ENV")
            .env_remove("BASH_ENV")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{value:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut expected = b"2\0".to_vec();
        expected.extend_from_slice(value.as_bytes());
        expected.push(0);
        expected.extend_from_slice(SENTINEL.as_bytes());
        expected.push(0);
        assert_eq!(output.stdout, expected, "{value:?}: {token}");
    }
    assert!(quote_shell_path(Path::new("a\0b"), ShellFlavor::Posix).is_err());
}
