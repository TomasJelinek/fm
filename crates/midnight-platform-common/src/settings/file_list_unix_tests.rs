use crate::settings::{decode_preferences, encode_preferences};
use midnight_core::UserPreferences;
use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};

#[test]
fn file_list_destination_retains_invalid_utf8_bytes_in_raw_native_encoding() {
    let mut preferences = UserPreferences::default();
    preferences.file_list.path = Some(PathBuf::from(OsString::from_vec(
        b"/tmp/\xff\xfe.txt".to_vec(),
    )));
    preferences.file_list.destination = 2;
    let encoded = encode_preferences(preferences.clone()).unwrap();
    assert!(
        std::str::from_utf8(&encoded)
            .unwrap()
            .contains("unix-bytes")
    );
    assert_eq!(decode_preferences(&encoded).unwrap(), preferences);
}
