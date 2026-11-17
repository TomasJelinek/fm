//! Which users may change a Rename parent or stage directory.
//!
//! A protected parent prevents other users from replacing or removing our names:
//! it is owned by the effective UID or root and is either sticky or not writable
//! by other users. Group write is accepted only for the caller's user-private
//! group: the effective GID, named like the effective user in the local account
//! database, with no other listed member and no other local account using it as
//! its primary group. Shared groups and world-writable non-sticky parents remain
//! refused; accounts outside `/etc/passwd` and `/etc/group` (directory services)
//! keep the strict rule. Same-UID and root interference are outside this
//! boundary. The application's Linux platform layer applies the same rule.
use crate::rename_sys as sys;
use std::{
    fs::File,
    io::Read,
    sync::Mutex,
    time::{Duration, Instant},
};

/// Whether other users can neither replace nor remove names in a directory
/// with this owner, group and mode.
pub(crate) fn protected_parent(uid: u32, gid: u32, mode: u32) -> bool {
    let caller = sys::effective_uid();
    if uid != caller && uid != 0 {
        return false;
    }
    if mode & 0o1000 != 0 {
        return true;
    }
    mode & 0o002 == 0 && (mode & 0o020 == 0 || caller_private_group(gid))
}

/// Whether only the caller can write to a directory: owned by the effective
/// UID, never world-writable and group-writable only for its private group.
pub(crate) fn writable_only_by_caller(uid: u32, gid: u32, mode: u32) -> bool {
    uid == sys::effective_uid()
        && mode & 0o002 == 0
        && (mode & 0o020 == 0 || caller_private_group(gid))
}

/// Whether `gid` is the effective user's private group. A short-lived cache
/// avoids rereading the account files for every check of the same parent.
fn caller_private_group(gid: u32) -> bool {
    const VALIDITY: Duration = Duration::from_secs(10);
    static CACHE: Mutex<Option<(u32, u32, Instant, bool)>> = Mutex::new(None);
    if gid != sys::effective_gid() {
        return false;
    }
    #[cfg(test)]
    if let Some(verdict) = crate::rename_test_hooks::private_group() {
        return verdict;
    }
    let uid = sys::effective_uid();
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
    use super::private_group_in;

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
}
