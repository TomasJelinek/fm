//! Whether a typed Copy/Move destination is an existing folder, asked before the
//! dialog admits a job. Advisory and read-only: the transfer checks it again.
use crate::{Cancellation, DesktopRequestId, FsError};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct DestinationCheckRequest {
    pub request: DesktopRequestId,
    pub path: PathBuf,
    pub cancel: Cancellation,
}

#[derive(Clone, Debug)]
pub struct DestinationCheckResult {
    pub request: DesktopRequestId,
    pub path: PathBuf,
    /// `Ok(true)`: an existing folder, links followed. `Ok(false)`: something else
    /// exists there. A missing path is an error of kind `NotFound`.
    pub result: Result<bool, FsError>,
}
