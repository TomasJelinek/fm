//! Throwaway signing keys, a scripted transport and release fixtures.
#![allow(dead_code)]
use blake2::{Blake2b512, Digest};
use midnight_update::{
    release::{PLATFORM, Platform},
    transport::{Head, Request, Sink, Transport, TransportError},
};
use sha2::Sha256;
use std::{
    collections::HashMap,
    hash::{BuildHasher, Hasher},
    io::Write,
    path::Path,
    sync::{Arc, Condvar, Mutex},
};

/// A key made for one test, never stored.
pub struct TestKey {
    pair: ed25519_compact::KeyPair,
    id: [u8; 8],
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    for (index, chunk) in out.chunks_mut(8).enumerate() {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_usize(index);
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        );
        let value = hasher.finish().to_le_bytes();
        chunk.copy_from_slice(&value[..chunk.len()]);
    }
    out
}

pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

impl TestKey {
    pub fn generate() -> Self {
        let pair = ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new(random_bytes()));
        Self {
            pair,
            id: random_bytes(),
        }
    }

    /// The minisign public key file.
    pub fn public_file(&self) -> String {
        let mut bytes = b"Ed".to_vec();
        bytes.extend_from_slice(&self.id);
        bytes.extend_from_slice(self.pair.pk.as_ref());
        format!(
            "untrusted comment: minisign public key {:016X}\n{}\n",
            u64::from_le_bytes(self.id),
            base64(&bytes)
        )
    }

    pub fn anchor(&self) -> midnight_update::trust::TrustAnchor {
        midnight_update::trust::TrustAnchor::parse(&self.public_file())
            .unwrap()
            .unwrap()
    }

    /// A pre-hashed minisign signature of `data` with `trusted` as its trusted comment.
    pub fn sign(&self, data: &[u8], trusted: &str) -> String {
        self.sign_with(data, trusted, true)
    }

    /// The legacy format, which signs the data itself.
    pub fn sign_legacy(&self, data: &[u8], trusted: &str) -> String {
        self.sign_with(data, trusted, false)
    }

    fn sign_with(&self, data: &[u8], trusted: &str, prehashed: bool) -> String {
        let message = if prehashed {
            Blake2b512::digest(data).to_vec()
        } else {
            data.to_vec()
        };
        let signature = self.pair.sk.sign(&message, None);
        let mut first = if prehashed {
            b"ED".to_vec()
        } else {
            b"Ed".to_vec()
        };
        first.extend_from_slice(&self.id);
        first.extend_from_slice(signature.as_ref());
        let mut global = signature.as_ref().to_vec();
        global.extend_from_slice(trusted.as_bytes());
        let global = self.pair.sk.sign(&global, None);
        format!(
            "untrusted comment: signature from a test key\n{}\ntrusted comment: {trusted}\n{}\n",
            base64(&first),
            base64(global.as_ref())
        )
    }
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn platform() -> Platform {
    PLATFORM.expect("tests run on a supported platform")
}

/// One scripted answer.
#[derive(Clone, Debug)]
pub struct Answer {
    pub head: Head,
    pub body: Vec<u8>,
    /// Break the connection after this many body bytes.
    pub cut_after: Option<usize>,
}

impl Answer {
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            head: Head {
                status: 200,
                ..Head::default()
            },
            body: body.into(),
            cut_after: None,
        }
    }

    pub fn status(status: u16) -> Self {
        Self {
            head: Head {
                status,
                ..Head::default()
            },
            body: Vec::new(),
            cut_after: None,
        }
    }

    pub fn redirect(location: &str) -> Self {
        Self {
            head: Head {
                status: 302,
                location: Some(location.to_owned()),
                ..Head::default()
            },
            body: Vec::new(),
            cut_after: None,
        }
    }
}

/// A transport that answers from a table, honours ranges, and can hold requests
/// until released (to prove that callers do not wait for it).
#[derive(Default)]
pub struct FakeTransport {
    answers: Mutex<HashMap<String, Vec<Answer>>>,
    pub requests: Mutex<Vec<Request>>,
    gate: Mutex<bool>,
    opened: Condvar,
    offline: Mutex<bool>,
}

impl FakeTransport {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            gate: Mutex::new(true),
            ..Self::default()
        })
    }

    /// Answers `url` with `answer`; several answers are used in turn, the last repeats.
    pub fn answer(&self, url: &str, answer: Answer) {
        self.answers
            .lock()
            .unwrap()
            .entry(url.to_owned())
            .or_default()
            .push(answer);
    }

    pub fn hold(&self) {
        *self.gate.lock().unwrap() = false;
    }

    pub fn release(&self) {
        *self.gate.lock().unwrap() = true;
        self.opened.notify_all();
    }

    pub fn set_offline(&self, offline: bool) {
        *self.offline.lock().unwrap() = offline;
    }

    pub fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl Transport for FakeTransport {
    fn get(&self, request: &Request, sink: &mut dyn Sink) -> Result<Head, TransportError> {
        {
            let mut open = self.gate.lock().unwrap();
            while !*open {
                open = self.opened.wait(open).unwrap();
            }
        }
        self.requests.lock().unwrap().push(request.clone());
        if *self.offline.lock().unwrap() {
            return Err(TransportError::Unreachable("offline".into()));
        }
        let answer = {
            let mut answers = self.answers.lock().unwrap();
            let Some(list) = answers.get_mut(&request.url) else {
                return Ok(Head {
                    status: 404,
                    ..Head::default()
                });
            };
            if list.len() > 1 {
                list.remove(0)
            } else {
                list[0].clone()
            }
        };
        let mut head = answer.head.clone();
        if !head.is_success() {
            return Ok(head);
        }
        let mut body = &answer.body[..];
        let start = request.range_from as usize;
        if start > 0 && start <= body.len() {
            head.status = 206;
            head.range_start = Some(request.range_from);
            body = &body[start..];
        }
        head.content_length = Some(body.len() as u64);
        sink.begin(&head).map_err(TransportError::Sink)?;
        let limit = answer.cut_after.unwrap_or(usize::MAX).min(body.len());
        for chunk in body[..limit].chunks(1000) {
            sink.write(chunk).map_err(TransportError::Sink)?;
        }
        if limit < body.len() {
            return Err(TransportError::Protocol("connection reset".into()));
        }
        Ok(head)
    }
}

/// A release's files: a valid archive for this platform, its checksums and signature.
pub struct ReleaseFiles {
    pub version: String,
    pub archive_name: String,
    pub archive: Vec<u8>,
    pub sums: Vec<u8>,
    pub signature: String,
}

/// The files of a release folder: `(relative path, content)`. The executable is
/// `binary`; `BUILD-INFO.json` is generated with real checksums.
pub fn release_tree(
    version: &str,
    binary: &str,
    extra: &[(&str, &[u8])],
) -> Vec<(String, Vec<u8>)> {
    let platform = platform();
    let executable = format!("executable {version}").into_bytes();
    let mut files: Vec<(String, Vec<u8>)> = vec![(binary.to_owned(), executable.clone())];
    let mut modules = Vec::new();
    for (path, content) in extra {
        files.push(((*path).to_owned(), content.to_vec()));
        modules.push(serde_json::json!({
            "path": path, "bytes": content.len(), "sha256": sha256_hex(content), "executable": false
        }));
    }
    let info = serde_json::json!({
        "version": version, "target": platform.target, "revision": "0".repeat(40),
        "profile": "release", "rust": "1.99.1", "binary": binary,
        "binary_sha256": sha256_hex(&executable),
        "viewer_plugins": [{"id": "test", "files": modules}],
    });
    files.push((
        "BUILD-INFO.json".into(),
        serde_json::to_vec_pretty(&info).unwrap(),
    ));
    files
}

/// Packs `files` below `root/` as this platform's archive format.
pub fn pack(root: &str, files: &[(String, Vec<u8>)]) -> Vec<u8> {
    if platform().archive_suffix == "zip" {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.add_directory(format!("{root}/"), options).unwrap();
            for (path, content) in files {
                zip.start_file(format!("{root}/{path}"), options).unwrap();
                zip.write_all(content).unwrap();
            }
            zip.finish().unwrap();
        }
        buffer.into_inner()
    } else {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(encoder);
        for (path, content) in files {
            // The name goes into the header as is, so tests can build refused names.
            let name = format!("{root}/{path}");
            let mut header = tar::Header::new_gnu();
            header.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name.as_bytes());
            header.set_size(content.len() as u64);
            header.set_mode(if path.ends_with(".exe") || !path.contains('.') {
                0o755
            } else {
                0o644
            });
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            tar.append(&header, &content[..]).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }
}

impl ReleaseFiles {
    pub fn new(key: &TestKey, version: &str, binary: &str) -> Self {
        Self::with_tree(
            key,
            version,
            release_tree(version, binary, &[("viewers/test/a.txt", b"module")]),
        )
    }

    pub fn with_tree(key: &TestKey, version: &str, tree: Vec<(String, Vec<u8>)>) -> Self {
        let platform = platform();
        let parsed = semver::Version::parse(version).unwrap();
        let archive_name = platform.archive_name(&parsed);
        let archive = pack(&platform.archive_root(&parsed), &tree);
        let sums = format!(
            "{}  {archive_name}\n{}  other-file.zip\n",
            sha256_hex(&archive),
            "0".repeat(64)
        )
        .into_bytes();
        let signature = key.sign(
            &sums,
            &format!("timestamp:1 file:SHA256SUMS release:midnight-explorer-{version}"),
        );
        Self {
            version: version.to_owned(),
            archive_name,
            archive,
            sums,
            signature,
        }
    }

    /// GitHub's release-list JSON entry for these files, served from `base`.
    pub fn listing_entry(&self, base: &str, prerelease: bool) -> serde_json::Value {
        serde_json::json!({
            "tag_name": format!("v{}", self.version),
            "name": format!("Midnight Explorer {}", self.version),
            "draft": false,
            "prerelease": prerelease,
            "html_url": format!("{base}/release/v{}", self.version),
            "body": "## Changes\n- **Faster** scrolling\n- See [notes](https://example.invalid)",
            "published_at": "2026-11-23T10:00:00Z",
            "assets": [
                {"name": self.archive_name, "size": self.archive.len(), "browser_download_url": format!("{base}/download/{}", self.archive_name)},
                {"name": "SHA256SUMS", "size": self.sums.len(), "browser_download_url": format!("{base}/download/SHA256SUMS")},
                {"name": "SHA256SUMS.minisig", "size": self.signature.len(), "browser_download_url": format!("{base}/download/SHA256SUMS.minisig")},
            ]
        })
    }

    /// Serves the files on `transport` below `base`.
    pub fn serve(&self, transport: &FakeTransport, base: &str) {
        transport.answer(
            &format!("{base}/download/{}", self.archive_name),
            Answer::ok(self.archive.clone()),
        );
        transport.answer(
            &format!("{base}/download/SHA256SUMS"),
            Answer::ok(self.sums.clone()),
        );
        transport.answer(
            &format!("{base}/download/SHA256SUMS.minisig"),
            Answer::ok(self.signature.clone().into_bytes()),
        );
    }
}

/// Writes `files` into `folder`.
pub fn write_tree(folder: &Path, files: &[(String, Vec<u8>)]) {
    for (path, content) in files {
        let path = folder.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
}
