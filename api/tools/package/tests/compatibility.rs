use midnight_api_package::{append_registry_package, seed_legacy_releases};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(1);
struct Output(PathBuf);
impl Output {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "midnight-sdk-registry-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn registry(&self) -> PathBuf {
        self.0.join("registry")
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        assert!(self.0.starts_with(std::env::temp_dir()));
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn record(name: &str, version: &str, bytes: &[u8]) -> Value {
    json!({"name":name,"vers":version,"cksum":format!("{:x}",Sha256::digest(bytes)),
        "deps":[],"features":{},"yanked":false})
}
fn publish(registry: &std::path::Path, value: &Value, bytes: &[u8]) -> bool {
    append_registry_package(registry, &serde_json::to_vec(value).unwrap(), bytes).is_ok()
}

#[test]
fn fresh_bundle_contains_both_versions_and_exact_historical_bytes() {
    let output = Output::new();
    let legacy = seed_legacy_releases(&output.0).unwrap();
    assert_eq!(legacy.len(), 2);
    for (name, archive, index) in [
        (
            "midnight-plugin-api",
            include_bytes!("../releases/0.1.0/midnight-plugin-api-0.1.0.crate").as_slice(),
            include_bytes!("../releases/0.1.0/midnight-plugin-api.index.json").as_slice(),
        ),
        (
            "midnight-plugin-sdk",
            include_bytes!("../releases/0.1.0/midnight-plugin-sdk-0.1.0.crate").as_slice(),
            include_bytes!("../releases/0.1.0/midnight-plugin-sdk.index.json").as_slice(),
        ),
    ] {
        let new_bytes = format!("{name} current release");
        assert!(publish(
            &output.registry(),
            &record(name, "0.2.0", new_bytes.as_bytes()),
            new_bytes.as_bytes()
        ));
        assert_eq!(
            fs::read(output.registry().join(format!("{name}-0.1.0.crate"))).unwrap(),
            archive
        );
        let lines = fs::read(output.registry().join("index/mi/dn").join(name)).unwrap();
        assert!(lines.starts_with(index));
        let versions = String::from_utf8(lines)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap()["vers"].clone())
            .collect::<Vec<_>>();
        assert_eq!(versions, [json!("0.1.0"), json!("0.2.0")]);
    }
    assert_eq!(
        fs::read(output.0.join("include/0.1.0/midnight_plugin_v1.h")).unwrap(),
        include_bytes!("../releases/0.1.0/midnight_plugin_v1.h")
    );
}

#[test]
fn same_version_content_or_metadata_conflicts_preserve_existing_release() {
    let output = Output::new();
    let first = record("midnight-plugin-api", "0.2.0", b"original");
    assert!(publish(&output.registry(), &first, b"original"));
    let index = output.registry().join("index/mi/dn/midnight-plugin-api");
    let before = fs::read(&index).unwrap();
    let changed = record("midnight-plugin-api", "0.2.0", b"different");
    assert!(!publish(&output.registry(), &changed, b"different"));
    let mut metadata = first.clone();
    metadata["yanked"] = json!(true);
    assert!(!publish(&output.registry(), &metadata, b"original"));
    assert_eq!(fs::read(&index).unwrap(), before);
    assert_eq!(
        fs::read(output.registry().join("midnight-plugin-api-0.2.0.crate")).unwrap(),
        b"original"
    );
}

#[test]
fn identical_release_is_idempotent_without_duplicate_registry_records() {
    let output = Output::new();
    let value = record("midnight-plugin-sdk", "0.2.0", b"same");
    assert!(publish(&output.registry(), &value, b"same"));
    let index = output.registry().join("index/mi/dn/midnight-plugin-sdk");
    let before = fs::read(&index).unwrap();
    assert!(publish(&output.registry(), &value, b"same"));
    assert_eq!(fs::read(index).unwrap(), before);
}

#[test]
fn wrong_archive_checksum_is_refused_before_creating_registry_files() {
    let output = Output::new();
    assert!(!publish(
        &output.registry(),
        &record("midnight-plugin-api", "0.2.0", b"expected"),
        b"tampered"
    ));
    assert!(!output.registry().exists());
}

#[test]
fn registry_identity_cannot_escape_the_fresh_output() {
    for (name, version) in [("../escape", "0.2.0"), ("midnight-plugin-api", "../escape")] {
        let output = Output::new();
        assert!(!publish(
            &output.registry(),
            &record(name, version, b"x"),
            b"x"
        ));
        assert!(!output.registry().exists());
    }
}
