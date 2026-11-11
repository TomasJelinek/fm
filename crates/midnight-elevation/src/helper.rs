//! The privileged process reads one directory. It has no plugin or shell entry point.
use crate::{
    native,
    protocol::{self, Failure, Reply, Request, WireEntry},
};
use midnight_core::Cancellation;
use std::{fs, io, time::UNIX_EPOCH};

pub(crate) fn reply(request: Request, cancel: &Cancellation) -> Reply {
    collect(request, cancel).map_err(|error| Failure {
        denied: error.kind() == io::ErrorKind::PermissionDenied,
        message: error.to_string().chars().take(2048).collect(),
    })
}
fn collect(request: Request, cancel: &Cancellation) -> io::Result<Vec<WireEntry>> {
    if cancel.is_cancelled() {
        return Err(crate::cancelled());
    }
    let path = native::decode_path(&request.path)?;
    if !path.is_absolute() {
        return Err(protocol::invalid("Listing path must be absolute"));
    }
    let mut entries = Vec::new();
    let mut total_bytes = 0usize;
    for entry in fs::read_dir(&path)? {
        if cancel.is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Listing cancelled",
            ));
        }
        let entry = entry?;
        let file_type = entry.file_type()?;
        let metadata = fs::symlink_metadata(entry.path()).ok();
        let kind = if file_type.is_symlink() {
            if fs::metadata(entry.path()).is_ok_and(|value| value.is_dir()) {
                1
            } else {
                3
            }
        } else if file_type.is_dir() {
            0
        } else if file_type.is_file() {
            2
        } else {
            4
        };
        let name = native::encode_path(std::path::Path::new(&entry.file_name()));
        total_bytes = total_bytes.saturating_add(name.len() + 64);
        if entries.len() >= protocol::MAX_ENTRIES || total_bytes > protocol::MAX_FRAME - 1024 {
            return Err(protocol::invalid(
                "Directory exceeds elevated listing limits",
            ));
        }
        let modified = metadata
            .as_ref()
            .and_then(|value| value.modified().ok())
            .and_then(|time| match time.duration_since(UNIX_EPOCH) {
                Ok(value) => i64::try_from(value.as_secs())
                    .ok()
                    .map(|seconds| (seconds, value.subsec_nanos())),
                Err(error) => {
                    let value = error.duration();
                    let seconds = i64::try_from(value.as_secs()).ok()?;
                    if value.subsec_nanos() == 0 {
                        Some((-seconds, 0))
                    } else {
                        seconds
                            .checked_add(1)
                            .map(|seconds| (-seconds, 1_000_000_000 - value.subsec_nanos()))
                    }
                }
            });
        let (native_attributes, unix_mode) =
            metadata.as_ref().map(native::metadata).unwrap_or_default();
        entries.push(WireEntry {
            name,
            kind,
            size: metadata
                .as_ref()
                .filter(|_| kind == 2)
                .map(fs::Metadata::len),
            modified,
            native_attributes,
            unix_mode,
        });
    }
    Ok(entries)
}
