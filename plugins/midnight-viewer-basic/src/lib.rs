mod casefold;
mod decode;
mod io;
mod search;
mod viewer;
mod window;
midnight_plugin_sdk::export_viewer!(viewer::BasicViewer);

#[cfg(test)]
mod decode_tests;
#[cfg(test)]
mod search_tests;
