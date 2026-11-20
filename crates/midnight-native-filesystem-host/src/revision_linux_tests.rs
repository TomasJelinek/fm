use midnight_plugin_host::RevisionPolicy::{AcceptChangeTimeOnly, Exact};

fn matches(expected: &[u8], current: &[u8]) -> bool {
    super::matches(expected, current, Exact)
}

fn token(prefix: &[u8]) -> Vec<u8> {
    [prefix, &(0..56).collect::<Vec<u8>>()].concat()
}

#[test]
fn current_and_legacy_linux_revisions_match_complete_metadata() {
    let current = token(b"native.linux.read.v1");
    assert!(matches(&current, &current));
    assert!(matches(&token(b"linux-file-v1"), &current));
}

#[test]
fn every_linux_metadata_byte_must_match() {
    let current = token(b"native.linux.read.v1");
    for prefix in [b"linux-file-v1".as_slice(), b"native.linux.read.v1"] {
        for byte in 0..56 {
            let mut changed = token(prefix);
            changed[prefix.len() + byte] ^= 1;
            assert!(!matches(&changed, &current), "metadata byte {byte}");
        }
    }
}

#[test]
fn malformed_or_unknown_linux_revisions_never_match() {
    let current = token(b"native.linux.read.v1");
    for prefix in [b"linux-file-v1".as_slice(), b"native.linux.read.v1"] {
        let valid = token(prefix);
        for length in 0..valid.len() {
            assert!(!matches(&valid[..length], &current));
            assert!(!matches(&valid[..length], &valid[..length]));
        }
        let extended = [valid.as_slice(), &[0]].concat();
        assert!(!matches(&extended, &current));
        assert!(!matches(&extended, &extended));
    }
    for prefix in [
        b"linux-file-v2".as_slice(),
        b"native.linux.read.v2",
        b"native.windows.read.v1",
    ] {
        let unknown = token(prefix);
        assert!(!matches(&unknown, &current));
        assert!(!matches(&unknown, &unknown));
    }
    assert!(!matches(&current, &token(b"linux-file-v1")));
}

#[test]
fn find_capture_requires_exact_native_identity_and_content_revision() {
    let mut object = [0u8; 16];
    object[..8].copy_from_slice(&23u64.to_le_bytes());
    let entry = midnight_core::FindEntry {
        path: "unused.txt".into(),
        kind: midnight_core::EntryKind::File,
        identity: midnight_core::TransferIdentity { device: 17, object },
        revision: midnight_core::FindRevision([31, 37, 41, 43, 47, 0o100644]),
        metadata: Default::default(),
    };
    let payload: Vec<u8> = [17u64, 23, 31, 37, 41, 43, 47]
        .into_iter()
        .flat_map(u64::to_le_bytes)
        .collect();

    for prefix in [b"native.linux.read.v1".as_slice(), b"linux-file-v1"] {
        let valid = [prefix, payload.as_slice()].concat();
        assert!(super::matches_find_entry(&entry, &valid));
        for byte in prefix.len()..valid.len() {
            let mut substituted = valid.clone();
            substituted[byte] ^= 1;
            assert!(
                !super::matches_find_entry(&entry, &substituted),
                "byte {byte}"
            );
        }
        for length in 0..valid.len() {
            assert!(!super::matches_find_entry(&entry, &valid[..length]));
        }
        assert!(!super::matches_find_entry(
            &entry,
            &[valid.as_slice(), &[0]].concat()
        ));
        for kind in [
            midnight_core::EntryKind::Directory,
            midnight_core::EntryKind::Symlink,
        ] {
            let mut changed = entry.clone();
            changed.kind = kind;
            assert!(!super::matches_find_entry(&changed, &valid));
        }
    }
    assert!(!super::matches_find_entry(
        &entry,
        &[b"unknown.v1".as_slice(), &payload].concat()
    ));
}

/// VIEW-05: a viewer read accepts a revision whose ctime (the trailing 16 bytes: seconds
/// and nanoseconds) alone moved; the device, inode, length and mtime bytes still count.
#[test]
fn a_moved_ctime_alone_is_accepted_only_under_its_policy() {
    let current = token(b"native.linux.read.v1");
    let accepts =
        |expected: &[u8], current: &[u8]| super::matches(expected, current, AcceptChangeTimeOnly);
    for prefix in [b"linux-file-v1".as_slice(), b"native.linux.read.v1"] {
        assert!(accepts(&token(prefix), &current));
        for byte in 0..56 {
            let mut changed = token(prefix);
            changed[prefix.len() + byte] ^= 1;
            assert_eq!(
                accepts(&changed, &current),
                byte >= 40,
                "metadata byte {byte}"
            );
            assert!(!matches(&changed, &current), "metadata byte {byte}");
        }
        let valid = token(prefix);
        for length in 0..valid.len() {
            assert!(!accepts(&valid[..length], &current));
        }
        assert!(!accepts(&[valid.as_slice(), &[0]].concat(), &current));
    }
    for prefix in [
        b"linux-file-v2".as_slice(),
        b"native.linux.read.v2",
        b"native.windows.read.v1",
    ] {
        assert!(!accepts(&token(prefix), &current));
        assert!(!accepts(&token(prefix), &token(prefix)));
    }
    // A current revision must use the plugin's current prefix under either policy.
    assert!(!accepts(&current, &token(b"linux-file-v1")));
}
