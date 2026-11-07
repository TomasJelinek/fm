use super::*;
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
};

#[test]
fn raw_utf16_parent_edits_cannot_save_or_preview_a_different_program_or_icon() {
    let mut raw: Vec<_> = absolute("raw-parent").as_os_str().encode_wide().collect();
    raw.push(0xd800);
    assert_raw_parent_edits_are_refused(PathBuf::from(OsString::from_wide(&raw)));
}

#[test]
fn unchanged_raw_windows_paths_survive_renaming_and_only_explicit_edits_replace_them() {
    let mut original = command();
    let mut raw: Vec<_> = absolute("tool").as_os_str().encode_wide().collect();
    raw.push(0xd800);
    let path = PathBuf::from(OsString::from_wide(&raw));
    original.icon = Some(path.clone());
    let UserCommandInvocation::Program { executable, .. } = &mut original.invocation else {
        panic!()
    };
    *executable = path;
    let mut form = Form::from_command(&original);
    form.name = "Different label".into();
    let accepted = form.command(Some(&original)).unwrap();
    assert_eq!(accepted.icon, original.icon);
    assert_eq!(accepted.invocation, original.invocation);
    form.executable_edited = true;
    let edited = form.command(Some(&original)).unwrap();
    assert_ne!(edited.invocation, original.invocation);
    assert_eq!(edited.icon, original.icon);
}
