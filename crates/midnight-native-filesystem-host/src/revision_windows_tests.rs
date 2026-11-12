use super::matches;

fn token(prefix: &[u8]) -> Vec<u8> {
    [prefix, &(0..48).collect::<Vec<u8>>()].concat()
}

#[test]
fn current_and_legacy_windows_revisions_match_complete_metadata() {
    let current = token(b"native.windows.read.v1");
    assert!(matches(&current, &current));
    assert!(matches(&token(b"windows-file-v1"), &current));
}

#[test]
fn every_windows_metadata_byte_must_match() {
    let current = token(b"native.windows.read.v1");
    for prefix in [b"windows-file-v1".as_slice(), b"native.windows.read.v1"] {
        for byte in 0..48 {
            let mut changed = token(prefix);
            changed[prefix.len() + byte] ^= 1;
            assert!(!matches(&changed, &current), "metadata byte {byte}");
        }
    }
}

#[test]
fn malformed_or_unknown_windows_revisions_never_match() {
    let current = token(b"native.windows.read.v1");
    for prefix in [b"windows-file-v1".as_slice(), b"native.windows.read.v1"] {
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
        b"windows-file-v2".as_slice(),
        b"native.windows.read.v2",
        b"native.linux.read.v1",
    ] {
        let unknown = token(prefix);
        assert!(!matches(&unknown, &current));
        assert!(!matches(&unknown, &unknown));
    }
    assert!(!matches(&current, &token(b"windows-file-v1")));
}

#[test]
fn find_capture_requires_exact_native_identity_and_content_revision() {
    let object = [5u8; 16];
    let entry = midnight_core::FindEntry {
        path: "unused.txt".into(),
        kind: midnight_core::EntryKind::File,
        identity: midnight_core::TransferIdentity { device: 17, object },
        revision: midnight_core::FindRevision([31, 37, 99, 32, 41, 0]),
        metadata: Default::default(),
    };
    let mut payload = 17u64.to_le_bytes().to_vec();
    payload.extend_from_slice(&object);
    for number in [31u64, 37, 41] {
        payload.extend_from_slice(&number.to_le_bytes());
    }

    for prefix in [b"native.windows.read.v1".as_slice(), b"windows-file-v1"] {
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
