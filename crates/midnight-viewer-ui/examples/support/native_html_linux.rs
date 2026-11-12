//! Linux qualification uses the same installed content and directory contracts as the app.
use midnight_core::{Cancellation, TransferService};
use std::sync::{Arc, atomic::Ordering};

pub(super) fn service(handle: tokio::runtime::Handle) -> midnight_viewer_engine::Service {
    let filesystem = midnight_native_filesystem_host::InstalledProvider::shared(None);
    let directories = midnight_platform_linux::LinuxFileSystem::new(None);
    let capture: midnight_viewer_engine::NativeCapture = Arc::new(move |path, closed| {
        let main = filesystem
            .capture(path, &|| closed.load(Ordering::Acquire))
            .map_err(|error| format!("Installed native capture: {error}"))?;
        if closed.load(Ordering::Acquire) {
            return Err("Viewer closed".into());
        }
        // Authority comes from the original explicitly selected native fixture parent, never
        // CapturedFile.path(), a displayed name or a renderer-provided path.
        let parent = path.parent().ok_or("Fixture has no selected parent")?;
        let directory = directories
            .prepare_checksum_directory(parent, &Cancellation::default())
            .map_err(|error| format!("Fixture directory unavailable ({:?})", error.kind))?;
        let resources =
            midnight_viewer_engine::captured_directory_resources(directory, main.clone())
                .map_err(|error| error.to_string())?;
        Ok(main.with_resources(resources))
    });
    midnight_viewer_engine::Service::with_native_capture(handle, capture)
}
