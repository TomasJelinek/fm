use super::ProviderError;
use std::{ffi::OsStr, path::PathBuf};
pub(super) const PROVIDER: &str = "native.unavailable";
pub(super) const MAX_ENCODED_ADDRESS_BYTES: usize = 0;
pub(super) fn encode(_value: &OsStr) -> Result<Vec<u8>, ProviderError> {
    Err(ProviderError::Unsupported)
}
pub(super) fn decode(_value: &[u8]) -> Result<PathBuf, ProviderError> {
    Err(ProviderError::Unsupported)
}
