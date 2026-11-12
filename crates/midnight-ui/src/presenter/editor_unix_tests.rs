use std::{
    ffi::OsString,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
};

#[test]
fn editor_refuses_edited_lossy_unix_parent_without_replacing_preferences() {
    let mut bytes = std::env::temp_dir()
        .join("raw-parent")
        .as_os_str()
        .as_bytes()
        .to_vec();
    bytes.push(0xff);
    super::tests::assert_raw_parent_edit_is_refused(PathBuf::from(OsString::from_vec(bytes)));
}
