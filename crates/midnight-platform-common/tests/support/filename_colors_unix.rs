use super::*;
use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

#[test]
fn invalid_native_unicode_stays_visible_but_never_matches_a_color_rule() {
    let name = OsStr::from_bytes(b"file\xff.rs");
    let compiled = CompiledFilenameColors::new(&[FilenameColorRule::default()]).unwrap();
    assert!(compiled.style(name, EntryKind::File).is_none());
    assert!(
        midnight_platform_common::filter::CompiledFilter::new(&Default::default())
            .unwrap()
            .matches(name)
    );
    assert_eq!(name.as_bytes(), b"file\xff.rs");
}
