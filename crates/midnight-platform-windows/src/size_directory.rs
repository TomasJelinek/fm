//! Read-only junction resolution and directory identities for cycle-safe size scans.
use crate::mutation::native;
use midnight_core::{Cancellation, FsError, SizeDirectoryIdentity, SizeDirectoryTarget};
use std::{
    fs::{self, OpenOptions},
    hash::{BuildHasher, RandomState},
    io,
    os::windows::fs::OpenOptionsExt,
    path::Path,
    sync::OnceLock,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE,
};

pub(super) fn resolve(
    path: &Path,
    cancel: &Cancellation,
) -> Result<Option<SizeDirectoryTarget>, FsError> {
    let result = (|| {
        if cancel.is_cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        // canonicalize uses an OS final-path query and opens no contents. In
        // particular it resolves legacy junctions before directory enumeration.
        let resolved = fs::canonicalize(path)?;
        let file = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&resolved)?;
        if !file.metadata()?.is_dir() {
            return Ok(None);
        }
        let identity = identity(&file, &resolved)?;
        if cancel.is_cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        Ok(Some(SizeDirectoryTarget {
            path: resolved,
            identity,
        }))
    })();
    result.map_err(|error| FsError::from_io("resolve directory size target", path, error))
}

/// The directory's identity for cycle detection: its 128-bit file ID, or where the file
/// system has none its 64-bit file index (`local_identity`: FAT32, exFAT, CD/DVD,
/// WebDAV, SMB 2). Where that is all zero too (the WebDAV redirector and some SMB
/// servers report index 0 for every entry) it would make every folder repeat its
/// ancestor, so the canonical path stands in: a link reached again through another name
/// still resolves to the same final path, and distinct folders stay distinct.
fn identity(file: &fs::File, resolved: &Path) -> io::Result<SizeDirectoryIdentity> {
    let identity = match native::local_identity(file) {
        Ok(identity) => Some(identity),
        Err(error) if native::unsupported(&error) => None,
        Err(error) => return Err(error),
    };
    Ok(choose_identity(identity, resolved))
}

fn choose_identity(identity: Option<native::Identity>, resolved: &Path) -> SizeDirectoryIdentity {
    match identity {
        Some(identity) if identity.is_known() => SizeDirectoryIdentity {
            device: identity.volume,
            object: identity.id,
        },
        identity => path_identity(identity.map(|identity| identity.volume), resolved),
    }
}

/// A process-local identity derived from the canonical path, with random keys so that
/// no chosen name collides with another; the device is the volume serial number where
/// one was reported.
fn path_identity(volume: Option<u64>, resolved: &Path) -> SizeDirectoryIdentity {
    static KEYS: OnceLock<[RandomState; 3]> = OnceLock::new();
    let keys = KEYS.get_or_init(|| std::array::from_fn(|_| RandomState::new()));
    let mut object = [0u8; 16];
    object[..8].copy_from_slice(&keys[0].hash_one(resolved).to_le_bytes());
    object[8..].copy_from_slice(&keys[1].hash_one(resolved).to_le_bytes());
    SizeDirectoryIdentity {
        device: volume.unwrap_or_else(|| keys[2].hash_one(resolved.components().next())),
        object,
    }
}

#[cfg(test)]
#[path = "size_directory_tests.rs"]
mod tests;
