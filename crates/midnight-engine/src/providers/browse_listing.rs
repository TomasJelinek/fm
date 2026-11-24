//! Browse-only listing of a Windows directory link whose own listing is denied.
//!
//! Windows' compatibility junctions (`My Music` in Documents, `Application Data`,
//! `Documents and Settings` and others) carry an explicit "Everyone: deny list folder"
//! entry on the link itself, so that programs which walk links blindly neither loop nor
//! count a folder twice. Their targets stay readable. Explicit navigation already opens
//! such a link's target (`OpenService::directory_destination`); this lists the target for
//! the inline tree's disclosure probes, expansions, refreshes and watch probes, with its
//! entries addressed below the link, exactly as an ordinary junction lists. Recursive
//! walks (sizes, Find, Compare, Copy/Move) keep their own link rules and never use it.
use midnight_core::{Cancellation, Entry, FileSystem, FsError};
use std::{io, path::Path};

/// Lists `path` for browsing. When `path` is a directory link (`link`) whose listing
/// Windows denies (`PermissionDenied`) before it produced any entry, lists the link's
/// OS-resolved target instead (`resolve_size_directory`: the final path of an actual
/// directory) and gives each entry its path below `path`. A link whose target cannot be
/// resolved keeps its own denial; a target that cannot be listed reports its own error,
/// so a link to a denied folder still fails. Other platforms, ordinary folders and every
/// other failure list `path` only.
pub(crate) fn read_browse_directory(
    filesystem: &dyn FileSystem,
    path: &Path,
    link: bool,
    cancel: &Cancellation,
    emit: &mut dyn FnMut(Entry) -> bool,
) -> Result<(), FsError> {
    let mut emitted = false;
    let own = filesystem.read_directory(path, cancel, &mut |entry| {
        emitted = true;
        emit(entry)
    });
    let denied = match own {
        Err(error)
            if cfg!(windows)
                && link
                && !emitted
                && error.kind == io::ErrorKind::PermissionDenied
                && !cancel.is_cancelled() =>
        {
            error
        }
        result => return result,
    };
    let Ok(Some(target)) = filesystem.resolve_size_directory(path, cancel) else {
        return Err(denied);
    };
    let target = target.path;
    let mut invalid = false;
    filesystem.read_directory(&target, cancel, &mut |mut entry| {
        // Only an immediate native child of the target can be readdressed below the link.
        if entry.virtual_node.is_some()
            || entry.network_node.is_some()
            || entry.path.parent() != Some(target.as_path())
            || entry.path.file_name() != Some(entry.name.as_os_str())
        {
            invalid = true;
            return false;
        }
        entry.path = path.join(&entry.name);
        emit(entry)
    })?;
    if invalid {
        return Err(FsError::from_io(
            "list directory",
            &target,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "listed entry outside its directory link target",
            ),
        ));
    }
    Ok(())
}

#[cfg(all(test, windows))]
#[path = "browse_listing_tests.rs"]
pub(crate) mod tests;
