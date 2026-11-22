#![cfg(target_os = "linux")]
#![deny(unsafe_code, unsafe_op_in_unsafe_fn)]
//! Independent native Linux provider. No application/private platform crate dependencies.
mod adapter;
mod content;
mod mounts;
mod ownership;
mod rename;
mod rename_recovery;
#[allow(unsafe_code)]
mod rename_sys;
mod storage_kind;
#[allow(unsafe_code)]
mod system;
mod volume_details;
#[allow(unsafe_code)]
mod volume_information;
pub type Provider = midnight_filesystem_plugin_sdk::native::NativeFilesystem<adapter::Linux>;
use midnight_filesystem_plugin_api::abi::*;
// Storage queries report `StorageKind::Local` (a local disk of unknown media type),
// which hosts accept from ABI minor 4; plugin.toml requires the same minor.
midnight_filesystem_plugin_sdk::export_filesystem!(
    Provider,
    "native.linux",
    "0.2.0",
    CAP_BROWSE
        | CAP_READ
        | CAP_STAT
        | CAP_VOLUMES
        | CAP_STORAGE
        | CAP_NATIVE_ADDRESS
        | CAP_VOLUME_DETAILS
        | CAP_PREPARED_RENAME,
    minimum_host_minor = LOCAL_STORAGE_HOST_MINOR
);

#[cfg(test)]
extern crate self as midnight_filesystem_native_linux;
#[cfg(test)]
mod link_kind_tests;
#[cfg(test)]
mod rename_stage_tests;
#[cfg(test)]
mod rename_test_hooks;
#[cfg(test)]
#[allow(unsafe_code)]
#[path = "../tests/support/mod.rs"]
mod rename_test_support;
#[cfg(test)]
mod rename_tests;
