use super::ProviderError;
use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
};

pub(super) const PROVIDER: &str = "native.linux";
pub(super) const MAX_ENCODED_ADDRESS_BYTES: usize = super::nodes::MAX_PATH_STORAGE;
pub(super) fn encode(value: &OsStr) -> Result<Vec<u8>, ProviderError> {
    Ok(value.as_bytes().to_vec())
}
pub(super) fn decode(value: &[u8]) -> Result<PathBuf, ProviderError> {
    if value.contains(&0) {
        return Err(ProviderError::InvalidResponse("NUL in native address"));
    }
    Ok(PathBuf::from(OsString::from_vec(value.to_vec())))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_byte_round_trip_keeps_non_unicode_and_long_paths() {
        let mut bytes = b"/".to_vec();
        bytes.extend(std::iter::repeat_n(0xff, 10_000));
        let value = OsString::from_vec(bytes);
        assert_eq!(decode(&encode(&value).unwrap()).unwrap().as_os_str(), value);
        assert!(decode(&[0]).is_err());
    }
}
