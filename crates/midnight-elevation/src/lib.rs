//! A bounded, authenticated, one-shot elevated directory reader.
#![deny(unsafe_op_in_unsafe_fn)]

mod helper;
#[cfg(windows)]
#[path = "windows.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod native;
mod protocol;

use midnight_core::{Cancellation, ElevatedDirectoryService, Entry, EntryId, EntryKind, FsError};
use std::{
    ffi::OsString,
    io,
    path::{Component, Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

/// Stateless parent adapter. Installation and channel I/O happen only in the listing worker.
#[derive(Default)]
pub struct ElevatedDirectories;

impl ElevatedDirectoryService for ElevatedDirectories {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        read(path, cancel, emit)
            .map_err(|error| FsError::from_io("authorized directory listing", path, error))
    }
}
fn read(path: &Path, cancel: &Cancellation, emit: &mut dyn FnMut(Entry) -> bool) -> io::Result<()> {
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    if !path.is_absolute() {
        return Err(protocol::invalid("Directory path must be absolute"));
    }
    let helper = trusted_helper()?;
    let request = protocol::Request {
        path: native::encode_path(path),
    };
    let reply = native::list(&helper, &request, cancel)?;
    let entries = reply.map_err(|error| {
        io::Error::new(
            if error.denied {
                io::ErrorKind::PermissionDenied
            } else {
                io::ErrorKind::Other
            },
            error.message,
        )
    })?;
    for (index, wire) in entries.into_iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let name = native::decode_path(&wire.name)?;
        let mut components = name.components();
        if !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
            || name.as_os_str().is_empty()
            || name.file_name() != Some(name.as_os_str())
        {
            return Err(protocol::invalid("Helper returned a non-basename entry"));
        }
        let kind = match wire.kind {
            0 => EntryKind::Directory,
            1 => EntryKind::DirectoryLink,
            2 => EntryKind::File,
            3 => EntryKind::Symlink,
            4 => EntryKind::Other,
            _ => return Err(protocol::invalid("Helper returned an unknown entry kind")),
        };
        let mut entry = Entry::new(EntryId(index as u64 + 1), path.join(name), kind);
        entry.kind = kind;
        entry.metadata.size = wire.size;
        entry.metadata.modified = wire.modified.and_then(|(seconds, nanos)| {
            if seconds >= 0 {
                UNIX_EPOCH.checked_add(Duration::new(seconds as u64, nanos))
            } else {
                UNIX_EPOCH
                    .checked_sub(Duration::from_secs(seconds.unsigned_abs()))
                    .and_then(|time| time.checked_add(Duration::from_nanos(u64::from(nanos))))
            }
        });
        entry.metadata.native_attributes = wire.native_attributes;
        entry.metadata.unix_mode = wire.unix_mode;
        if !emit(entry) {
            return Err(cancelled());
        }
    }
    Ok(())
}
const HELPER: &str = if cfg!(windows) {
    "midnight-elevated-listing.exe"
} else {
    "midnight-elevated-listing"
};

/// Locate the helper beside the installed executable and accept it only from a protected
/// location. It runs as Administrator/root, so the plugin policy (which also trusts the
/// current user) is not sufficient: the unelevated user must not be able to replace it.
fn trusted_helper() -> io::Result<PathBuf> {
    let executable = midnight_plugin_host::installed_executable().map_err(io::Error::other)?;
    let parent = executable
        .parent()
        .ok_or_else(|| protocol::invalid("Missing installation"))?;
    let helper = parent.join(HELPER);
    for component in helper.ancestors() {
        let metadata = match std::fs::symlink_metadata(component) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound && component == helper => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!(
                        "The authorization helper {HELPER} is not installed beside Midnight \
                         Explorer in {}",
                        parent.display()
                    ),
                ));
            }
            Err(error) => return Err(error),
        };
        // Canonicalization below must not resolve a link. Any other reparse point is judged
        // by the platform check on pinned handles (Windows accepts only storage-only tags).
        if metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Authorization helper path contains a redirected component",
            ));
        }
    }
    if !std::fs::metadata(&helper)?.is_file() {
        return Err(protocol::invalid(
            "Authorization helper is not a regular file",
        ));
    }
    let helper = helper.canonicalize()?;
    native::protected_helper(&helper)?;
    Ok(helper)
}
pub(crate) fn cancelled() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "Authorized directory listing cancelled",
    )
}
/// Dedicated helper entry point; startup arguments identify only the authenticated local channel.
#[doc(hidden)]
pub fn serve(args: Vec<OsString>) -> io::Result<()> {
    native::serve(&args)
}
