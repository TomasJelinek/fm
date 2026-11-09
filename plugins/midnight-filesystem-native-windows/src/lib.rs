#![cfg(windows)]
#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]
//! Independent native Windows Browse/Read provider. No application crate dependencies.
mod adapter;
mod content;
mod path;
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
    "0.1.0",
    CAP_BROWSE
        | CAP_READ
        | CAP_STAT
        | CAP_VOLUMES
        | CAP_STORAGE
        | CAP_NATIVE_ADDRESS
        | CAP_VOLUME_DETAILS
);
