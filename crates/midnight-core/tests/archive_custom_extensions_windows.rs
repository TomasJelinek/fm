#![cfg(windows)]
use midnight_core::{ArchiveExtensionHints, ArchiveFormat, UserPreferences};
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
};

#[test]
fn unicode_suffix_after_unpaired_surrogate_preserves_original_path() {
    let mut raw = vec![b'x' as u16, 0xd800, b'.' as u16];
    raw.extend("ŽLUŤOUČKÝ".encode_utf16());
    let path = PathBuf::from(OsString::from_wide(&raw));
    let mut preferences = UserPreferences::default();
    preferences
        .archive_custom_extensions
        .insert("žluťoučký".into(), ArchiveFormat::Zip);
    assert_eq!(
        ArchiveExtensionHints::from_path(&path).enabled_format(&preferences),
        Some(ArchiveFormat::Zip)
    );
    assert_eq!(path.as_os_str().encode_wide().collect::<Vec<_>>(), raw);
}
