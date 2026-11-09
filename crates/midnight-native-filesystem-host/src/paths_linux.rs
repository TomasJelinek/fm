use crate::discovery::invalid;
use midnight_filesystem_plugin_api::{Address, AddressEncoding, Failure, NameEncoding};
use std::{
    ffi::OsString,
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
}
