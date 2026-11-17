//! Which users may change a Rename parent or stage directory.
//!
//! A protected parent prevents other users from replacing or removing our names:
//! it is owned by the effective UID or root and is either sticky or not writable
//! by other users. Group write is accepted only for the caller's user-private
//! group: the effective GID, named like the effective user in the local account
//! database, with no other listed member and no other local account using it as
//! its primary group, and only on a directory without a POSIX access ACL. With an
//! access ACL the group mode bits are the ACL mask, which also bounds the named
//! users and groups the ACL lets write. Shared-group, ACL-shared and
//! world-writable non-sticky parents remain refused; accounts outside
//! `/etc/passwd` and `/etc/group` (directory services) keep the strict rule.
//! Same-UID and root interference are outside this boundary.
//!
//! A Rename stage is created owner-only. Where an explicit `fchmod(0700)` cannot
//! change the mount-provided mode of a filesystem whose permissions come from
//! mount options, the stage is admitted only while no other user can write to it
//! (group write only as for a protected parent), or while it has exactly the
//! mode, owner and group of a parent that is not protected and neither carries an
//! access ACL, so it never lets another user change more than its parent does.
//! A Rename parent must be protected before the stage is created and again
//! before the capture is requested, so the second case never leads to a capture
//! here. Unlike the platform layer, this provider also reads the stage (to sync
//! it), so a mount-provided mode without owner read is refused. Mode bits
//! protect only where the kernel enforces them: FUSE mounts with `allow_other`
//! but without `default_permissions`, and CIFS mounts with `noperm`, exclude
//! other local users only as far as their server does.
//!
//! The application's Linux platform layer applies the same rules
//! (`crates/midnight-platform-linux/src/ownership.rs`). The two copies cannot
//! share code, so both carry the same rule tables; a change to one must change
//! both.
use crate::rename_sys as sys;
use midnight_filesystem_plugin_api::Failure;
use std::{
    cell::LazyCell,
    fs::{File, Metadata},
    io::Read,
    os::unix::fs::MetadataExt,
    sync::Mutex,
    time::{Duration, Instant},
};

/// Whether other users can neither replace nor remove names in `directory`,
/// whose current metadata is `metadata` (see the module documentation).
pub(crate) fn protected_parent(directory: &File, metadata: &Metadata) -> bool {
    protected_mode(metadata.uid(), metadata.gid(), metadata.mode(), || {
        sys::may_have_access_acl(directory)
    })
}

/// Whether the mount-provided mode of a new stage that `fchmod(0700)` could not
/// change is admitted (see the module documentation). `stage` and `parent` are
/// readable descriptors of the stage, whose current metadata is
/// `stage_metadata`, and of its parent, so neither ACL query needs `/proc`.
pub(crate) fn admits_stage(
    stage: &File,
    stage_metadata: &Metadata,
    parent: &File,
) -> Result<bool, Failure> {
    let parent_metadata = parent.metadata().map_err(Failure::from_io)?;
    Ok(mount_mode_admitted(
        (
            stage_metadata.uid(),
            stage_metadata.gid(),
            stage_metadata.mode(),
        ),
        || sys::may_have_access_acl(stage),
        (
            parent_metadata.uid(),
            parent_metadata.gid(),
            parent_metadata.mode(),
        ),
        || sys::may_have_access_acl(parent),
    ))
}

/// `admits_stage` for the (owner, group, mode) of a stage and of its parent.
/// Each ACL closure reports whether that directory may carry an access ACL and
/// is asked only when its answer can decide. Admitted are a stage only the
/// caller can write (group write only as for a protected parent), and one with
/// exactly the mode, owner and group of a parent that is not protected, neither
/// carrying an access ACL: it lets nobody change anything the parent does not
/// already let them change.
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

/// `protected_parent` for a directory's owner, group and mode. `acl` reports
/// whether the directory may carry an access ACL; it is asked only when group
/// write is what would admit the directory.
fn protected_mode(uid: u32, gid: u32, mode: u32, acl: impl FnOnce() -> bool) -> bool {
    let caller = sys::effective_uid();
    (uid == caller || uid == 0) && (mode & 0o1000 != 0 || private_writers_only(gid, mode, acl))
}

/// Whether only the caller can write to a directory with this owner, group and
/// mode: owned by the effective UID, never world-writable, and group-writable
/// only as a protected parent may be; `acl` as for `protected_mode`.
fn writable_only_mode(uid: u32, gid: u32, mode: u32, acl: impl FnOnce() -> bool) -> bool {
    uid == sys::effective_uid() && private_writers_only(gid, mode, acl)
}

/// No write access for other users, and group write only for the caller's
/// private group on a directory whose group bits are not an ACL mask.
fn private_writers_only(gid: u32, mode: u32, acl: impl FnOnce() -> bool) -> bool {
    mode & 0o002 == 0 && (mode & 0o020 == 0 || (caller_private_group(gid) && !acl()))
}

/// Whether `gid` is the effective user's private group. Results are cached
/// briefly because every Rename checks the same parent several times.
fn caller_private_group(gid: u32) -> bool {
    const VALIDITY: Duration = Duration::from_secs(10);
    static CACHE: VerdictCache = VerdictCache::new();
    if gid != sys::effective_gid() {
        return false;
    }
    #[cfg(test)]
    if let Some(verdict) = crate::rename_test_hooks::private_group() {
        return verdict;
    }
    let uid = sys::effective_uid();
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

/// The user-private-group rule over `passwd(5)` and `group(5)` content. Unknown,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rename_test_support::{Temp, remove_access_acl, share_through_access_acl};
    use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};

    const PASSWD: &[u8] = b"root:x:0:0:root:/root:/bin/bash\n\
        alice:x:1000:1000:Alice:/home/alice:/bin/bash\n\
        bob:x:1001:100:Bob:/home/bob:/bin/bash\n";

    #[test]
    fn user_private_group_requires_matching_name_and_no_other_member() {
        let group = b"root:x:0:\nalice:x:1000:\nusers:x:100:bob\n";
        assert!(private_group_in(PASSWD, group, 1000, 1000));
        // Listing the user in its own group keeps it private.
        assert!(private_group_in(
            PASSWD,
            b"alice:x:1000:alice\n",
            1000,
            1000
        ));
        // Another member, another primary user, another name or a shared group never qualify.
        assert!(!private_group_in(PASSWD, b"alice:x:1000:bob\n", 1000, 1000));
        assert!(!private_group_in(PASSWD, b"staff:x:1000:\n", 1000, 1000));
        assert!(!private_group_in(PASSWD, group, 1001, 100));
        let shared = b"alice:x:1000:1000::/home/alice:/bin/sh\ncarol:x:1002:1000::/:/bin/sh\n";
        assert!(!private_group_in(shared, b"alice:x:1000:\n", 1000, 1000));
    }

    #[test]
    fn ambiguous_unknown_or_compatibility_accounts_never_qualify() {
        let group = b"alice:x:1000:\n";
        // Unknown user, duplicate group IDs, NIS compatibility and malformed records.
        assert!(!private_group_in(PASSWD, group, 4242, 1000));
        assert!(!private_group_in(
            PASSWD,
            b"alice:x:1000:\nalias:x:1000:\n",
            1000,
            1000
        ));
        assert!(!private_group_in(
            b"+::::::\nalice:x:1000:1000::/:/bin/sh\n",
            group,
            1000,
            1000
        ));
        assert!(!private_group_in(
            PASSWD,
            b"+:::\nalice:x:1000:\n",
            1000,
            1000
        ));
        assert!(!private_group_in(
            b"alice:x:not-a-number:1000\n",
            group,
            1000,
            1000
        ));
        assert!(!private_group_in(
            b"alice:x:1000:1000::/:/bin/sh\nalias:x:1000:1000::/:/bin/sh\n",
            group,
            1000,
            1000
        ));
    }

    /// The same table as the platform's test of this name
    /// (`crates/midnight-platform-linux/src/ownership_tests.rs`).
    #[test]
    fn protected_parent_accepts_owned_or_sticky_and_refuses_shared_writers() {
        let uid = sys::effective_uid();
        let gid = sys::effective_gid();
        let other_uid = if uid == 0 { 1 } else { uid + 1 };
        let other_gid = gid.wrapping_add(7919);
        let private = caller_private_group(gid);
        for (owner, group, mode, acl, expected) in [
            (uid, gid, 0o40700, false, true),
            (uid, gid, 0o40755, false, true),
            // Without group write, an ACL mask lets no named user or group write.
            (uid, gid, 0o40755, true, true),
            (0, gid, 0o40755, false, true),
            (uid, other_gid, 0o40775, false, false),
            (uid, gid, 0o40777, false, false),
            (uid, other_gid, 0o41777, false, true),
            (uid, other_gid, 0o41777, true, true),
            (0, other_gid, 0o41777, false, true),
            (other_uid, gid, 0o40755, false, false),
            (other_uid, gid, 0o41777, false, false),
            // Group write is the caller's alone only for its private group and only
            // while the group bits are not the mask of an ACL naming other writers.
            (uid, gid, 0o40775, false, private),
            (0, gid, 0o40775, false, private),
            (uid, gid, 0o40775, true, false),
            (0, gid, 0o40775, true, false),
        ] {
            let asked = std::cell::Cell::new(false);
            let verdict = protected_mode(owner, group, mode, || {
                asked.set(true);
                acl
            });
            assert_eq!(
                verdict, expected,
                "owner {owner} group {group} mode {mode:o} acl {acl}"
            );
            // The ACL is read only when group write is what would admit the parent.
            let group_write_decides = (owner == uid || owner == 0)
                && mode & 0o1000 == 0
                && mode & 0o002 == 0
                && mode & 0o020 != 0
                && group == gid
                && private;
            assert_eq!(asked.get(), group_write_decides, "mode {mode:o}");
        }
        assert!(!writable_only_mode(uid, other_gid, 0o40770, || false));
        assert!(!writable_only_mode(uid, gid, 0o41777, || false));
        assert!(!writable_only_mode(other_uid, gid, 0o40700, || false));
        assert!(writable_only_mode(uid, other_gid, 0o40755, || true));
        assert_eq!(writable_only_mode(uid, gid, 0o40775, || false), private);
        assert!(!writable_only_mode(uid, gid, 0o40775, || true));
    }

    /// The same table as the platform's test of this name
    /// (`crates/midnight-platform-linux/src/ownership_tests.rs`).
    #[test]
    fn mount_provided_stage_modes_are_never_wider_than_an_unprotected_parent() {
        const T: bool = true;
        const F: bool = false;
        // u, g: the effective user and group; o: another group; r: another owner;
        // p: whether g is the caller's private group; s: whether u is root.
        let u = sys::effective_uid();
        let g = sys::effective_gid();
        let o = g.wrapping_add(7919);
        let r = u.wrapping_add(1);
        let p = caller_private_group(g);
        let s = u == 0;
        // (stage, its ACL, parent, its ACL) -> (admitted, stage ACL read, parent ACL read)
        for (stage, stage_acl, parent, parent_acl, expected) in [
            // Only the caller can write the stage, whatever its parent.
            ((u, g, 0o40755), F, (u, g, 0o40700), F, (T, F, F)),
            ((u, o, 0o40750), F, (u, g, 0o40700), F, (T, F, F)),
            // Without group write, an ACL mask lets no named user or group write.
            ((u, g, 0o40755), T, (u, g, 0o40700), F, (T, F, F)),
            // Private-group write, but only on a stage without an access ACL.
            ((u, g, 0o40775), F, (u, g, 0o40700), F, (p, p, F)),
            ((u, g, 0o40775), T, (u, g, 0o40700), F, (F, p, F)),
            // Wider than the parent: refused without reading an ACL.
            ((u, g, 0o40777), F, (u, g, 0o40755), F, (F, F, F)),
            ((u, g, 0o40757), F, (u, g, 0o40755), F, (F, F, F)),
            ((u, o, 0o40775), F, (u, g, 0o40755), F, (F, F, F)),
            // Exactly as open as a parent other users can already change ...
            ((u, g, 0o40777), F, (u, g, 0o40777), F, (T, T, T)),
            ((u, o, 0o40775), F, (u, o, 0o40775), F, (T, T, T)),
            // ... while neither carries an access ACL.
            ((u, g, 0o40777), T, (u, g, 0o40777), F, (F, T, F)),
            ((u, g, 0o40777), F, (u, g, 0o40777), T, (F, T, T)),
            // Another parent owner, group or mode, or a protected (sticky) parent.
            ((u, g, 0o40777), F, (r, g, 0o40777), F, (F, F, F)),
            ((u, g, 0o40777), F, (0, g, 0o40777), F, (s, s, s)),
            ((u, g, 0o40777), F, (u, o, 0o40777), F, (F, F, F)),
            ((u, g, 0o40777), F, (u, g, 0o41777), F, (F, F, F)),
            ((u, g, 0o41777), F, (u, g, 0o41777), F, (F, F, F)),
            // A private group admits the stage alone; any other group leaves the
            // parent unprotected, so the stage is then exactly as open as it.
            ((u, g, 0o40775), F, (u, g, 0o40775), F, (T, T, !p)),
        ] {
            let asked = (std::cell::Cell::new(false), std::cell::Cell::new(false));
            let verdict = mount_mode_admitted(
                stage,
                || {
                    asked.0.set(true);
                    stage_acl
                },
                parent,
                || {
                    asked.1.set(true);
                    parent_acl
                },
            );
            assert_eq!(
                (verdict, asked.0.get(), asked.1.get()),
                expected,
                "stage {:?} mode {:o} acl {stage_acl}, parent {:?} mode {:o} acl {parent_acl}",
                (stage.0, stage.1),
                stage.2,
                (parent.0, parent.1),
                parent.2
            );
        }
    }

    #[test]
    fn access_acls_are_detected_through_readable_and_path_descriptors() {
        let temp = Temp::new();
        let directory = temp.path.join("shared");
        std::fs::create_dir(&directory).unwrap();
        let open = |flags: i32| {
            OpenOptions::new()
                .read(true)
                .custom_flags(flags | libc::O_DIRECTORY | libc::O_NOFOLLOW)
                .open(&directory)
                .unwrap()
        };
        let readable = open(0);
        let pinned = open(libc::O_PATH);
        assert!(!sys::may_have_access_acl(&readable));
        assert!(!sys::may_have_access_acl(&pinned));
        if !share_through_access_acl(&directory, "access ACL detection") {
            return;
        }
        assert!(sys::may_have_access_acl(&readable));
        assert!(sys::may_have_access_acl(&pinned));
        let metadata = pinned.metadata().unwrap();
        assert_eq!(metadata.mode() & 0o7777, 0o775);
        assert!(!protected_parent(&pinned, &metadata));
        // As a stage below its 0755 fixture parent it is neither the caller's
        // alone nor exactly as open as that parent.
        let parent = File::open(&temp.path).unwrap();
        assert!(!admits_stage(&readable, &metadata, &parent).unwrap());
        remove_access_acl(&directory);
        assert!(!sys::may_have_access_acl(&readable));
        assert!(!sys::may_have_access_acl(&pinned));
        let metadata = pinned.metadata().unwrap();
        assert_eq!(
            protected_parent(&pinned, &metadata),
            caller_private_group(metadata.gid())
        );
    }

    #[test]
    fn a_verdict_being_computed_never_blocks_cached_checks() {
        use std::{sync::mpsc, thread};
        // Elapses only on failure: a lock held while computing blocks the cached check.
        const DEADLINE: Duration = Duration::from_secs(10);
        let validity = Duration::from_secs(600);
        let cache = VerdictCache::new();
        let shared = &cache;
        assert!(cache.get((1, 1), validity, || true));
        let (entered, computing) = mpsc::channel();
        let (finish, finished) = mpsc::channel::<()>();
        thread::scope(|scope| {
            let slow = scope.spawn(move || {
                shared.get((2, 2), validity, move || {
                    entered.send(()).unwrap();
                    finished.recv_timeout(DEADLINE).is_ok()
                })
            });
            computing.recv_timeout(DEADLINE).unwrap();
            assert!(cache.get((1, 1), validity, || panic!(
                "a cached check waited for another verdict's computation"
            )));
            finish.send(()).unwrap();
            assert!(slow.join().unwrap());
        });
        assert!(cache.get((2, 2), validity, || false));
    }
}
