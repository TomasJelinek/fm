use flate2::{Compression, GzBuilder, read::GzDecoder};
use std::{collections::BTreeMap, io::Read};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Cargo records the entire repository commit inside each package. Keep that
/// provenance in the distribution manifest instead: a commit changing only an
/// independent plugin must not invalidate its locked API archive checksum.
/// No source bytes, manifests, dependency versions or third-party crates change.
pub fn canonical_package(bytes: &[u8], prefix: &str) -> Result<Vec<u8>> {
    const LIMIT: usize = 16 * 1024 * 1024;
    if bytes.len() > LIMIT {
        return Err("oversized Cargo archive".into());
    }
    let mut archive = tar::Archive::new(GzDecoder::new(bytes));
    let mut files = BTreeMap::new();
    let mut total = 0usize;
    for (index, entry) in archive.entries()?.enumerate() {
        let mut entry = entry?;
        let path = entry
            .path()?
            .to_str()
            .ok_or("non-Unicode crate member")?
            .to_owned();
        let mut parts = path.split('/');
        if parts.next() != Some(prefix)
            || path.contains(['\\', ':'])
            || parts.clone().next().is_none()
            || parts.any(|part| part.is_empty() || part == "." || part == "..")
            || !entry.header().entry_type().is_file()
        {
            return Err("invalid Cargo package member".into());
        }
        if index >= 512 || entry.size() > 1024 * 1024 {
            return Err("Cargo package member budget exceeded".into());
        }
        let mode = if entry.header().mode()? & 0o111 == 0 {
            0o644
        } else {
            0o755
        };
        let mut data = Vec::new();
        entry
            .by_ref()
            .take(1024 * 1024 + 1)
            .read_to_end(&mut data)?;
        total = total
            .checked_add(data.len())
            .ok_or("Cargo archive size overflow")?;
        if data.len() > 1024 * 1024 || total > LIMIT {
            return Err("Cargo archive expanded size exceeded".into());
        }
        if path == format!("{prefix}/.cargo_vcs_info.json") {
            continue;
        }
        if files.insert(path, (mode, data)).is_some() {
            return Err("duplicate Cargo package member".into());
        }
    }
    let encoder = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::default());
    let mut result = tar::Builder::new(encoder);
    for (path, (mode, data)) in files {
        let mut header = tar::Header::new_gnu();
        header.set_mode(mode);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_size(data.len() as u64);
        header.set_cksum();
        result.append_data(&mut header, path, data.as_slice())?;
    }
    Ok(result.into_inner()?.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use std::io::Read;

    fn fixture(commit: &str, data: &[u8], reverse: bool) -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut archive = tar::Builder::new(encoder);
        let mut files = vec![
            ("p-1.0.0/.cargo_vcs_info.json", commit.as_bytes()),
            ("p-1.0.0/src/data.bin", data),
        ];
        if reverse {
            files.reverse();
        }
        for (name, bytes) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_mtime(if reverse { 987 } else { 123 });
            header.set_cksum();
            archive.append_data(&mut header, name, bytes).unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn unrelated_git_commits_and_tar_order_do_not_change_release_checksum() {
        let data = [0, 13, 10, 255];
        let first = canonical_package(&fixture("commit-a", &data, false), "p-1.0.0").unwrap();
        let second = canonical_package(&fixture("commit-b", &data, true), "p-1.0.0").unwrap();
        assert_eq!(first, second);
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(first.as_slice()));
        let mut entries = archive.entries().unwrap();
        let mut entry = entries.next().unwrap().unwrap();
        assert_eq!(entry.path().unwrap().to_str(), Some("p-1.0.0/src/data.bin"));
        let mut extracted = Vec::new();
        entry.read_to_end(&mut extracted).unwrap();
        assert_eq!(extracted, data, "source/binary bytes must remain exact");
        assert!(entries.next().is_none());
    }

    #[test]
    fn changed_sources_change_artifact_and_wrong_package_prefix_is_rejected() {
        let before = canonical_package(&fixture("a", b"before", false), "p-1.0.0").unwrap();
        let after = canonical_package(&fixture("a", b"after", false), "p-1.0.0").unwrap();
        assert_ne!(before, after);
        assert!(canonical_package(&fixture("a", b"data", false), "other-1.0.0").is_err());
    }
}
