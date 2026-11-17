//! Shared ownership rules for private staging directories and protected parents.
//!
//! Private staging creates a directory with mode 0700 and pins it without following
//! links. Filesystems whose permissions come from mount options (vfat, exfat, ntfs3
//! or ntfs-3g/fuseblk, CIFS without Unix extensions, GVfs and other FUSE mounts)
//! cannot store that mode: an explicit `fchmod(0700)` of our own new directory keeps
//! the mount-provided mode. Such a directory is accepted with that mode, so its
//! protection against other users is exactly the mount's. Ownership by the effective
//! UID is always required, and a creator removes a directory it then refuses.
//!
//! A protected parent prevents other users from replacing or removing our names:
//! it is owned by the effective UID or root and is either sticky or not writable by
//! other users. Group write is accepted only for the caller's user-private group:
//! the effective GID, named like the effective user in the local account database,
//! with no other listed member and no other local account using it as its primary
//! group. Shared groups and world-writable non-sticky parents remain refused.
//! Accounts outside `/etc/passwd`/`/etc/group` (directory services) keep the strict
//! rule. Same-UID and root interference remain outside this boundary.

use rustix::fs::{AtFlags, FileType, Mode, OFlags, fchmod, fstat, openat, statat, unlinkat};
use std::{
    ffi::OsStr,
    fs::File,
    io::{self, Read},
    os::fd::AsFd,
    sync::Mutex,
    time::{Duration, Instant},
};

/// How a directory created by this process with mode 0700 may be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CreatedDirectory {
    /// Owned by the effective UID with exactly mode 0700.
    Private,
    /// Owned by the effective UID on a filesystem that cannot store the requested
    /// mode; the value is the mount-provided permission bits.
    MountProvided(u32),
}

/// Classifies a directory this process just created with `mkdirat(..., 0o700)`
/// and opened without following links. Refuses a non-directory or foreign owner.
pub(crate) fn check_created_directory(directory: &File) -> io::Result<CreatedDirectory> {
    let mode = owned_directory_mode(directory)?;
    if mode == 0o700 {
        return Ok(CreatedDirectory::Private);
    }
    #[cfg(test)]
    if let Some(simulated) = simulation::mount_mode() {
        return Ok(CreatedDirectory::MountProvided(simulated));
    }
    // A restrictive umask or default ACL is corrected on our own new directory.
    // Mount-option permissions keep their mode (vfat/exfat silently, others with
    // an error), which identifies a filesystem that cannot represent it.
    let _ = set_private_mode(directory);
    let mode = owned_directory_mode(directory)?;
    Ok(if mode == 0o700 {
        CreatedDirectory::Private
    } else {
        CreatedDirectory::MountProvided(mode)
    })
}

fn owned_directory_mode(directory: &File) -> io::Result<u32> {
    let stat = fstat(directory)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the new private directory is not a directory",
        ));
    }
    #[cfg(test)]
    if simulation::foreign_owner() {
        return Err(foreign_owner());
    }
    if stat.st_uid != rustix::process::geteuid().as_raw() {
        return Err(foreign_owner());
    }
    #[cfg(test)]
    if let Some(simulated) = simulation::mount_mode() {
        return Ok(simulated);
    }
    Ok(stat.st_mode & 0o777)
}

fn foreign_owner() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "the new private directory is not owned by this user",
    )
}

fn set_private_mode(directory: &File) -> io::Result<()> {
    // The pinned handle may be O_PATH; reopen exactly that directory for fchmod.
    let readable = openat(
        directory,
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    fchmod(&readable, Mode::RWXU)?;
    Ok(())
}

/// Removes the empty directory this process created as `parent/name` and then
/// refused. `pinned` is its `(device, inode)` when it was opened; a replaced or
/// non-directory entry is left untouched, and rmdir never removes contents.
pub(crate) fn remove_refused_directory(
    parent: impl AsFd,
    name: &OsStr,
    pinned: Option<(u64, u64)>,
) -> io::Result<()> {
    let current = statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW)?;
    if FileType::from_raw_mode(current.st_mode) != FileType::Directory
        || pinned.is_some_and(|identity| identity != (current.st_dev, current.st_ino))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the refused directory was replaced and is left untouched",
        ));
    }
    unlinkat(&parent, name, AtFlags::REMOVEDIR)?;
    Ok(())
}

/// Whether other users can neither replace nor remove names in a directory with
/// this owner, group and mode (see the module documentation).
pub(crate) fn protected_parent(uid: u32, gid: u32, mode: u32) -> bool {
    let caller = rustix::process::geteuid().as_raw();
    if uid != caller && uid != 0 {
        return false;
    }
    if mode & 0o1000 != 0 {
        return true;
    }
    mode & 0o002 == 0 && (mode & 0o020 == 0 || caller_private_group(gid))
}

/// Whether only the caller can write to a directory owned by the effective UID:
/// never world-writable, and group-writable only for its user-private group.
pub(crate) fn writable_only_by_caller(uid: u32, gid: u32, mode: u32) -> bool {
    uid == rustix::process::geteuid().as_raw()
        && mode & 0o002 == 0
        && (mode & 0o020 == 0 || caller_private_group(gid))
}

/// Whether `gid` is the effective user's private group. Results are cached briefly
/// because recursive transfers check the same parents' group for every child.
pub(crate) fn caller_private_group(gid: u32) -> bool {
    const VALIDITY: Duration = Duration::from_secs(10);
    static CACHE: Mutex<Option<(u32, u32, Instant, bool)>> = Mutex::new(None);
    if gid != rustix::process::getegid().as_raw() {
        return false;
    }
    let uid = rustix::process::geteuid().as_raw();
    let mut cache = CACHE.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((cached_uid, cached_gid, checked, verdict)) = *cache
        && (cached_uid, cached_gid) == (uid, gid)
        && checked.elapsed() < VALIDITY
    {
        return verdict;
    }
    let verdict = match (
        read_account_file("/etc/passwd"),
        read_account_file("/etc/group"),
    ) {
        (Some(passwd), Some(group)) => private_group_in(&passwd, &group, uid, gid),
        _ => false,
    };
    *cache = Some((uid, gid, Instant::now(), verdict));
    verdict
}

fn read_account_file(path: &str) -> Option<Vec<u8>> {
    const MAX_ACCOUNT_FILE: u64 = 4 * 1024 * 1024;
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX_ACCOUNT_FILE + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_ACCOUNT_FILE).then_some(bytes)
}

/// The user-private-group rule over `passwd(5)`/`group(5)` content. Unknown,
/// ambiguous, malformed or NIS-compatibility entries never qualify a group.
pub(crate) fn private_group_in(passwd: &[u8], group: &[u8], uid: u32, gid: u32) -> bool {
    let Some(passwd) = account_records(passwd, 4) else {
        return false;
    };
    let mut user = None;
    for fields in &passwd {
        let (Some(entry_uid), Some(entry_gid)) = (number(fields[2]), number(fields[3])) else {
            return false;
        };
        if entry_gid == gid && entry_uid != uid {
            return false;
        }
        if entry_uid == uid
            && user
                .replace(fields[0])
                .is_some_and(|name| name != fields[0])
        {
            return false;
        }
    }
    let Some(user) = user else {
        return false;
    };
    let Some(groups) = account_records(group, 4) else {
        return false;
    };
    let mut matching = groups
        .iter()
        .filter(|fields| number(fields[2]) == Some(gid));
    let (Some(fields), None) = (matching.next(), matching.next()) else {
        return false;
    };
    fields[0] == user
        && fields[3]
            .split(|byte| *byte == b',')
            .filter(|member| !member.is_empty())
            .all(|member| member == user)
}

fn account_records(content: &[u8], fields: usize) -> Option<Vec<Vec<&[u8]>>> {
    let mut records = Vec::new();
    for line in content.split(|byte| *byte == b'\n') {
        if line.is_empty() || line.starts_with(b"#") {
            continue;
        }
        // NIS compatibility entries name accounts that are not listed here.
        if line.starts_with(b"+") || line.starts_with(b"-") {
            return None;
        }
        let record: Vec<&[u8]> = line.split(|byte| *byte == b':').collect();
        if record.len() < fields || record[0].is_empty() {
            return None;
        }
        records.push(record);
    }
    Some(records)
}

fn number(field: &[u8]) -> Option<u32> {
    std::str::from_utf8(field).ok()?.parse().ok()
}

/// Test-only simulation of filesystems that cannot represent POSIX modes or
/// that report a foreign owner, on the calling thread only.
#[cfg(test)]
pub(crate) mod simulation {
    use std::cell::Cell;

    thread_local! {
        static MOUNT_MODE: Cell<Option<u32>> = const { Cell::new(None) };
        static FOREIGN_OWNER: Cell<bool> = const { Cell::new(false) };
    }

    pub(crate) fn mount_mode() -> Option<u32> {
        MOUNT_MODE.with(Cell::get)
    }

    pub(crate) fn foreign_owner() -> bool {
        FOREIGN_OWNER.with(Cell::get)
    }

    /// Newly created private directories report `mode`, which cannot be changed.
    pub(crate) fn with_mount_mode<T>(mode: u32, run: impl FnOnce() -> T) -> T {
        MOUNT_MODE.with(|cell| cell.set(Some(mode)));
        let result = run();
        MOUNT_MODE.with(|cell| cell.set(None));
        result
    }

    /// Newly created private directories report another owner.
    pub(crate) fn with_foreign_owner<T>(run: impl FnOnce() -> T) -> T {
        FOREIGN_OWNER.with(|cell| cell.set(true));
        let result = run();
        FOREIGN_OWNER.with(|cell| cell.set(false));
        result
    }
}

#[cfg(test)]
#[path = "ownership_tests.rs"]
mod tests;
