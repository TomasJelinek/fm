use super::{EncodedPath, invalid, nibble};
use std::os::windows::ffi::OsStrExt;
use std::{ffi::OsString, fmt::Write, io, os::windows::ffi::OsStringExt, path::PathBuf};

pub(super) fn encode(path: PathBuf) -> EncodedPath {
    let mut hex = String::new();
    for unit in path.as_os_str().encode_wide() {
        write!(&mut hex, "{unit:04x}").expect("writing to a string cannot fail");
    }
    EncodedPath {
        encoding: "windows-utf16".into(),
        hex,
    }
}

pub(super) fn decode(encoded: EncodedPath) -> io::Result<PathBuf> {
    if encoded.encoding != "windows-utf16" {
        return Err(invalid(
            "foreign native path encoding; expected windows-utf16",
        ));
    }
    if !encoded.hex.len().is_multiple_of(4) {
        return Err(invalid(
            "native UTF-16 path must have four hex digits per unit",
        ));
    }
    let units = encoded
        .hex
        .as_bytes()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|unit| {
            Ok(nibble(unit[0])? << 12
                | nibble(unit[1])? << 8
                | nibble(unit[2])? << 4
                | nibble(unit[3])?)
        })
        .collect::<io::Result<Vec<u16>>>()?;
    Ok(OsString::from_wide(&units).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_unicode_tab_paths_roundtrip_through_preferences() {
        let mut preferences = midnight_core::UserPreferences::default();
        preferences.tabs[0] = midnight_core::PaneTabPreferences {
            archives: Default::default(),
            paths: vec![PathBuf::from(OsString::from_wide(&[
                0x43, 0x3a, 0x5c, 0xd800,
            ]))],
            active: 0,
        };
        let bytes = crate::settings::encode_preferences(preferences.clone()).unwrap();
        assert_eq!(
            crate::settings::decode_preferences(&bytes).unwrap(),
            preferences
        );
        assert!(
            std::str::from_utf8(&bytes)
                .unwrap()
                .contains("windows-utf16")
        );
    }

    #[test]
    fn unpaired_utf16_surrogate_roundtrips_without_replacement() {
        let units = [0x43, 0x3a, 0x5c, 0xd800, 0x5c, 0xdcff];
        let path = PathBuf::from(OsString::from_wide(&units));
        let encoded = super::super::NativePath::from_path(path.clone());
        let json = serde_json::to_string(&encoded).unwrap();
        assert!(json.contains("windows-utf16"));
        assert_eq!(encoded.into_path().unwrap(), path);
    }

    #[test]
    fn malformed_and_foreign_encodings_are_rejected() {
        for json in [
            r#"{"encoding":"unix-bytes","hex":"2f746d70"}"#,
            r#"{"encoding":"windows-utf16","hex":"004"}"#,
            r#"{"encoding":"windows-utf16","hex":"zzzz"}"#,
            r#"{"encoding":"windows-utf16","hex":"0043","extra":true}"#,
            r#"{"encoding":"windows-utf16","encoding":"windows-utf16","hex":"0043"}"#,
        ] {
            let decoded = serde_json::from_str::<super::super::NativePath>(json);
            assert!(decoded.is_err() || decoded.unwrap().into_path().is_err());
        }
    }

    #[test]
    fn malformed_unicode_bookmark_roundtrips_through_preferences() {
        let path = PathBuf::from(OsString::from_wide(&[0x43, 0x3a, 0x5c, 0xd800]));
        let preferences = midnight_core::UserPreferences {
            bookmarks: vec![midnight_core::Bookmark {
                id: midnight_core::BookmarkId(1),
                name: "Raw".into(),
                path,
                slot: None,
            }],
            ..midnight_core::UserPreferences::default()
        };
        let bytes = crate::settings::encode_preferences(preferences.clone()).unwrap();
        assert_eq!(
            crate::settings::decode_preferences(&bytes).unwrap(),
            preferences
        );
    }
}
