//! Explicitly selected PNG/SVG artwork becomes an independent managed snapshot.
use midnight_core::Cancellation;
use std::{
    io,
    path::{Path, PathBuf},
};
#[path = "icon_svg.rs"]
mod svg;

pub(crate) fn snapshot(
    path: &Path,
    home: Option<&Path>,
    cancel: &Cancellation,
) -> io::Result<Option<PathBuf>> {
    if cancel.is_cancelled() {
        return Ok(None);
    }
    midnight_core::validate_picked_path(path).map_err(io::Error::other)?;
    let bytes = crate::desktop::read_bounded(path, 1024 * 1024).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "The selected icon is not a readable bounded image file.",
        )
    })?;
    if cancel.is_cancelled() {
        return Ok(None);
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        // Binary decoding remains in the app's restricted thumbnail broker wrapper.
        return Ok(Some(path.to_path_buf()));
    }
    let pixels = svg::decode(&bytes, 128, cancel).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Choose a PNG or a simple SVG icon without external resources, filters, masks or patterns.",
        )
    })?;
    if cancel.is_cancelled() {
        return Ok(None);
    }
    let path = crate::icon_storage::save(home, &pixels, cancel)?;
    Ok((!cancel.is_cancelled()).then_some(path))
}
