use super::*;
use std::{ffi::OsString, os::windows::ffi::OsStringExt};

#[test]
fn unpaired_surrogate_names_with_identical_display_labels_remain_distinct() {
    let left = Entry::new(
        EntryId(1),
        Path::new("/left").join(OsString::from_wide(&[0xd800])),
        EntryKind::File,
    );
    let right = Entry::new(
        EntryId(1),
        Path::new("/right").join(OsString::from_wide(&[0xd801])),
        EntryKind::File,
    );
    assert_eq!(left.display_name, right.display_name);
    let mut capture = request(vec![left], vec![right]);
    Arc::make_mut(&mut capture).options.case_sensitive = false;
    let result = compute(capture, &|| false).unwrap();
    assert_eq!(
        result
            .selections
            .each_ref()
            .map(|selection| selection.len()),
        [1, 1]
    );
    assert_eq!(result.ambiguous, 0);
}
