use crate::discovery::invalid;
use midnight_filesystem_plugin_api::{
    Address, AddressEncoding, ErrorCode, Failure, MAX_RENAME_NAME_BYTES, NameEncoding, NativeText,
    WireError,
};
use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};
pub(crate) const ENCODING: AddressEncoding = AddressEncoding::UnixBytes;
#[cfg(test)]
pub(crate) const MANIFEST_ENCODING: &str = "unix-bytes";
pub(crate) fn encode(path: &Path) -> Result<Address, Failure> {
    let address = Address {
        encoding: ENCODING,
        bytes: path.as_os_str().as_bytes().to_vec(),
    };
    decode(&address)?;
    Ok(address)
}
pub(crate) fn decode(address: &Address) -> Result<PathBuf, Failure> {
    if address.encoding != ENCODING {
        return Err(invalid("Foreign native address encoding"));
    }
    let path = PathBuf::from(name(NameEncoding::UnixBytes, &address.bytes)?);
    if !path.is_absolute() {
        return Err(invalid("Native address must be absolute"));
    }
    Ok(path)
}
pub(crate) fn name(encoding: NameEncoding, bytes: &[u8]) -> Result<OsString, Failure> {
    if encoding != NameEncoding::UnixBytes || bytes.contains(&0) {
        return Err(invalid("Invalid native byte name"));
    }
    Ok(OsString::from_vec(bytes.to_vec()))
}

pub(crate) fn rename_name(name: &OsStr) -> Result<NativeText, Failure> {
    let resource_limit = || {
        Failure::new(
            ErrorCode::ResourceLimit,
            "Rename basename resource limit exceeded",
        )
    };
    let raw = name.as_bytes();
    if raw.len() > MAX_RENAME_NAME_BYTES {
        return Err(resource_limit());
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(raw.len())
        .map_err(|_| resource_limit())?;
    bytes.extend_from_slice(raw);
    let text = NativeText {
        encoding: NameEncoding::UnixBytes,
        bytes,
    };
    text.validate_rename_basename().map_err(|error| {
        if error == WireError::Limit {
            resource_limit()
        } else {
            invalid(error)
        }
    })?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_paths_preserve_invalid_utf8_and_have_no_max_path_cutoff() {
        let mut bytes = b"/".to_vec();
        bytes.extend("long-directory/".repeat(300).bytes());
        bytes.extend([0xff, b'x']);
        let path = PathBuf::from(OsString::from_vec(bytes));
        assert_eq!(decode(&encode(&path).unwrap()).unwrap(), path);
        assert!(name(NameEncoding::UnixBytes, &[0]).is_err());
        assert!(name(NameEncoding::WindowsUtf16Le, b"ab").is_err());
    }

    #[test]
    fn rename_name_preserves_invalid_utf8_and_literal_backslashes() {
        let raw = [b'a', 0xff, b'\\', 0xfe, b'.', b'x'];
        let original = OsString::from_vec(raw.to_vec());
        let encoded = rename_name(&original).unwrap();
        assert_eq!(encoded.encoding, NameEncoding::UnixBytes);
        assert_eq!(encoded.bytes, raw);
        assert_eq!(name(encoded.encoding, &encoded.bytes).unwrap(), original);
    }

    #[test]
    fn rename_name_preserves_distinct_unicode_spellings() {
        let composed = rename_name(std::ffi::OsStr::new("é.txt")).unwrap();
        let decomposed = rename_name(std::ffi::OsStr::new("e\u{301}.txt")).unwrap();
        assert_ne!(composed.bytes, decomposed.bytes);
        assert_eq!(
            name(composed.encoding, &composed.bytes).unwrap(),
            std::ffi::OsStr::new("é.txt")
        );
        assert_eq!(
            name(decomposed.encoding, &decomposed.bytes).unwrap(),
            std::ffi::OsStr::new("e\u{301}.txt")
        );
    }

    #[test]
    fn rename_name_checks_encoded_byte_budget_without_max_path_cutoff() {
        let long = OsString::from("ž".repeat(301));
        let encoded = rename_name(&long).unwrap();
        assert_eq!(encoded.bytes.len(), "ž".len() * 301);
        assert_eq!(name(encoded.encoding, &encoded.bytes).unwrap(), long);

        let maximum = midnight_filesystem_plugin_api::MAX_RENAME_NAME_BYTES;
        let boundary = OsString::from_vec(vec![0xff; maximum]);
        let encoded = rename_name(&boundary).unwrap();
        assert_eq!(encoded.bytes.len(), maximum);
        assert_eq!(name(encoded.encoding, &encoded.bytes).unwrap(), boundary);
        let overflow = OsString::from_vec(vec![0xff; maximum + 1]);
        assert_eq!(
            rename_name(&overflow).unwrap_err().code,
            midnight_filesystem_plugin_api::ErrorCode::ResourceLimit
        );
    }

    #[test]
    fn rename_name_rejects_invalid_unix_destinations() {
        for invalid_name in [b"".as_slice(), b".", b"..", b"child/name", b"\0", b"a\0b"] {
            assert_eq!(
                rename_name(std::ffi::OsStr::from_bytes(invalid_name))
                    .unwrap_err()
                    .code,
                midnight_filesystem_plugin_api::ErrorCode::Invalid,
                "{invalid_name:?}"
            );
        }
        for valid in ["CON.fixture", "trailing.", "trailing "] {
            let encoded = rename_name(std::ffi::OsStr::new(valid)).unwrap();
            assert_eq!(
                name(encoded.encoding, &encoded.bytes).unwrap(),
                std::ffi::OsStr::new(valid)
            );
        }
    }
}
