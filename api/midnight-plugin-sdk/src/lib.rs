//! Public SDK. All concrete parsing and its tests remain in independent plugins.
#![deny(unsafe_code)]

mod buffers;
#[allow(unsafe_code)]
mod ffi;
mod guard;
#[allow(unsafe_code)]
mod host;
mod jobs;
mod outbox;
mod output;
mod plugin;
mod runtime;

#[doc(hidden)]
pub use ffi::entry as __entry;
pub use host::Content;
pub use midnight_plugin_api as api;
pub use outbox::Output;
pub use plugin::{Request, Viewer};
pub use runtime::{Context, ReadContinuation, SendError, Work};

/// Export the single versioned C entry point from a standalone viewer cdylib.
#[macro_export]
macro_rules! export_viewer {
    ($viewer:ty) => {
        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        /// # Safety
        /// The caller supplies aligned writable ABI memory covering `capacity`.
        pub unsafe extern "C" fn midnight_plugin_entry_v1(
            output: *mut $crate::api::abi::PluginApi,
            capacity: u32,
        ) -> $crate::api::abi::Status {
            // SAFETY: forwarding the entry point's caller-owned memory contract.
            unsafe { $crate::__entry::<$viewer>(output, capacity) }
        }
    };
}
