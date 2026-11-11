//! Backend-only scheduling, broker, and persistent cache for bounded thumbnail workers.
#![forbid(unsafe_code)]

pub mod archive_source;
pub mod cache;
mod cache_store;
mod catalog;
pub use catalog::catalog;
pub mod decode;
#[cfg(windows)]
#[path = "paths_windows.rs"]
mod paths;
#[cfg(target_os = "linux")]
#[path = "paths_linux.rs"]
mod paths;
mod providers;
pub mod scheduler;
pub mod service;

#[cfg(test)]
mod archive_source_tests;
