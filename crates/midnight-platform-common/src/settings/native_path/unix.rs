use super::{EncodedPath, invalid, nibble};
use std::{ffi::OsString, fmt::Write, io, os::unix::ffi::OsStringExt, path::PathBuf};

pub(super) fn encode(path: PathBuf) -> EncodedPath {
    let mut hex = String::new();
    for byte in path.into_os_string().into_vec() {
        write!(&mut hex, "{byte:02x}").expect("writing to a string cannot fail");
    }
    EncodedPath {
        encoding: "unix-bytes".into(),
        hex,
    }
}

pub(super) fn decode(encoded: EncodedPath) -> io::Result<PathBuf> {
    if encoded.encoding != "unix-bytes" {
        return Err(invalid("foreign native path encoding; expected unix-bytes"));
    }
    if !encoded.hex.len().is_multiple_of(2) {
        return Err(invalid(
            "native byte path must have two hex digits per byte",
        ));
    }
    let bytes = encoded
        .hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|byte| Ok(((nibble(byte[0])? << 4) | nibble(byte[1])?) as u8))
        .collect::<io::Result<Vec<u8>>>()?;
    Ok(OsString::from_vec(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_unicode_tab_paths_roundtrip_through_preferences() {
        let mut preferences = midnight_core::UserPreferences::default();
        preferences.tabs[0] = midnight_core::PaneTabPreferences {
            networks: Default::default(),
            archives: Default::default(),
            paths: vec![PathBuf::from(OsString::from_vec(b"/tmp/\xff\xfe".to_vec()))],
            active: 0,
        };
        let bytes = crate::settings::encode_preferences(preferences.clone()).unwrap();
        assert_eq!(
            crate::settings::decode_preferences(&bytes).unwrap(),
            preferences
        );
        assert!(std::str::from_utf8(&bytes).unwrap().contains("unix-bytes"));
    }

    #[test]
    fn invalid_utf8_bytes_roundtrip_without_replacement() {
        let path = PathBuf::from(OsString::from_vec(b"/tmp/\xff\xfe".to_vec()));
        let encoded = super::super::NativePath::from_path(path.clone());
        let json = serde_json::to_string(&encoded).unwrap();
        assert!(json.contains("unix-bytes"));
        assert_eq!(encoded.into_path().unwrap(), path);
    }

    #[test]
    fn malformed_and_foreign_encodings_are_rejected() {
        for json in [
            r#"{"encoding":"windows-utf16","hex":"0043003a005c"}"#,
            r#"{"encoding":"unix-bytes","hex":"2f1"}"#,
            r#"{"encoding":"unix-bytes","hex":"zz"}"#,
            r#"{"encoding":"unix-bytes","hex":"2f","extra":true}"#,
            r#"{"encoding":"unix-bytes","encoding":"unix-bytes","hex":"2f"}"#,
        ] {
            let decoded = serde_json::from_str::<super::super::NativePath>(json);
            assert!(decoded.is_err() || decoded.unwrap().into_path().is_err());
        }
    }

    #[test]
    fn malformed_unicode_bookmark_roundtrips_through_preferences() {
        let path = PathBuf::from(OsString::from_vec(b"/tmp/\xff\xfe".to_vec()));
        let preferences = midnight_core::UserPreferences {
            bookmarks: vec![midnight_core::Bookmark {
                archive: None,
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
