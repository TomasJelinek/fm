//! Backend authority and supervision for optional interactive browser providers.
#![forbid(unsafe_code)]

pub use midnight_web_viewer_api as api;
mod providers;
mod resources;
mod session;
pub use providers::{Discovery, Provider, discover_with_diagnostics};
pub use resources::Resources;
pub use session::Session;
