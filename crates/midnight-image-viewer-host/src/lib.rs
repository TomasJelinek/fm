//! Backend-only discovery and supervision of persistent restricted image-viewer processes.
#![forbid(unsafe_code)]

pub use midnight_image_viewer_api::{DocumentMetadata, Frame, Metadata, Viewport};
mod providers;
pub use providers::{Discovery, Provider, discover, discover_with_diagnostics};
mod raster_reply;
mod session;
pub use session::{ImageSession, Source};

#[cfg(windows)]
#[path = "paths_windows.rs"]
mod paths;
#[cfg(target_os = "linux")]
#[path = "paths_linux.rs"]
mod paths;
