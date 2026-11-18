//! Shared application content authority. Factories defer all provider work to backend jobs.
use midnight_native_filesystem_host::InstalledProvider;
use midnight_thumbnail_host::service;
use std::{
    collections::VecDeque,
    io,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    sync::{Arc, Mutex},
};

// Thumbnails and comparison need only the borrowed main-content cancellation flag.
pub(crate) type Capture = midnight_compare_engine::NativeCapture;

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

/// Metadata-only external-compare capture through the same installed cache.
/// The Engine front door arms each successful return before checking cancellation.
pub(crate) fn external_comparison_factory(
    filesystem: Arc<InstalledProvider>,
) -> midnight_engine::ExternalCompareNativeCapture {
    Arc::new(move |path, cancel| {
        filesystem
            .capture_checked(path, &|| cancel.is_cancelled())
            .map_err(|failure| external_capture_error(path, cancel, failure))
    })
}

fn external_capture_error(
    path: &Path,
    cancel: &midnight_core::Cancellation,
    failure: midnight_native_filesystem_host::CaptureFailure,
) -> midnight_engine::ExternalCompareCaptureError {
    let error = if cancel.is_cancelled() {
        io::ErrorKind::Interrupted.into()
    } else {
        io::Error::other(format!("Native filesystem: {}", failure.error))
    };
    midnight_engine::ExternalCompareCaptureError {
        error: midnight_core::FsError::from_io("capture comparison", path, error),
        cleanup_unconfirmed: failure.cleanup_unconfirmed,
    }
}

/// Viewer-only asset authority. Main content always comes from the installed filesystem plugin.
/// Parent directory pins are shared across prefetched panel files, at most sixteen cache entries.
pub(crate) fn factory_with_resources(
    filesystem: Arc<InstalledProvider>,
    directories: Arc<dyn midnight_core::TransferService>,
) -> midnight_viewer_engine::NativeCapture {
    type Directory = Arc<dyn midnight_core::TransferDirectory>;
    let cache = Mutex::new(VecDeque::<(PathBuf, Directory)>::new());
    Arc::new(move |path, cancel| {
        let main = filesystem
            .capture(path, &|| cancel.load(Ordering::Acquire))
            .map_err(|failure| format!("Native filesystem: {failure}"))?;
        if cancel.load(Ordering::Acquire) {
            return Err("Viewer closed".into());
        }
        let Some(parent) = path.parent() else {
            return Ok(main);
        };
        // This is the original selected native parent, never CapturedFile.path() or a display name.
        let cached = cache.lock().ok().and_then(|entries| {
            entries
                .iter()
                .find(|(key, _)| key == parent)
                .map(|(_, directory)| directory.clone())
        });
        let cancelled = midnight_core::Cancellation::observing(cancel.clone());
        let directory = if let Some(directory) = cached
            && directory.revalidate(&cancelled).is_ok()
        {
            Some(directory)
        } else {
            let prepared = directories
                .prepare_checksum_directory(parent, &cancelled)
                .ok();
            if let Some(directory) = &prepared
                && let Ok(mut entries) = cache.lock()
            {
                entries.retain(|(key, _)| key != parent);
                entries.push_back((parent.to_path_buf(), directory.clone()));
                while entries.len() > 16 {
                    entries.pop_front();
                }
            }
            prepared
        };
        if cancel.load(Ordering::Acquire) {
            return Err("Viewer closed".into());
        }
        match directory.and_then(|directory| {
            midnight_viewer_engine::captured_directory_resources(directory, main.clone()).ok()
        }) {
            Some(resources) => Ok(main.with_resources(resources)),
            // A denied/unavailable scope never gains a native path fallback. The main document
            // remains readable; the web broker reports unavailable relative resources on lookup.
            None => Ok(main),
        }
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

#[cfg(test)]
#[path = "native_external_compare_tests.rs"]
mod external_compare_tests;
