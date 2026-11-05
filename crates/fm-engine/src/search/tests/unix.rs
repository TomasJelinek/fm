use super::*;
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[test]
fn invalid_native_bytes_never_match_a_lossy_replacement_character() {
    let mut request = request("�", &["placeholder", "�valid"]);
    let source = Arc::make_mut(&mut request.source);
    source[0] = Entry::new(
        EntryId(1),
        PathBuf::from("folder").join(OsString::from_vec(vec![0xff])),
        EntryKind::File,
    );
    assert_eq!(rows(&ready(&request)), [(1, 3)]);
    request.query.clear();
    assert_eq!(ready(&request).matches.len(), 2);
}
