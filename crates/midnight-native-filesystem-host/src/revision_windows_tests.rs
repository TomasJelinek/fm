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
