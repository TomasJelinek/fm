//! Compose explicit image selection, restricted decoding and native managed storage.
//! All callbacks execute within the engine's bounded path-picker worker.
use crate::thumbnails::Routing;
use midnight_core::{
    Cancellation, FsError, PathPickerKind, PathPickerRequest, PathPickerService,
    ThumbnailerAssociations,
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

struct Pixels {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}
type Decode = Arc<
    dyn Fn(&Path, &ThumbnailerAssociations, &(dyn Fn() -> bool + Sync)) -> io::Result<Pixels>
        + Send
        + Sync,
>;
struct Picker {
    native: Arc<dyn PathPickerService>,
    decode: Decode,
    routing: Arc<Routing>,
}

fn selected_image_target(path: &Path) -> io::Result<PathBuf> {
    // Theme icons commonly link to shared artwork. Resolve only this explicit
    // selection; the provider still captures a regular file without following links.
    path.canonicalize()
}

pub(crate) fn requires_import(kind: PathPickerKind, import_resource: bool) -> bool {
    kind == PathPickerKind::CommandIconImage
        || (import_resource && kind == PathPickerKind::CommandIconResource)
}

pub(crate) fn installed(
    native: Arc<dyn PathPickerService>,
    filesystem: Arc<midnight_native_filesystem_host::InstalledProvider>,
    routing: Arc<Routing>,
) -> Arc<dyn PathPickerService> {
    with_installation(native, filesystem, routing, || {
        midnight_plugin_host::installed_executable().map_err(io::Error::other)
    })
}

/// Thumbnailers are discovered relative to the resolved plugin installation, like every
/// other host, never relative to the running executable.
fn with_installation(
    native: Arc<dyn PathPickerService>,
    filesystem: Arc<midnight_native_filesystem_host::InstalledProvider>,
    routing: Arc<Routing>,
    installation: impl Fn() -> io::Result<PathBuf> + Send + Sync + 'static,
) -> Arc<dyn PathPickerService> {
    Arc::new(Picker {
        native,
        routing,
        decode: Arc::new(move |path, associations, cancelled| {
            let executable = installation()?;
            let target = selected_image_target(path)?;
            let source = filesystem
                .capture(&target, cancelled)
                .map_err(io::Error::other)?;
            let extension = path
                .extension()
                .and_then(|suffix| suffix.to_str())
                .unwrap_or("");
            let frame = midnight_thumbnail_host::artwork::decode_image(
                &executable,
                &source,
                extension,
                associations,
                cancelled,
            )?;
            Ok(Pixels {
                width: frame.width,
                height: frame.height,
                rgba: frame.rgba,
            })
        }),
    })
}

impl PathPickerService for Picker {
    fn pick_path(
        &self,
        request: &PathPickerRequest,
        cancel: &Cancellation,
    ) -> Result<Option<PathBuf>, FsError> {
        if !requires_import(
            request.kind,
            self.native.command_icon_resource_requires_import(),
        ) {
            return self.native.pick_path(request, cancel);
        }
        let snapshot = self.routing.snapshot();
        let cancelled = || cancel.is_cancelled() || !self.routing.is_current(snapshot.generation);
        let mut selection = request.clone();
        if selection.kind == PathPickerKind::CommandIconImage {
            selection.kind = PathPickerKind::Image;
        }
        let Some(path) = self.native.pick_path(&selection, cancel)? else {
            return Ok(None);
        };
        if cancelled() {
            return Ok(None);
        }
        midnight_core::validate_picked_path(&path)?;
        let pixels = (self.decode)(&path, &snapshot.associations, &cancelled);
        if cancelled() {
            return Ok(None);
        }
        let pixels =
            pixels.map_err(|error| FsError::from_io("import command icon", &path, error))?;
        let stored =
            self.native
                .store_command_icon(pixels.width, pixels.height, &pixels.rgba, cancel);
        if cancelled() {
            return Ok(None);
        }
        let stored = stored?;
        midnight_core::validate_picked_path(&stored)?;
        Ok(Some(stored))
    }
}

#[cfg(test)]
#[path = "command_artwork_tests.rs"]
mod tests;

#[cfg(all(test, target_os = "linux"))]
#[path = "command_artwork_linux_tests.rs"]
mod linux_tests;
