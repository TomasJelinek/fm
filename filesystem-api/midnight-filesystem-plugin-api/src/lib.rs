//! Independent synchronous filesystem Browse/Read contract; no host/application dependency.
#![forbid(unsafe_code)]
pub mod abi;
mod activation;
pub use activation::ActivationTarget;
mod model;
mod rename;
pub use rename::*;
mod volume;
mod wire;
pub use model::*;
pub use volume::*;
pub use wire::{WireError, decode_request, decode_response, encode_request, encode_response};

pub const DEFAULT_REQUEST_BYTES: u64 = 32 * 1024 * 1024;
pub const DEFAULT_REPLY_BYTES: u64 = 1024 * 1024;
pub const MAX_BATCH: u32 = 128;
pub const MAX_READ: u32 = 256 * 1024;
pub const MAX_REVISION: usize = 4096;
pub const MAX_DIAGNOSTIC: usize = 4096;
