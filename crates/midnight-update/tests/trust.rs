//! The signature model: every refusal names its reason.
use crate::support::{TestKey, sha256_hex};
use midnight_update::trust::{TrustAnchor, TrustError, parse_sums, verify_sums};
use semver::Version;

fn version() -> Version {
    Version::parse("0.5.1").unwrap()
}

fn sums() -> Vec<u8> {
    format!(
        "{}  midnight-explorer-0.5.1-x86_64-pc-windows-msvc.zip\n{} *midnight-explorer-0.5.1-x86_64-unknown-linux-gnu.tar.gz\n",
        sha256_hex(b"windows archive"),
        sha256_hex(b"linux archive")
    )
    .into_bytes()
}

const COMMENT: &str = "timestamp:1700000000 file:SHA256SUMS release:midnight-explorer-0.5.1";

#[test]
fn a_good_signature_verifies_and_lists_the_files() {
    let key = TestKey::generate();
    let signature = key.sign(&sums(), COMMENT);
    let signed = verify_sums(Some(&key.anchor()), &sums(), &signature, &version()).unwrap();
    assert!(
        signed
            .digest("midnight-explorer-0.5.1-x86_64-pc-windows-msvc.zip")
            .is_some()
    );
    assert!(
        signed
            .digest("midnight-explorer-0.5.1-x86_64-unknown-linux-gnu.tar.gz")
            .is_some()
    );
}

#[test]
fn another_key_is_refused() {
    let signer = TestKey::generate();
    let trusted = TestKey::generate();
    let signature = signer.sign(&sums(), COMMENT);
    assert_eq!(
        verify_sums(Some(&trusted.anchor()), &sums(), &signature, &version()).unwrap_err(),
        TrustError::WrongKey
    );
}

#[test]
fn a_changed_checksum_list_is_refused() {
    let key = TestKey::generate();
    let signature = key.sign(&sums(), COMMENT);
    let mut tampered = sums();
    tampered[0] = if tampered[0] == b'0' { b'1' } else { b'0' };
    assert_eq!(
        verify_sums(Some(&key.anchor()), &tampered, &signature, &version()).unwrap_err(),
        TrustError::BadSignature
    );
}

#[test]
fn a_changed_trusted_comment_is_refused() {
    let key = TestKey::generate();
    let signature = key.sign(&sums(), COMMENT).replace("0.5.1", "0.5.2");
    assert_eq!(
        verify_sums(
            Some(&key.anchor()),
            &sums(),
            &signature,
            &Version::parse("0.5.2").unwrap()
        )
        .unwrap_err(),
        TrustError::BadSignature
    );
}

#[test]
fn a_signature_for_another_release_is_refused() {
    let key = TestKey::generate();
    let signature = key.sign(
        &sums(),
        "timestamp:1 file:SHA256SUMS release:midnight-explorer-0.5.0",
    );
    assert_eq!(
        verify_sums(Some(&key.anchor()), &sums(), &signature, &version()).unwrap_err(),
        TrustError::WrongRelease {
            expected: "midnight-explorer-0.5.1".into(),
            found: "midnight-explorer-0.5.0".into()
        }
    );
    let unmarked = key.sign(&sums(), "timestamp:1 file:SHA256SUMS");
    assert!(matches!(
        verify_sums(Some(&key.anchor()), &sums(), &unmarked, &version()).unwrap_err(),
        TrustError::WrongRelease { .. }
    ));
}

#[test]
fn a_legacy_signature_is_refused() {
    let key = TestKey::generate();
    let signature = key.sign_legacy(&sums(), COMMENT);
    assert_eq!(
        verify_sums(Some(&key.anchor()), &sums(), &signature, &version()).unwrap_err(),
        TrustError::LegacySignature
    );
}

#[test]
fn a_build_without_a_key_installs_nothing() {
    let key = TestKey::generate();
    let signature = key.sign(&sums(), COMMENT);
    assert_eq!(
        verify_sums(None, &sums(), &signature, &version()).unwrap_err(),
        TrustError::NoKey
    );
    // A placeholder (a comment and no key line) is no key; the repository's own file now
    // holds the release key, which parses.
    assert!(
        TrustAnchor::parse("untrusted comment: no release key yet")
            .unwrap()
            .is_none()
    );
    let release = include_str!("../../../packaging/update-signing.pub");
    assert!(TrustAnchor::parse(release).unwrap().is_some());
    assert_eq!(
        TrustAnchor::parse("untrusted comment: x\nnot-base64!").unwrap_err(),
        TrustError::BadKey
    );
}

#[test]
fn a_damaged_signature_file_is_refused() {
    let key = TestKey::generate();
    assert_eq!(
        verify_sums(Some(&key.anchor()), &sums(), "garbage", &version()).unwrap_err(),
        TrustError::MalformedSignature
    );
}

#[test]
fn a_tampered_archive_is_refused_by_its_signed_line() {
    let key = TestKey::generate();
    let folder = tempfile::tempdir().unwrap();
    let archive = folder.path().join("a.zip");
    std::fs::write(&archive, b"windows archive").unwrap();
    let signature = key.sign(&sums(), COMMENT);
    let signed = verify_sums(Some(&key.anchor()), &sums(), &signature, &version()).unwrap();
    let name = "midnight-explorer-0.5.1-x86_64-pc-windows-msvc.zip";
    signed.check_file(name, &archive).unwrap();
    std::fs::write(&archive, b"windows archive with a payload").unwrap();
    assert_eq!(
        signed.check_file(name, &archive).unwrap_err(),
        TrustError::DigestMismatch(name.into())
    );
    assert_eq!(
        signed
            .check_file(
                "midnight-explorer-0.5.2-x86_64-pc-windows-msvc.zip",
                &archive
            )
            .unwrap_err(),
        TrustError::NotListed("midnight-explorer-0.5.2-x86_64-pc-windows-msvc.zip".into())
    );
}

#[test]
fn malformed_checksum_lists_are_refused() {
    for text in [
        "",
        "abc  file\n",
        &format!("{}  \n", "0".repeat(64)),
        &format!("{}  dir/file\n", "0".repeat(64)),
        &format!("{}  a\n{}  a\n", "0".repeat(64), "1".repeat(64)),
        &format!("{} a\n", "0".repeat(64)),
    ] {
        assert_eq!(
            parse_sums(text.as_bytes()).unwrap_err(),
            TrustError::MalformedSums,
            "{text:?}"
        );
    }
    assert!(parse_sums(format!("{}  a\r\n", "a".repeat(64)).as_bytes()).is_ok());
}

/// A key pair, checksum list and signature made by `scripts/release-signing.py`
/// (`tests/fixtures/`), so the release tool and the application agree on the format.
#[test]
fn the_release_scripts_signature_verifies() {
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let public = std::fs::read_to_string(fixtures.join("test-signing.pub")).unwrap();
    let sums = std::fs::read(fixtures.join("SHA256SUMS")).unwrap();
    let signature = std::fs::read_to_string(fixtures.join("SHA256SUMS.minisig")).unwrap();
    let anchor = TrustAnchor::parse(&public).unwrap().unwrap();
    let signed = verify_sums(
        Some(&anchor),
        &sums,
        &signature,
        &Version::parse("9.9.9").unwrap(),
    )
    .unwrap();
    assert!(
        signed
            .digest("midnight-explorer-9.9.9-x86_64-pc-windows-msvc.zip")
            .is_some()
    );
    let mut tampered = sums.clone();
    tampered.push(b'\n');
    assert!(
        verify_sums(
            Some(&anchor),
            &tampered,
            &signature,
            &Version::parse("9.9.9").unwrap()
        )
        .is_err()
    );
}
