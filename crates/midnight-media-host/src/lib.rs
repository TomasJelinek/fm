//! Backend playback, trusted media installation and restricted decoder transport.
#![forbid(unsafe_code)]

mod playback;
pub mod system_decoders;
pub mod video_installation;
pub mod video_process;
pub mod video_session;
pub mod video_transport;
mod volume;

pub use midnight_worker_platform::{CpuBudget, ResourcePolicy};
pub use playback::*;
pub use volume::{AudioVolume, MAX_VOLUME_PERCENT};
