//! Public platform-neutral interactive web-viewer contract.
#![forbid(unsafe_code)]

mod decode;
mod encode;
mod fields;
mod types;
mod validation;
mod wire;

pub use types::*;
pub use validation::{rgba_length, validate_read};
pub use wire::{read_message, write_message};
