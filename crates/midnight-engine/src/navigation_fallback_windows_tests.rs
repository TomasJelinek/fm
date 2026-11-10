use super::*;
use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::Path};

struct Root;
impl midnight_core::NavigationFallbackService for Root {
    fn navigation_fallback_root(&self) -> Result<PathBuf, midnight_core::FsError> {
        Ok(PathBuf::from("C:\\"))
    }
}

#[test]
fn unavailable_drive_and_unc_share_roots_fall_back_to_os_drive_once() {
    let cancel = Cancellation::default();
    for path in [Path::new("Z:\\"), Path::new("\\\\offline\\share\\")] {
        let error = midnight_core::FsError::from_io("list", path, std::io::ErrorKind::Other.into());
        assert_eq!(
            navigation_fallback::next_path(path, &error, Some(&Root), &cancel).unwrap(),
            Some(PathBuf::from("C:\\"))
        );
    }
    let error = midnight_core::FsError::from_io(
        "list",
        Path::new("C:\\"),
        std::io::ErrorKind::PermissionDenied.into(),
    );
    assert!(
        navigation_fallback::next_path(Path::new("C:\\"), &error, Some(&Root), &cancel)
            .unwrap()
            .is_none()
    );
}

#[test]
fn unpaired_utf16_parent_is_not_converted_through_display_text() {
    let parent = PathBuf::from(OsString::from_wide(&[67, 58, 92, 0xd800]));
    let child = parent.join("missing");
    let error =
        midnight_core::FsError::from_io("list", &child, std::io::ErrorKind::NotFound.into());
    assert_eq!(
        navigation_fallback::next_path(&child, &error, Some(&Root), &Cancellation::default())
            .unwrap(),
        Some(parent)
    );
}
