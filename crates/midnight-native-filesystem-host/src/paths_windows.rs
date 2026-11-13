use crate::discovery::invalid;
use midnight_filesystem_plugin_api::{
    Address, AddressEncoding, ErrorCode, Failure, MAX_RENAME_NAME_BYTES, NameEncoding, NativeText,
    WireError,
};
use std::{
    ffi::{OsStr, OsString},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};
pub(crate) const ENCODING: AddressEncoding = AddressEncoding::WindowsUtf16Le;
#[cfg(test)]
pub(crate) const MANIFEST_ENCODING: &str = "windows-utf16le";
pub(crate) fn encode(path: &Path) -> Result<Address, Failure> {
    let address = Address {
        encoding: ENCODING,
        bytes: path
            .as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect(),
    };
    decode(&address)?;
    Ok(address)
}
pub(crate) fn decode(address: &Address) -> Result<PathBuf, Failure> {
    if address.encoding != ENCODING {
        return Err(invalid("Foreign native address encoding"));
    }
    let path = PathBuf::from(name(NameEncoding::WindowsUtf16Le, &address.bytes)?);
    if !path.is_absolute() {
        return Err(invalid("Native address must be absolute"));
    }
    Ok(path)
}
pub(crate) fn name(encoding: NameEncoding, bytes: &[u8]) -> Result<OsString, Failure> {
    let (units, tail) = bytes.as_chunks::<2>();
    if encoding != NameEncoding::WindowsUtf16Le || !tail.is_empty() || units.contains(&[0, 0]) {
        return Err(invalid("Invalid native UTF-16 name"));
    }
    Ok(OsString::from_wide(
        &units
            .iter()
            .map(|unit| u16::from_le_bytes(*unit))
            .collect::<Vec<_>>(),
    ))
}

pub(crate) fn rename_name(name: &OsStr) -> Result<NativeText, Failure> {
    let resource_limit = || {
        Failure::new(
            ErrorCode::ResourceLimit,
            "Rename basename resource limit exceeded",
        )
    };
    // Stop the preflight at one excess unit; accepted names are encoded in full.
    let byte_length = name
        .encode_wide()
        .take(MAX_RENAME_NAME_BYTES / 2 + 1)
        .count()
        .checked_mul(2)
        .filter(|length| *length <= MAX_RENAME_NAME_BYTES)
        .ok_or_else(resource_limit)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(byte_length)
        .map_err(|_| resource_limit())?;
    bytes.extend(name.encode_wide().flat_map(u16::to_le_bytes));
    let text = NativeText {
        encoding: NameEncoding::WindowsUtf16Le,
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
    fn native_paths_preserve_unpaired_units_and_have_no_max_path_cutoff() {
        let mut units: Vec<_> = "C:\\".encode_utf16().collect();
        units.extend("long-directory\\".repeat(30).encode_utf16());
        units.extend([0xd800, b'x' as u16]);
        let path = PathBuf::from(OsString::from_wide(&units));
        assert_eq!(decode(&encode(&path).unwrap()).unwrap(), path);
        assert!(name(NameEncoding::WindowsUtf16Le, &[0, 0]).is_err());
        assert!(name(NameEncoding::WindowsUtf16Le, &[1]).is_err());
        assert!(name(NameEncoding::UnixBytes, b"abc").is_err());
    }

    #[test]
    fn rename_name_preserves_raw_utf16_and_distinct_unicode_spellings() {
        let raw = OsString::from_wide(&[0xd800, 0x61, 0xdc00, 0xd83d, 0xde80]);
        let encoded = rename_name(&raw).unwrap();
        assert_eq!(encoded.encoding, NameEncoding::WindowsUtf16Le);
        assert_eq!(
            encoded.bytes,
            [0, 0xd8, b'a', 0, 0, 0xdc, 0x3d, 0xd8, 0x80, 0xde]
        );
        assert_eq!(name(encoded.encoding, &encoded.bytes).unwrap(), raw);

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
        assert_eq!(encoded.bytes.len(), 602);
        assert_eq!(name(encoded.encoding, &encoded.bytes).unwrap(), long);

        // Unpaired units occupy three WTF-8 bytes internally but two wire bytes.
        let maximum = midnight_filesystem_plugin_api::MAX_RENAME_NAME_BYTES;
        let boundary = OsString::from_wide(&vec![0xd800; maximum / 2]);
        let encoded = rename_name(&boundary).unwrap();
        assert_eq!(encoded.bytes.len(), maximum);
        assert_eq!(name(encoded.encoding, &encoded.bytes).unwrap(), boundary);
        let overflow = OsString::from_wide(&vec![0xd800; maximum / 2 + 1]);
        assert_eq!(
            rename_name(&overflow).unwrap_err().code,
            midnight_filesystem_plugin_api::ErrorCode::ResourceLimit
        );
    }

    #[test]
    fn rename_name_rejects_invalid_windows_destinations() {
        for invalid_name in [
            "",
            ".",
            "..",
            "child/name",
            "child\\name",
            "file:stream",
            "embedded\0nul",
            "bad\u{1}control",
            "bad<name",
            "bad>name",
            "bad\"name",
            "bad|name",
            "bad?name",
            "bad*name",
            "trailing.",
            "trailing ",
            "CON",
            "con.txt",
            "CONIN$",
            "conout$.log",
            "PRN",
            "AUX.dat",
            "NUL",
            "com1.txt",
            "LPT9",
            "COM¹",
            "LPT²",
        ] {
            assert_eq!(
                rename_name(std::ffi::OsStr::new(invalid_name))
                    .unwrap_err()
                    .code,
                midnight_filesystem_plugin_api::ErrorCode::Invalid,
                "{invalid_name:?}"
            );
        }
    }

    #[test]
    fn rename_name_keeps_observed_extended_windows_spellings_decodable() {
        for observed in ["observed.", "observed ", "CON.fixture"] {
            let text = midnight_filesystem_plugin_api::NativeText {
                encoding: NameEncoding::WindowsUtf16Le,
                bytes: observed.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            };
            text.validate_rename_observed_basename().unwrap();
            assert_eq!(
                name(text.encoding, &text.bytes).unwrap(),
                std::ffi::OsStr::new(observed)
            );
            assert_eq!(
                rename_name(std::ffi::OsStr::new(observed))
                    .unwrap_err()
                    .code,
                midnight_filesystem_plugin_api::ErrorCode::Invalid
            );
        }
    }
}
