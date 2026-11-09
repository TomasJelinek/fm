#![cfg(target_os = "linux")]
#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]
//! Independent native Linux provider. No application/private platform crate dependencies.
mod adapter;
mod content;
mod mounts;
mod storage_kind;
#[allow(unsafe_code)]
mod system;
mod volume_details;
#[allow(unsafe_code)]
mod volume_information;
pub type Provider = midnight_filesystem_plugin_sdk::native::NativeFilesystem<adapter::Linux>;
use midnight_filesystem_plugin_api::abi::*;
midnight_filesystem_plugin_sdk::export_filesystem!(
    Provider,
    "native.linux",
    "0.1.0",
    CAP_BROWSE
        | CAP_READ
        | CAP_STAT
        | CAP_VOLUMES
        | CAP_STORAGE
        | CAP_NATIVE_ADDRESS
        | CAP_VOLUME_DETAILS
);
