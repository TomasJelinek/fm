//! Explicit custom-command artwork uses the installed restricted image worker.
use crate::{decode, providers};
use midnight_core::ThumbnailerAssociations;
use midnight_thumbnail_api::{Frame, Request};
use std::{io, path::Path, time::Duration};

/// Decode a static image using trusted installed code. Call on a bounded backend
/// worker; content is a captured read-only source, never an authority-bearing path.
pub fn decode_image(
    executable: &Path,
    source: &dyn decode::Source,
    extension: &str,
    associations: &ThumbnailerAssociations,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> io::Result<Frame> {
    if cancelled() {
        return Err(io::ErrorKind::Interrupted.into());
    }
    let catalog = providers::discover(executable)?;
    let provider = providers::select(&catalog, associations, extension, true)
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported,
            "No installed image thumbnailer supports this artwork. Install an image thumbnailer or choose another image format."))?;
    decode::decode_cooperative(
        &provider.executable,
        source,
        Request {
            id: 1,
            source_length: source.length(),
            width: 128,
            height: 128,
            frame: 0,
        },
        cancelled,
        Duration::from_secs(10),
    )
}
