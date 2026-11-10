#![cfg(unix)]
use midnight_core::{ArchiveExtensionHints, ArchiveFormat, UserPreferences};
use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};

#[test]
fn unicode_suffix_after_non_utf8_basename_preserves_original_path() {
    let mut raw = b"/tmp/raw-\xff.".to_vec();
    raw.extend_from_slice("ŽLUŤOUČKÝ".as_bytes());
    let path = PathBuf::from(OsString::from_vec(raw.clone()));
    let mut preferences = UserPreferences::default();
    preferences
        .archive_custom_extensions
        .insert("žluťoučký".into(), ArchiveFormat::Zip);
    assert_eq!(
        ArchiveExtensionHints::from_path(&path).enabled_format(&preferences),
        Some(ArchiveFormat::Zip)
    );
    assert_eq!(path.into_os_string().into_vec(), raw);
}
