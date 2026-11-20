//! Backend authority and supervision for optional interactive browser providers.
#![forbid(unsafe_code)]

pub use midnight_web_viewer_api as api;
pub use midnight_worker_platform::browser::SandboxRefusal;
mod providers;
mod resource_jobs;
mod resources;
mod session;
pub use providers::{Discovery, Provider, discover_with_diagnostics};
pub use resources::{ResourceCancellation, ResourcePathPolicy, Resources};
pub use session::{PolicyCeiling, Session};
