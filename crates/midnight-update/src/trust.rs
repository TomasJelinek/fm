//! The signature model: the release's `SHA256SUMS` is signed with the project's update
//! key (minisign, Ed25519, pre-hashed), whose public half is compiled in.
use semver::Version;
use sha2::{Digest, Sha256};
use std::{
    fmt,
    fs::File,
    io::{self, Read},
    path::Path,
};

/// The public update key, from `packaging/update-signing.pub`. A build whose file holds
/// no key line (the placeholder) installs no update.
const BUILT_IN_KEY: &str = include_str!("../../../packaging/update-signing.pub");

/// The most bytes of `SHA256SUMS` and of its signature that are read.
pub const MAX_SUMS_BYTES: u64 = 64 * 1024;
pub const MAX_SIGNATURE_BYTES: u64 = 4 * 1024;

/// The public key that every installed update must be signed with.
#[derive(Clone, Debug)]
pub struct TrustAnchor {
    key: minisign_verify::PublicKey,
}

impl TrustAnchor {
    /// The key compiled into this build, `None` when the build has none. In debug
    /// builds only, `MIDNIGHT_UPDATE_TEST_PUBLIC_KEY` names a key file that replaces it
    /// for local end-to-end tests; release builds ignore the variable.
    pub fn built_in() -> Option<Self> {
        #[cfg(debug_assertions)]
        if let Some(path) = std::env::var_os("MIDNIGHT_UPDATE_TEST_PUBLIC_KEY") {
            return std::fs::read_to_string(path)
                .ok()
                .and_then(|text| Self::parse(&text).ok().flatten());
        }
        Self::parse(BUILT_IN_KEY).ok().flatten()
    }

    /// Parses a minisign public key file. Comment-only text (the placeholder) is `None`.
    pub fn parse(text: &str) -> Result<Option<Self>, TrustError> {
        let Some(line) = text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("untrusted comment:"))
        else {
            return Ok(None);
        };
        if line.starts_with('#') {
            return Ok(None);
        }
        minisign_verify::PublicKey::from_base64(line)
            .map(|key| Some(Self { key }))
            .map_err(|_| TrustError::BadKey)
    }
}

/// Why a download was refused. Every variant names the check that failed.
#[derive(Debug, PartialEq, Eq)]
pub enum TrustError {
    /// This build has no update key.
    NoKey,
    /// The compiled-in key is not a minisign Ed25519 key.
    BadKey,
    /// The signature file is not a minisign signature.
    MalformedSignature,
    /// Signed with a different key than this build trusts.
    WrongKey,
    /// The signature does not match `SHA256SUMS` (changed after signing, or forged).
    BadSignature,
    /// A legacy (not pre-hashed) signature; only pre-hashed ones are accepted.
    LegacySignature,
    /// The signed comment names another release than the one offered.
    WrongRelease { expected: String, found: String },
    /// `SHA256SUMS` is not a list of `<sha256>  <file name>` lines.
    MalformedSums,
    /// `SHA256SUMS` has no line for this file.
    NotListed(String),
    /// The file's SHA-256 differs from its signed line.
    DigestMismatch(String),
    /// Reading the downloaded file failed.
    Read(String),
}

impl fmt::Display for TrustError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoKey => write!(
                f,
                "This build has no update signing key, so it installs no updates."
            ),
            Self::BadKey => write!(f, "This build's update signing key is invalid."),
            Self::MalformedSignature => write!(f, "The release's signature file is damaged."),
            Self::WrongKey => write!(
                f,
                "The release is signed with a key this version does not trust."
            ),
            Self::BadSignature => write!(
                f,
                "The release's checksum list does not match its signature."
            ),
            Self::LegacySignature => write!(f, "The release uses an outdated signature format."),
            Self::WrongRelease { expected, found } => write!(
                f,
                "The signature belongs to another release ({found}), not {expected}."
            ),
            Self::MalformedSums => write!(f, "The release's checksum list is damaged."),
            Self::NotListed(name) => write!(f, "The signed checksum list does not name {name}."),
            Self::DigestMismatch(name) => write!(
                f,
                "{name} does not match its signed checksum: the download is damaged or was changed."
            ),
            Self::Read(detail) => write!(f, "The download cannot be read: {detail}"),
        }
    }
}

/// The marker the signed comment carries for `version`: `release:midnight-explorer-0.5.1`.
pub fn release_marker(version: &Version) -> String {
    format!("release:midnight-explorer-{version}")
}

/// A `SHA256SUMS` whose signature verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedSums {
    entries: Vec<(String, [u8; 32])>,
}

impl SignedSums {
    pub fn digest(&self, name: &str) -> Option<[u8; 32]> {
        self.entries
            .iter()
            .find(|(entry, _)| entry == name)
            .map(|(_, digest)| *digest)
    }

    /// Checks that `path`'s SHA-256 equals the signed line for `name`.
    pub fn check_file(&self, name: &str, path: &Path) -> Result<(), TrustError> {
        let expected = self
            .digest(name)
            .ok_or_else(|| TrustError::NotListed(name.to_owned()))?;
        let actual = sha256_file(path).map_err(|error| TrustError::Read(error.to_string()))?;
        if actual == expected {
            Ok(())
        } else {
            Err(TrustError::DigestMismatch(name.to_owned()))
        }
    }
}

/// Verifies `signature` over `sums` with `anchor` and that its trusted comment names
/// `version`, then parses the list.
pub fn verify_sums(
    anchor: Option<&TrustAnchor>,
    sums: &[u8],
    signature: &str,
    version: &Version,
) -> Result<SignedSums, TrustError> {
    let anchor = anchor.ok_or(TrustError::NoKey)?;
    let signature = minisign_verify::Signature::decode(signature)
        .map_err(|_| TrustError::MalformedSignature)?;
    anchor
        .key
        .verify(sums, &signature, false)
        .map_err(|error| match error {
            minisign_verify::Error::UnexpectedKeyId => TrustError::WrongKey,
            minisign_verify::Error::UnexpectedAlgorithm => TrustError::LegacySignature,
            _ => TrustError::BadSignature,
        })?;
    let marker = release_marker(version);
    let comment = signature.trusted_comment();
    if !comment.split_whitespace().any(|token| token == marker) {
        let found = comment
            .split_whitespace()
            .find_map(|token| token.strip_prefix("release:"))
            .unwrap_or("none")
            .to_owned();
        return Err(TrustError::WrongRelease {
            expected: format!("midnight-explorer-{version}"),
            found,
        });
    }
    parse_sums(sums)
}

/// Parses `<64 hex digits>  <name>` (or ` *<name>`) lines; a name appears once.
pub fn parse_sums(sums: &[u8]) -> Result<SignedSums, TrustError> {
    let text = std::str::from_utf8(sums).map_err(|_| TrustError::MalformedSums)?;
    let mut entries: Vec<(String, [u8; 32])> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let (hex, rest) = line.split_at_checked(64).ok_or(TrustError::MalformedSums)?;
        let name = rest
            .strip_prefix("  ")
            .or_else(|| rest.strip_prefix(" *"))
            .ok_or(TrustError::MalformedSums)?;
        let digest = decode_hex(hex).ok_or(TrustError::MalformedSums)?;
        if name.is_empty()
            || name.contains(['/', '\\'])
            || entries.iter().any(|(entry, _)| entry == name)
        {
            return Err(TrustError::MalformedSums);
        }
        entries.push((name.to_owned(), digest));
    }
    if entries.is_empty() {
        return Err(TrustError::MalformedSums);
    }
    Ok(SignedSums { entries })
}

pub(crate) fn decode_hex(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

pub(crate) fn sha256_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    sha256_reader(&mut file)
}

pub(crate) fn sha256_reader(reader: &mut dyn Read) -> io::Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}
