use crate::discovery::invalid;
use midnight_filesystem_plugin_api::{Address, AddressEncoding, Failure, NameEncoding};
use std::{
    ffi::OsString,
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
}
