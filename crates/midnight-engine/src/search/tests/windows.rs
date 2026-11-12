use super::*;
use std::{ffi::OsString, os::windows::ffi::OsStringExt};

#[test]
fn unpaired_native_surrogates_never_match_a_lossy_replacement_character() {
    let mut request = request("�", &["placeholder", "�valid"]);
    let source = Arc::make_mut(&mut request.source);
    source[0] = Entry::new(
        EntryId(1),
        PathBuf::from("folder").join(OsString::from_wide(&[0xd800])),
        EntryKind::File,
    );
    assert_eq!(rows(&ready(&request)), [(1, 3)]);
    request.query.clear();
    assert_eq!(ready(&request).matches.len(), 2);
    request.filter = midnight_core::PanelFilter {
        enabled: true,
        expression: "*.none".into(),
        ..Default::default()
    };
    let result = ready(&request);
    assert_eq!(result.filtered.len(), 1);
    assert_eq!(
        result.source[result.filtered[0].source_row].name,
        OsString::from_wide(&[0xd800])
    );
}
