#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]
//! Independent native macOS provider; only public API/SDK and OS bindings are shared.

#[cfg(any(target_os = "macos", test))]
mod policy;

#[cfg(target_os = "macos")]
mod adapter;
#[cfg(target_os = "macos")]
mod content;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod directory;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod system;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod volumes;

#[cfg(target_os = "macos")]
pub type Provider = midnight_filesystem_plugin_sdk::native::NativeFilesystem<adapter::MacOs>;
#[cfg(target_os = "macos")]
use midnight_filesystem_plugin_api::abi::*;
#[cfg(target_os = "macos")]
midnight_filesystem_plugin_sdk::export_filesystem!(
    Provider,
    "native.macos",
    "0.1.0",
    CAP_BROWSE
        | CAP_READ
        | CAP_STAT
        | CAP_VOLUMES
        | CAP_STORAGE
        | CAP_NATIVE_ADDRESS
        | CAP_VOLUME_DETAILS
);
