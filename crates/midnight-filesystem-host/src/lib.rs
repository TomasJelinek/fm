//! Host-owned discovery, source validation, process supervision and archive session.
#![forbid(unsafe_code)]
mod discovery;
mod nested;
mod session;
mod staging;
pub use discovery::{Catalog, Package};
/// The decoder that a package opens an archive with (`Catalog::choose_format`).
pub use midnight_filesystem_api::Format;
pub use session::{Mount, NativeAllowance, NativeLimit, NativeSession, NativeSlot, Reservation};
#[cfg(test)]
mod captured_source_tests;
