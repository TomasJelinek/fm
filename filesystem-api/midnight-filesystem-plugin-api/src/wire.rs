mod decode;
mod encode;
mod validate;
pub use decode::{decode_request, decode_response};
pub use encode::{encode_request, encode_response};

const PREFIX: &[u8; 8] = b"FSBR\x01\0\0\0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    Limit,
    Malformed,
    Unsupported,
}
impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?} filesystem payload")
    }
}
impl std::error::Error for WireError {}
