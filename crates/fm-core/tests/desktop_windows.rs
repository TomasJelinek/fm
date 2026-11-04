#![cfg(windows)]

use fm_core::{ShellFlavor, quote_shell_path};
use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};

#[test]
fn shell_insertion_rejects_unpaired_utf16_instead_of_retargeting() {
    let path = PathBuf::from(OsString::from_wide(&[0x0043, 0x003a, 0x005c, 0xd800]));
    assert!(quote_shell_path(&path, ShellFlavor::PowerShell).is_err());
}
