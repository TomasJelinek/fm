//! Shared ownership rules for private staging directories and protected parents.
//!
//! Private staging creates a directory with mode 0700 in a pinned parent and pins
//! it without following links; it must be a directory owned by the effective UID
//! on the parent's filesystem. Filesystems whose permissions come from mount
//! options (vfat, exfat, ntfs-3g/fuseblk, CIFS without Unix extensions, GVfs and
//! other FUSE mounts) cannot store that mode: an explicit `fchmod(0700)` of our own
//! new directory keeps the mount-provided mode. Such a mode is accepted only while
//! no other user can write to the directory (group write only as for a protected
//! parent: the private group, without an access ACL), or while it has exactly the
//! mode, owner and group of a parent that is not protected and neither carries an
//! access ACL. The directory therefore never lets another user change more than
//! its parent does. Mode bits protect only where the kernel enforces them: FUSE
//! mounts with `allow_other` but without `default_permissions`, and CIFS mounts
//! with `noperm`, exclude other local users only as far as their server does. A
//! creator removes a directory it refuses. The Linux Rename plugin applies the
//! same rule (`plugins/midnight-filesystem-native-linux/src/ownership.rs`); both
//! carry the same rule tables.
//!
//! A protected parent prevents other users from replacing or removing our names:
//! it is owned by the effective UID or root and is either sticky or not writable by
//! other users. Group write is accepted only for the caller's user-private group:
//! the effective GID, named like the effective user in the local account database,
//! with no other listed member and no other local account using it as its primary
//! group, and only on a directory without a POSIX access ACL. With an access ACL the
//! group mode bits are the ACL mask, which also bounds the named users and groups
//! the ACL lets write. Shared-group, ACL-shared and world-writable non-sticky parents
//! remain refused. Accounts outside `/etc/passwd`/`/etc/group` (directory services)
//! keep the strict rule. Same-UID and root interference remain outside this boundary.

use rustix::fs::{
    AtFlags, FileType, Mode, OFlags, Stat, fchmod, fgetxattr_query, fstat, getxattr_query, openat,
    statat, unlinkat,
};
use std::{
    cell::LazyCell,
    ffi::OsStr,
    fs::File,
    io::{self, Read},
    os::fd::{AsFd, AsRawFd},
    sync::Mutex,
    time::{Duration, Instant},
};

/// The extended attribute that holds an inode's POSIX access ACL.
const ACCESS_ACL: &str = "system.posix_acl_access";

/// How a directory created by this process with mode 0700 may be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CreatedDirectory {
    /// Owned by the effective UID with exactly mode 0700.
    Private,
    /// Owned by the effective UID on a filesystem that cannot store the requested
    /// mode; the value is the mount-provided permission bits, which the module's
    /// rule admitted.
    MountProvided(u32),
}

/// Classifies a directory this process just created in `parent` with
/// `mkdirat(..., 0o700)` and opened without following links (see the module
/// documentation). Refuses a non-directory, a foreign owner, another filesystem
/// than the parent's and a mount-provided mode that would let other users change
/// the directory more than `parent` allows; the caller removes a directory it
/// refuses.
pub(crate) fn check_created_directory(
    parent: impl AsFd,
    directory: &File,
) -> io::Result<CreatedDirectory> {
    let parent_stat = fstat(&parent)?;
    let stat = owned_directory(directory, parent_stat.st_dev)?;
    if stat.st_mode & 0o777 == 0o700 {
        return Ok(CreatedDirectory::Private);
    }
    // A restrictive umask or default ACL is corrected on our own new directory.
    // Mount-option permissions keep their mode (vfat/exfat silently, others with
    // an error), which identifies a filesystem that cannot represent it. The
    // pinned handle may be O_PATH: this readable reopen of exactly that directory
    // serves fchmod and the ACL query, which then needs no /proc fallback.
    let readable = reopen_readable(directory);
    if let Ok(readable) = &readable {
        let _ = fchmod(readable, Mode::RWXU);
    }
    let stat = owned_directory(directory, parent_stat.st_dev)?;
    if stat.st_mode & 0o777 == 0o700 {
        return Ok(CreatedDirectory::Private);
    }
    let admitted = mount_mode_admitted(
        (stat.st_uid, stat.st_gid, stat.st_mode),
        || match &readable {
            Ok(readable) => may_have_access_acl(readable),
            Err(_) => may_have_access_acl(directory),
        },
        (parent_stat.st_uid, parent_stat.st_gid, parent_stat.st_mode),
        // The parent may be an O_PATH pin too.
        || match reopen_readable(&parent) {
            Ok(readable) => may_have_access_acl(&readable),
            Err(_) => may_have_access_acl(&parent),
        },
    );
    if !admitted {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "its mount-provided permissions would let other users change it more than its parent folder allows",
        ));
    }
    Ok(CreatedDirectory::MountProvided(stat.st_mode & 0o777))
}

/// The status of a new private directory, which must be a directory owned by
/// the effective UID on the filesystem `device` of its parent.
fn owned_directory(directory: &File, device: u64) -> io::Result<Stat> {
    let stat = fstat(directory)?;
    #[cfg(test)]
    let stat = simulation::reported(stat);
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the new private directory is not a directory",
        ));
    }
    if stat.st_uid != rustix::process::geteuid().as_raw() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the new private directory is not owned by this user",
        ));
    }
    if stat.st_dev != device {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the new private directory is on another filesystem",
        ));
    }
    Ok(stat)
}

/// A readable descriptor of exactly the directory `directory` refers to.
fn reopen_readable(directory: impl AsFd) -> rustix::io::Result<std::os::fd::OwnedFd> {
    openat(
        directory,
        ".",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
}

/// Whether a mount-provided mode that `fchmod(0700)` could not change is
/// admitted. `stage` and `parent` are the (owner, group, mode) of the new
/// directory and of its parent; each ACL closure reports whether that directory
/// may carry an access ACL and is asked only when its answer can decide.
/// Admitted are a directory only the caller can write (group write only as for a
/// protected parent), and one with exactly the mode, owner and group of a parent
/// that is not protected, neither carrying an access ACL: it lets nobody change
/// anything the parent does not already let them change.
fn mount_mode_admitted(
    stage: (u32, u32, u32),
    stage_acl: impl FnOnce() -> bool,
    parent: (u32, u32, u32),
    parent_acl: impl FnOnce() -> bool,
) -> bool {
    let stage_acl = LazyCell::new(stage_acl);
    let (uid, gid, mode) = stage;
    if writable_only_mode(uid, gid, mode, || *stage_acl) {
        return true;
    }
    let (parent_uid, parent_gid, parent_mode) = parent;
    // Equal modes, owners and groups first: no ACL is read for a stage that
    // differs from its parent or for a protected parent.
    (mode & 0o7777, uid, gid) == (parent_mode & 0o7777, parent_uid, parent_gid)
        && !protected_mode(parent_uid, parent_gid, parent_mode, || false)
        && !*stage_acl
        && !parent_acl()
}

/// Removes the empty directory this process created as `parent/name` and then
/// refused. `pinned` is its `(device, inode)` when it was opened; a replaced or
/// non-directory entry is left untouched, and rmdir never removes contents.
pub(crate) fn remove_refused_directory(
    parent: impl AsFd,
    name: &OsStr,
    pinned: Option<(u64, u64)>,
) -> io::Result<()> {
    #[cfg(test)]
    simulation::before_refused_removal(name);
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

/// Whether other users can neither replace nor remove names in `directory`
/// (see the module documentation).
pub(crate) fn protected_parent(directory: impl AsFd) -> io::Result<bool> {
    let stat = fstat(&directory)?;
    Ok(protected_mode(
        stat.st_uid,
        stat.st_gid,
        stat.st_mode,
        || may_have_access_acl(&directory),
    ))
}

/// Whether only the caller can write to `directory`: owned by the effective UID,
/// never world-writable, and group-writable only as a protected parent may be.
pub(crate) fn writable_only_by_caller(directory: impl AsFd) -> io::Result<bool> {
    let stat = fstat(&directory)?;
    Ok(writable_only_mode(
        stat.st_uid,
        stat.st_gid,
        stat.st_mode,
        || may_have_access_acl(&directory),
    ))
}

/// `protected_parent` for a directory's owner, group and mode. `acl` reports
/// whether the directory may carry an access ACL; it is asked only when group
/// write is what would admit the directory.
fn protected_mode(uid: u32, gid: u32, mode: u32, acl: impl FnOnce() -> bool) -> bool {
    let caller = rustix::process::geteuid().as_raw();
    (uid == caller || uid == 0) && (mode & 0o1000 != 0 || private_writers_only(gid, mode, acl))
}

/// `writable_only_by_caller` for a directory's owner, group and mode; `acl` as
/// for `protected_mode`.
fn writable_only_mode(uid: u32, gid: u32, mode: u32, acl: impl FnOnce() -> bool) -> bool {
    uid == rustix::process::geteuid().as_raw() && private_writers_only(gid, mode, acl)
}

/// No write access for other users, and group write only for the caller's
/// private group on a directory whose group bits are not an ACL mask.
fn private_writers_only(gid: u32, mode: u32, acl: impl FnOnce() -> bool) -> bool {
    mode & 0o002 == 0 && (mode & 0o020 == 0 || (caller_private_group(gid) && !acl()))
}

/// Whether `directory` may carry a POSIX access ACL. Only a missing attribute
/// (ENODATA) or a filesystem without POSIX ACLs (EOPNOTSUPP) proves that it has
/// none; any other failure counts as an ACL, which refuses group write.
fn may_have_access_acl(directory: impl AsFd) -> bool {
    let directory = directory.as_fd();
    let query = match fgetxattr_query(directory, ACCESS_ACL) {
        // Kernels that refuse xattr calls on O_PATH descriptors read the same
        // pinned inode through its descriptor link instead.
        Err(rustix::io::Errno::BADF) => getxattr_query(
            format!("/proc/self/fd/{}", directory.as_raw_fd()).as_str(),
            ACCESS_ACL,
        ),
        query => query,
    };
    !matches!(
        query,
        Err(rustix::io::Errno::NODATA | rustix::io::Errno::OPNOTSUPP)
    )
}

/// Whether `gid` is the effective user's private group. Results are cached briefly
/// because recursive transfers check the same parents' group for every child.
pub(crate) fn caller_private_group(gid: u32) -> bool {
    const VALIDITY: Duration = Duration::from_secs(10);
    static CACHE: VerdictCache = VerdictCache::new();
    if gid != rustix::process::getegid().as_raw() {
        return false;
    }
    let uid = rustix::process::geteuid().as_raw();
    CACHE.get((uid, gid), VALIDITY, || {
        match (
            read_account_file("/etc/passwd"),
            read_account_file("/etc/group"),
        ) {
            (Some(passwd), Some(group)) => private_group_in(&passwd, &group, uid, gid),
            _ => false,
        }
    })
}

/// One briefly cached verdict. Its lock is never held while a verdict is
/// computed, so concurrent checks never queue behind another check's I/O; a
/// concurrent miss may compute the same verdict twice.
struct VerdictCache(Mutex<Option<Verdict>>);

/// A verdict, its key and when it was computed.
#[derive(Clone, Copy)]
struct Verdict {
    key: (u32, u32),
    computed: Instant,
    value: bool,
}

impl VerdictCache {
    const fn new() -> Self {
        Self(Mutex::new(None))
    }

    /// The verdict cached for `key` while it is younger than `validity`;
    /// otherwise `compute()`, which is then cached.
    fn get(&self, key: (u32, u32), validity: Duration, compute: impl FnOnce() -> bool) -> bool {
        let cached = *self.0.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(cached) = cached
            && cached.key == key
            && cached.computed.elapsed() < validity
        {
            return cached.value;
        }
        let value = compute();
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = Some(Verdict {
            key,
            computed: Instant::now(),
            value,
        });
        value
    }
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

/// Test-only simulation of filesystems that cannot represent POSIX modes, that
/// report a foreign owner or that place a new directory on another filesystem,
/// on the calling thread only. Each alters the status reported for a new private
/// directory, so the real checks decide. Also interference with the removal of a
/// refused directory.
#[cfg(test)]
pub(crate) mod simulation {
    use rustix::fs::Stat;
    use std::{
        cell::{Cell, RefCell},
        ffi::OsStr,
        thread::LocalKey,
    };

    type Interference = Box<dyn FnOnce(&OsStr)>;

    thread_local! {
        static MOUNT_MODE: Cell<Option<u32>> = const { Cell::new(None) };
        static FOREIGN_OWNER: Cell<bool> = const { Cell::new(false) };
        static FOREIGN_DEVICE: Cell<bool> = const { Cell::new(false) };
        static BEFORE_REFUSED_REMOVAL: RefCell<Option<Interference>> = const { RefCell::new(None) };
    }

    /// Calls `interfere` with the name of the refused directory just before the
    /// first removal of a refused directory that `run` performs.
    pub(crate) fn with_interference_before_refused_removal<T>(
        interfere: impl FnOnce(&OsStr) + 'static,
        run: impl FnOnce() -> T,
    ) -> T {
        BEFORE_REFUSED_REMOVAL.with(|hook| *hook.borrow_mut() = Some(Box::new(interfere)));
        let result = run();
        BEFORE_REFUSED_REMOVAL.with(|hook| hook.borrow_mut().take());
        result
    }

    pub(super) fn before_refused_removal(name: &OsStr) {
        if let Some(interfere) = BEFORE_REFUSED_REMOVAL.with(|hook| hook.borrow_mut().take()) {
            interfere(name);
        }
    }

    /// The status of a new private directory as the simulated filesystem reports it.
    pub(crate) fn reported(mut stat: Stat) -> Stat {
        if let Some(mode) = MOUNT_MODE.with(Cell::get) {
            stat.st_mode = (stat.st_mode & !0o7777) | mode;
        }
        if FOREIGN_OWNER.with(Cell::get) {
            stat.st_uid = stat.st_uid.wrapping_add(1);
        }
        if FOREIGN_DEVICE.with(Cell::get) {
            stat.st_dev = stat.st_dev.wrapping_add(1);
        }
        stat
    }

    /// Newly created private directories report the permission bits `mode`
    /// (with set-ID and sticky bits), which `fchmod` cannot change; the rule then
    /// admits or refuses that mode like a real mount-provided one.
    pub(crate) fn with_mount_mode<T>(mode: u32, run: impl FnOnce() -> T) -> T {
        MOUNT_MODE.with(|cell| cell.set(Some(mode)));
        let result = run();
        MOUNT_MODE.with(|cell| cell.set(None));
        result
    }

    /// Newly created private directories report another owner.
    pub(crate) fn with_foreign_owner<T>(run: impl FnOnce() -> T) -> T {
        with(&FOREIGN_OWNER, run)
    }

    /// Newly created private directories report another filesystem than their
    /// parent's.
    pub(crate) fn with_foreign_device<T>(run: impl FnOnce() -> T) -> T {
        with(&FOREIGN_DEVICE, run)
    }

    fn with<T>(flag: &'static LocalKey<Cell<bool>>, run: impl FnOnce() -> T) -> T {
        flag.with(|cell| cell.set(true));
        let result = run();
        flag.with(|cell| cell.set(false));
        result
    }
}

#[cfg(test)]
#[path = "ownership_tests.rs"]
mod tests;
