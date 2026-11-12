//! Safe filesystem author boundary. Calls and final cleanup belong to host backend workers.
#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]
#[allow(unsafe_code)]
mod ffi;
pub mod native;
mod reply;
mod runtime;
pub use api::{Failure, Request, Response};
pub use ffi::{CallContext, entry};
pub use midnight_filesystem_plugin_api as api;
pub use reply::Reply;

pub trait Filesystem: Send + Sync + Sized + 'static {
    /// Construct bounded state, without starting a private worker pool.
    fn create(config: api::abi::Config) -> Result<Self, Failure>;
    fn handle(&self, request: Request, context: &CallContext<'_>) -> Result<Reply, Failure>;
}

/// Nonzero IDs are never reused by this loaded SDK module, including across instances.
pub fn allocate_handle() -> Result<u64, Failure> {
    runtime::next_id().map_err(|_| {
        Failure::new(
            api::ErrorCode::ResourceLimit,
            "Filesystem handle identities exhausted.",
        )
    })
}

#[macro_export]
macro_rules! export_filesystem {
    ($filesystem:ty,$id:literal,$version:literal,$capabilities:expr) => {
        #[allow(unsafe_code)]
        /// Filesystem C entry; the caller must supply aligned writable output capacity.
        /// # Safety
        /// `out` must remain mapped for `capacity` bytes throughout this call.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn midnight_filesystem_entry_v1(
            out: *mut $crate::api::abi::Api,
            capacity: u32,
        ) -> $crate::api::abi::Status {
            // SAFETY: the exported C caller supplies the documented writable capacity.
            unsafe { $crate::entry::<$filesystem>(out, capacity, $id, $version, $capabilities) }
        }
    };
}

#[cfg(test)]
#[allow(unsafe_code)]
mod ffi_tests;
