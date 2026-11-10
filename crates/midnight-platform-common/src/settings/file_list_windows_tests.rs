use crate::settings::{decode_preferences, encode_preferences};
use midnight_core::UserPreferences;
use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};

#[test]
fn file_list_destination_retains_unpaired_utf16_units_in_raw_native_encoding() {
    let mut preferences = UserPreferences::default();
    preferences.file_list.path = Some(PathBuf::from(OsString::from_wide(&[
        0x43, 0x3a, 0x5c, 0xd800, 0x5c, 0xdcff,
    ])));
    preferences.file_list.destination = 2;
    let encoded = encode_preferences(preferences.clone()).unwrap();
    assert!(
        std::str::from_utf8(&encoded)
            .unwrap()
            .contains("windows-utf16")
    );
    assert_eq!(decode_preferences(&encoded).unwrap(), preferences);
}
