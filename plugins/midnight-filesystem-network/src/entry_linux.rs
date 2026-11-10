use crate::{NetworkFilesystem, linux::Linux};
use midnight_filesystem_plugin_api::abi::*;
midnight_filesystem_plugin_sdk::export_filesystem!(
    NetworkFilesystem<Linux>,
    "network.linux",
    "0.1.0",
    CAP_BROWSE | CAP_STAT | CAP_VOLUMES | CAP_ACTIVATE | CAP_REFRESH | CAP_RESOLVE_TEXT
);
