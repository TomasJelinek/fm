use super::*;
use std::{ffi::OsString, os::windows::ffi::OsStringExt};

#[test]
fn invalid_native_unicode_stays_visible_but_never_matches_a_color_rule() {
    let name = OsString::from_wide(&[b'f' as u16, 0xd800]);
    let compiled = CompiledFilenameColors::new(&[FilenameColorRule::default()]).unwrap();
    assert!(compiled.style(&name, EntryKind::File).is_none());
    assert!(
        midnight_platform_common::filter::CompiledFilter::new(&Default::default())
            .unwrap()
            .matches(&name)
    );
}
