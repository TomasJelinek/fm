//! Shared application content authority. Factories defer all provider work to backend jobs.
use midnight_native_filesystem_host::InstalledProvider;
use midnight_thumbnail_host::service;
use std::{io, path::PathBuf, sync::Arc, sync::atomic::Ordering};

pub(crate) type Capture = midnight_viewer_engine::NativeCapture;

pub(crate) fn archive(filesystem: Arc<InstalledProvider>) -> midnight_engine::ArchiveCapture {
    Arc::new(move |path, expected, cancel| {
        filesystem
            .capture_matching(path, expected, &|| cancel.is_cancelled())
            .map_err(|failure| {
                let error = if cancel.is_cancelled() {
                    io::ErrorKind::Interrupted.into()
                } else {
                    io::Error::other(format!("Native filesystem: {failure}"))
                };
                midnight_core::FsError::from_io("capture archive", path, error)
            })
    })
}

pub(crate) fn factory(filesystem: Arc<InstalledProvider>) -> Capture {
    Arc::new(move |path, cancel| {
        filesystem
            .capture(path, &|| cancel.load(Ordering::Acquire))
            .map_err(|failure| format!("Native filesystem: {failure}"))
    })
}

pub(crate) fn thumbnail(capture: Capture, path: PathBuf, confirmed_ssd: bool) -> service::Capture {
    Arc::new(move |cancel| {
        let file = capture(&path, cancel).map_err(|message| {
            if cancel.load(Ordering::Acquire) {
                io::ErrorKind::Interrupted.into()
            } else {
                io::Error::other(message)
            }
        })?;
        Ok(service::captured_source(file, confirmed_ssd))
    })
}

#[cfg(test)]
#[path = "native_content_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "native_archive_tests.rs"]
mod archive_tests;
