use super::*;
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[test]
fn invalid_utf8_names_with_equal_lossy_captions_have_different_signatures() {
    let first = Entry::new(
        midnight_core::EntryId(1),
        Path::new("root").join(OsString::from_vec(vec![0xff])),
        EntryKind::File,
    );
    let second = Entry::new(
        midnight_core::EntryId(2),
        Path::new("root").join(OsString::from_vec(vec![0xfe])),
        EntryKind::File,
    );
    assert_eq!(first.display_name, second.display_name);
    let mut left = Signature::default();
    let mut right = Signature::default();
    left.add(&first).unwrap();
    right.add(&second).unwrap();
    assert_ne!(left.digest(false), right.digest(false));
}
