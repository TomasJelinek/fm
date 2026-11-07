//! Host-owned discovery, source validation, process supervision and archive session.
#![forbid(unsafe_code)]
mod discovery;
mod session;
pub use discovery::{Catalog, Package};
pub use session::Mount;
