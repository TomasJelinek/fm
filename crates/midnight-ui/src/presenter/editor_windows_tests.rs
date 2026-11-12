use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
};

#[test]
fn editor_refuses_edited_lossy_utf16_parent_without_replacing_preferences() {
    let mut units: Vec<_> = std::env::temp_dir()
        .join("raw-parent")
        .as_os_str()
        .encode_wide()
        .collect();
    units.push(0xd800);
    super::tests::assert_raw_parent_edit_is_refused(PathBuf::from(OsString::from_wide(&units)));
}
