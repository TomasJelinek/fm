//! Compose explicit image selection, restricted decoding and native managed storage.
//! All callbacks execute within the engine's bounded path-picker worker.
use midnight_core::{Cancellation, FsError, PathPickerKind, PathPickerRequest, PathPickerService};
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
type Decode = Arc<dyn Fn(&Path, &Cancellation) -> io::Result<Pixels> + Send + Sync>;
struct Picker {
    native: Arc<dyn PathPickerService>,
    decode: Decode,
}

pub(crate) fn installed(
    native: Arc<dyn PathPickerService>,
    filesystem: Arc<midnight_native_filesystem_host::InstalledProvider>,
) -> Arc<dyn PathPickerService> {
    Arc::new(Picker {
        native,
        decode: Arc::new(move |path, cancel| {
            let source = filesystem
                .capture(path, &|| cancel.is_cancelled())
                .map_err(io::Error::other)?;
            let executable = std::env::current_exe()?;
            let extension = path
                .extension()
                .and_then(|suffix| suffix.to_str())
                .unwrap_or("");
            let frame = midnight_thumbnail_host::artwork::decode_image(
                &executable,
                &source,
                extension,
                &|| cancel.is_cancelled(),
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
        let import = request.kind == PathPickerKind::CommandIconImage
            || (request.kind == PathPickerKind::CommandIconResource
                && self.native.command_icon_resource_requires_import());
        if !import {
            return self.native.pick_path(request, cancel);
        }
        let mut selection = request.clone();
        if selection.kind == PathPickerKind::CommandIconImage {
            selection.kind = PathPickerKind::Image;
        }
        let Some(path) = self.native.pick_path(&selection, cancel)? else {
            return Ok(None);
        };
        if cancel.is_cancelled() {
            return Ok(None);
        }
        midnight_core::validate_picked_path(&path)?;
        let pixels = (self.decode)(&path, cancel)
            .map_err(|error| FsError::from_io("import command icon", &path, error))?;
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let stored =
            self.native
                .store_command_icon(pixels.width, pixels.height, &pixels.rgba, cancel)?;
        if cancel.is_cancelled() {
            return Ok(None);
        }
        midnight_core::validate_picked_path(&stored)?;
        Ok(Some(stored))
    }
}

#[cfg(test)]
#[path = "command_artwork_tests.rs"]
mod tests;
