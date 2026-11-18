//! Exact historical release inputs keep clean, sparse consumers independent of host sources.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const MAX_ARCHIVE: usize = 16 * 1024 * 1024;
const MAX_INDEX: usize = 1024 * 1024;

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn existing(path: &Path, limit: usize) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit as u64 {
        return Err("invalid existing release file".into());
    }
    Ok(Some(fs::read(path)?))
}

fn write_exact(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(previous) = existing(path, MAX_ARCHIVE)? {
        if previous != bytes {
            return Err("same-version release content conflict".into());
        }
        return Ok(());
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}

/// Append one exact registry record and archive to a fresh bundle. Identical duplicates are
/// idempotent; conflicting same-version metadata or bytes are never overwritten.
pub fn append_registry_package(registry: &Path, line: &[u8], archive: &[u8]) -> Result<()> {
    if line.len() > 64 * 1024 || archive.len() > MAX_ARCHIVE {
        return Err("release input budget exceeded".into());
    }
    let record: Value = serde_json::from_slice(line)?;
    let name = record["name"].as_str().ok_or("missing release name")?;
    let version = record["vers"].as_str().ok_or("missing release version")?;
    if !(4..=64).contains(&name.len())
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        || version.is_empty()
        || version.len() > 64
        || !version.as_bytes()[0].is_ascii_digit()
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".+-".contains(&byte))
        || record["cksum"].as_str() != Some(digest(archive).as_str())
    {
        return Err("invalid registry identity or archive checksum".into());
    }
    let index = registry.join("index").join(&name[..2]).join(&name[2..4]);
    let index_path = index.join(name);
    let previous = existing(&index_path, MAX_INDEX)?.unwrap_or_default();
    let mut duplicate = false;
    for old_line in previous.split(|byte| *byte == b'\n') {
        if old_line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let old: Value = serde_json::from_slice(old_line)?;
        if old["name"].as_str() != Some(name) {
            return Err("registry index name mismatch".into());
        }
        if old["vers"].as_str() == Some(version) {
            if old != record {
                return Err("same-version registry metadata conflict".into());
            }
            duplicate = true;
        }
    }
    let archive_path = registry.join(format!("{name}-{version}.crate"));
    // Validate an existing archive before creating or appending any output.
    if existing(&archive_path, MAX_ARCHIVE)?.is_some_and(|bytes| bytes != archive) {
        return Err("same-version release content conflict".into());
    }
    fs::create_dir_all(&index)?;
    write_exact(&archive_path, archive)?;
    if !duplicate {
        if previous.len() + line.len() + 2 > MAX_INDEX {
            return Err("release registry index budget exceeded".into());
        }
        let mut output = OpenOptions::new()
            .append(true)
            .create(true)
            .open(index_path)?;
        if !previous.is_empty() && !previous.ends_with(b"\n") {
            output.write_all(b"\n")?;
        }
        output.write_all(line)?;
        if !line.ends_with(b"\n") {
            output.write_all(b"\n")?;
        }
    }
    Ok(())
}

struct Legacy {
    name: &'static str,
    archive: &'static [u8],
    index: &'static [u8],
    archive_sha: &'static str,
    index_sha: &'static str,
}

/// Seed the exact already-released 0.1.0 packages before packaging the current source version.
/// These bytes are checked-in inputs; no historical checkout, registry service or repacking occurs.
pub fn seed_legacy_releases(output: &Path) -> Result<Vec<Value>> {
    let packages = [
        Legacy {
            name: "midnight-plugin-api",
            archive: include_bytes!("../releases/0.1.0/midnight-plugin-api-0.1.0.crate"),
            index: include_bytes!("../releases/0.1.0/midnight-plugin-api.index.json"),
            archive_sha: "f264a39c3ee43af518926887a7ef9e4a3751c093f21e924ca0e7f1e3d8009711",
            index_sha: "017a073a7ab39f31264a46484eeb019e573eeb77c9177cc97c883cf67dada00c",
        },
        Legacy {
            name: "midnight-plugin-sdk",
            archive: include_bytes!("../releases/0.1.0/midnight-plugin-sdk-0.1.0.crate"),
            index: include_bytes!("../releases/0.1.0/midnight-plugin-sdk.index.json"),
            archive_sha: "3cbd8b9bd227807c58ee3467a895e93ee0c59eb88064a8fc694f859233c8220a",
            index_sha: "62c3f4bb0960423bb830495a94f5890130ee78aae8d2ee8a9884385352f546d5",
        },
    ];
    let header = include_bytes!("../releases/0.1.0/midnight_plugin_v1.h");
    if digest(header) != "def35c854f09ed1304ecb0ebe7c170e1135ff56bf60cac6e61641a0844625a01" {
        return Err("historical API header checksum mismatch".into());
    }
    for package in &packages {
        let record: Value = serde_json::from_slice(package.index)?;
        if digest(package.archive) != package.archive_sha
            || digest(package.index) != package.index_sha
            || record["name"].as_str() != Some(package.name)
            || record["vers"].as_str() != Some("0.1.0")
        {
            return Err("historical release input checksum or identity mismatch".into());
        }
    }
    for package in &packages {
        append_registry_package(&output.join("registry"), package.index, package.archive)?;
    }
    let include = output.join("include/0.1.0");
    fs::create_dir_all(&include)?;
    write_exact(&include.join("midnight_plugin_v1.h"), header)?;
    let mut records: Vec<Value> =
        serde_json::from_slice(include_bytes!("../releases/0.1.0/artifacts.json"))?;
    if records.len() != packages.len() {
        return Err("historical release provenance count mismatch".into());
    }
    for (record, package) in records.iter_mut().zip(packages) {
        if record["archive"].as_str() != Some(format!("{}-0.1.0.crate", package.name).as_str())
            || record["sha256"].as_str() != Some(package.archive_sha)
        {
            return Err("historical release provenance identity mismatch".into());
        }
        record["preserved"] = Value::Bool(true);
    }
    Ok(records)
}
