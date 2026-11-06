pub mod decode;
pub mod markdown;
mod paging;
mod plugin;
mod search;
midnight_plugin_sdk::export_viewer!(plugin::Plugin);
