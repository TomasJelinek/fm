#![cfg(unix)]

use fm_core::{ShellFlavor, quote_shell_path};
use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};

#[test]
fn shell_insertion_rejects_non_utf8_instead_of_retargeting() {
    let path = PathBuf::from(OsString::from_vec(vec![b'/', 0xff]));
    assert!(quote_shell_path(&path, ShellFlavor::Posix).is_err());
}
