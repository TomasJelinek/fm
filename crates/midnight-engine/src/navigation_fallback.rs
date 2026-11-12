//! Native address recovery, invoked inside the existing bounded listing worker.
use midnight_core::{Cancellation, FsError, NavigationFallbackService};
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

pub(super) fn next_path(
    path: &Path,
    error: &FsError,
    service: Option<&dyn NavigationFallbackService>,
    cancel: &Cancellation,
) -> Result<Option<PathBuf>, FsError> {
    let Some(service) = service else {
        return Ok(None);
    };
    if cancel.is_cancelled() || !path.is_absolute() || !recoverable(error.kind) {
        return Ok(None);
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| parent.is_absolute() && *parent != path)
    {
        return Ok(Some(parent.to_owned()));
    }
    let root = service.navigation_fallback_root()?;
    if cancel.is_cancelled() || root == path {
        return Ok(None);
    }
    // The service cannot cause a cycle by returning an arbitrary directory.
    if !root.is_absolute() || root.parent().is_some() {
        return Err(FsError::from_io(
            "resolve navigation fallback",
            &root,
            ErrorKind::InvalidData.into(),
        ));
    }
    Ok(Some(root))
}

fn recoverable(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::NotFound
            | ErrorKind::NotADirectory
            | ErrorKind::PermissionDenied
            | ErrorKind::NotConnected
            | ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::HostUnreachable
            | ErrorKind::NetworkUnreachable
            | ErrorKind::NetworkDown
            | ErrorKind::TimedOut
            | ErrorKind::Other
    )
}
