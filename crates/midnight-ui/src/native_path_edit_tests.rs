use super::*;

#[cfg(windows)]
#[path = "native_path_edit_tests/windows.rs"]
mod native;
#[cfg(unix)]
#[path = "native_path_edit_tests/unix.rs"]
mod native;

#[test]
fn an_edited_lossy_directory_cannot_become_a_replacement_character_sibling() {
    let directory = native::raw_directory();
    assert!(ambiguous_edit(
        &directory,
        &format!("{}/child", directory.to_string_lossy())
    ));
}

#[test]
fn an_edited_file_path_keeps_its_raw_parent_out_of_text_interpretation() {
    let original = native::raw_directory().join("tool");
    let edited = original
        .with_file_name("other")
        .to_string_lossy()
        .into_owned();
    assert!(ambiguous_edit(original.parent().unwrap(), &edited));
}

#[test]
fn an_explicit_different_unicode_path_is_unambiguous() {
    assert!(!ambiguous_edit(
        &native::raw_directory(),
        &native::plain_directory().join("other").to_string_lossy()
    ));
}

#[test]
fn a_genuine_unicode_replacement_character_directory_remains_editable() {
    let directory = native::plain_directory().join("actual-\u{fffd}");
    assert!(!ambiguous_edit(
        &directory,
        &directory.join("other").to_string_lossy()
    ));
}

#[test]
fn replacing_only_an_invalid_basename_in_a_representable_parent_remains_explicit() {
    let original = native::raw_directory();
    assert!(!ambiguous_edit(
        original.parent().unwrap(),
        &original.to_string_lossy()
    ));
}
