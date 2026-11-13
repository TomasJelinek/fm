#![cfg(windows)]
#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]
//! Independent native Windows Browse/Read/Rename provider. No application crate dependencies.
mod adapter;
mod content;
mod path;
mod rename;
#[allow(unsafe_code)]
mod rename_native;
mod rename_recovery;
#[cfg(test)]
#[allow(unsafe_code)]
#[path = "../tests/support/mod.rs"]
mod test_support;

#[cfg(test)]
extern crate self as midnight_filesystem_native_windows;
#[allow(unsafe_code)]
mod storage_kind;
#[allow(unsafe_code)]
mod system;
#[allow(unsafe_code)]
mod volume_details;
#[allow(unsafe_code)]
mod volume_information;

pub type Provider = midnight_filesystem_plugin_sdk::native::NativeFilesystem<adapter::Windows>;
use midnight_filesystem_plugin_api::abi::*;
midnight_filesystem_plugin_sdk::export_filesystem!(
    Provider,
    "native.windows",
    "0.2.0",
    CAP_BROWSE
        | CAP_READ
        | CAP_STAT
        | CAP_VOLUMES
        | CAP_STORAGE
        | CAP_NATIVE_ADDRESS
        | CAP_VOLUME_DETAILS
        | CAP_PREPARED_RENAME
);
