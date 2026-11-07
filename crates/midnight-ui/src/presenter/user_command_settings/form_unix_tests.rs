use super::*;
use std::{
    ffi::OsString,
    os::unix::ffi::{OsStrExt, OsStringExt},
};

#[test]
fn raw_unix_parent_edits_cannot_save_or_preview_a_different_program_or_icon() {
    let mut raw = absolute("raw-parent").as_os_str().as_bytes().to_vec();
    raw.push(0xff);
    assert_raw_parent_edits_are_refused(PathBuf::from(OsString::from_vec(raw)));
}

#[test]
fn unchanged_raw_unix_paths_survive_renaming_and_only_explicit_edits_replace_them() {
    let mut original = command();
    let mut raw = absolute("tool").as_os_str().as_bytes().to_vec();
    raw.push(0xff);
    let path = PathBuf::from(OsString::from_vec(raw));
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
