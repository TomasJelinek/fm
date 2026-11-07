use super::ProviderError;
use std::{
    ffi::{OsStr, OsString},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
};

pub(super) const PROVIDER: &str = "native.windows";
pub(super) const MAX_ENCODED_ADDRESS_BYTES: usize = super::nodes::MAX_PATH_STORAGE * 2;
pub(super) fn encode(value: &OsStr) -> Result<Vec<u8>, ProviderError> {
    Ok(value.encode_wide().flat_map(u16::to_le_bytes).collect())
}
pub(super) fn decode(value: &[u8]) -> Result<PathBuf, ProviderError> {
    let (units, remainder) = value.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(ProviderError::InvalidResponse("odd UTF-16 native address"));
    }
    let wide = units
        .iter()
        .copied()
        .map(u16::from_le_bytes)
        .collect::<Vec<_>>();
    if wide.contains(&0) {
        return Err(ProviderError::InvalidResponse("NUL in native address"));
    }
    Ok(PathBuf::from(OsString::from_wide(&wide)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_utf16_round_trip_keeps_unpaired_surrogates_and_long_paths() {
        let mut wide = "C:\\".encode_utf16().collect::<Vec<_>>();
        wide.extend(std::iter::repeat_n(0xd800, 10_000));
        let value = OsString::from_wide(&wide);
        assert_eq!(decode(&encode(&value).unwrap()).unwrap().as_os_str(), value);
        assert!(decode(&[0, 0]).is_err());
        assert!(decode(&[1]).is_err());
    }
}
