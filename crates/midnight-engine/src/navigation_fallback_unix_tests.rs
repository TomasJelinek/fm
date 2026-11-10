use super::*;
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

struct Root;
impl midnight_core::NavigationFallbackService for Root {
    fn navigation_fallback_root(&self) -> Result<PathBuf, midnight_core::FsError> {
        Ok(PathBuf::from("/"))
    }
}

#[test]
fn undecodable_native_parent_is_not_converted_through_display_text() {
    let parent = PathBuf::from(OsString::from_vec(b"/parent\xff".to_vec()));
    let child = parent.join("missing");
    let error =
        midnight_core::FsError::from_io("list", &child, std::io::ErrorKind::NotFound.into());
    assert_eq!(
        navigation_fallback::next_path(&child, &error, Some(&Root), &Cancellation::default())
            .unwrap(),
        Some(parent)
    );
}
