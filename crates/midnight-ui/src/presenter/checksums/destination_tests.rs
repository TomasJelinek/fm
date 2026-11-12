use super::*;

#[cfg(unix)]
#[path = "destination_unix_tests.rs"]
mod native;
#[cfg(windows)]
#[path = "destination_windows_tests.rs"]
mod native;

#[test]
fn untouched_suggestion_keeps_the_exact_native_parent() {
    let suggested = native::raw_parent().join("checksums.sha256");
    assert_eq!(
        resolve(&suggested, &suggested.to_string_lossy()).unwrap(),
        suggested
    );
}

#[test]
fn filename_only_edits_keep_the_exact_native_parent() {
    let parent = native::raw_parent();
    let suggested = parent.join("checksums.sha256");
    assert_eq!(
        resolve(&suggested, "new 日本語.sha256").unwrap(),
        parent.join("new 日本語.sha256")
    );
}

#[test]
fn edited_lossy_absolute_path_cannot_target_a_replacement_character_sibling() {
    let parent = native::raw_parent();
    let suggested = parent.join("checksums.sha256");
    let sibling = PathBuf::from(parent.to_string_lossy().as_ref()).join("edited.sha256");
    assert_ne!(parent, sibling.parent().unwrap());
    assert!(resolve(&suggested, sibling.to_str().unwrap()).is_err());
}

#[test]
fn explicit_different_unicode_directory_remains_available() {
    let suggested = native::raw_parent().join("checksums.sha256");
    let destination = native::plain_parent().join("other 日本語.sha256");
    assert_eq!(
        resolve(&suggested, destination.to_str().unwrap()).unwrap(),
        destination
    );
}

#[test]
fn ordinary_unicode_replacement_characters_are_not_treated_as_lossy() {
    let suggested = native::plain_parent()
        .join("actual-\u{fffd}")
        .join("checksums.sha256");
    let destination = suggested.with_file_name("new.sha256");
    assert_eq!(
        resolve(&suggested, destination.to_str().unwrap()).unwrap(),
        destination
    );
}

#[test]
fn empty_navigation_relative_subpaths_and_nul_remain_invalid() {
    let suggested = native::plain_parent().join("checksums.sha256");
    for edited in ["", ".", "..", "relative/subpath.sha256", "bad\0name"] {
        assert!(resolve(&suggested, edited).is_err(), "{edited:?}");
    }
}
