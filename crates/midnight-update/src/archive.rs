//! Extracting a verified release archive into a staging folder, and checking the
//! staged copy against its own `BUILD-INFO.json`.
use crate::{
    platform,
    release::Platform,
    trust::{decode_hex, sha256_file},
};
use semver::Version;
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

/// An archive holds at most this many entries and this many uncompressed bytes.
pub const MAX_ENTRIES: usize = 65_536;
pub const MAX_UNPACKED_BYTES: u64 = 6 * 1024 * 1024 * 1024;

/// Why a downloaded release cannot be staged.
#[derive(Debug, PartialEq, Eq)]
pub enum StageError {
    /// The archive is damaged or not of the expected kind.
    Unreadable(String),
    /// An entry is outside the release folder, a link, or otherwise not allowed.
    BadEntry(String),
    TooLarge,
    /// `BUILD-INFO.json` is missing or damaged.
    NoBuildInfo(String),
    /// `BUILD-INFO.json` names another version or target than the release.
    WrongBuild {
        expected: String,
        found: String,
    },
    /// A staged file differs from its checksum in `BUILD-INFO.json`.
    FileMismatch(String),
    Disk(String),
}

impl fmt::Display for StageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(detail) => write!(f, "The release archive cannot be read: {detail}"),
            Self::BadEntry(name) => write!(f, "The release archive holds a refused entry: {name}"),
            Self::TooLarge => write!(f, "The release archive unpacks to more than allowed."),
            Self::NoBuildInfo(detail) => {
                write!(f, "The release has no valid BUILD-INFO.json: {detail}")
            }
            Self::WrongBuild { expected, found } => write!(
                f,
                "The release archive holds {found}, not the offered {expected}."
            ),
            Self::FileMismatch(path) => write!(
                f,
                "{path} in the release does not match its checksum in BUILD-INFO.json."
            ),
            Self::Disk(detail) => write!(f, "The update cannot be unpacked: {detail}"),
        }
    }
}

fn disk(error: io::Error) -> StageError {
    StageError::Disk(error.to_string())
}

/// The fields of `BUILD-INFO.json` the updater relies on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildInfo {
    pub version: Version,
    pub target: String,
    /// The executable's file name beside `BUILD-INFO.json` (`me.exe`, `midnight-explorer`).
    pub binary: String,
    pub binary_sha256: Option<[u8; 32]>,
    raw: serde_json::Value,
}

pub const BUILD_INFO: &str = "BUILD-INFO.json";

impl BuildInfo {
    /// Reads `<folder>/BUILD-INFO.json`.
    pub fn read(folder: &Path) -> Result<Self, StageError> {
        let path = folder.join(BUILD_INFO);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| StageError::NoBuildInfo(error.to_string()))?;
        if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
            return Err(StageError::NoBuildInfo("not a regular file".into()));
        }
        let text = fs::read(&path).map_err(|error| StageError::NoBuildInfo(error.to_string()))?;
        Self::parse(&text)
    }

    pub fn parse(text: &[u8]) -> Result<Self, StageError> {
        let raw: serde_json::Value = serde_json::from_slice(text)
            .map_err(|error| StageError::NoBuildInfo(error.to_string()))?;
        let field = |name: &str| {
            raw.get(name)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| StageError::NoBuildInfo(format!("no \"{name}\"")))
        };
        let version = Version::parse(&field("version")?)
            .map_err(|error| StageError::NoBuildInfo(error.to_string()))?;
        let target = field("target")?;
        let binary = field("binary")?;
        if !portable_name(&binary) {
            return Err(StageError::NoBuildInfo(format!(
                "invalid binary name {binary}"
            )));
        }
        let binary_sha256 = match raw.get("binary_sha256") {
            None => None,
            Some(value) => Some(
                value
                    .as_str()
                    .and_then(decode_hex)
                    .ok_or_else(|| StageError::NoBuildInfo("invalid binary_sha256".into()))?,
            ),
        };
        Ok(Self {
            version,
            target,
            binary,
            binary_sha256,
            raw,
        })
    }

    /// Every `{path, sha256}` record of the modules, paths relative to the folder.
    fn module_files(&self) -> Vec<(String, String, Option<u64>)> {
        let mut found = Vec::new();
        collect_files(&self.raw, &mut found);
        found
    }
}

fn collect_files(value: &serde_json::Value, found: &mut Vec<(String, String, Option<u64>)>) {
    match value {
        serde_json::Value::Object(map) => {
            if let (Some(path), Some(sha256)) = (
                map.get("path").and_then(serde_json::Value::as_str),
                map.get("sha256").and_then(serde_json::Value::as_str),
            ) {
                let bytes = map.get("bytes").and_then(serde_json::Value::as_u64);
                found.push((path.to_owned(), sha256.to_owned(), bytes));
            }
            for child in map.values() {
                collect_files(child, found);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                collect_files(child, found);
            }
        }
        _ => {}
    }
}

/// A file or folder name that is the same on every platform: no separators, no
/// `.`/`..`, no characters Windows refuses.
pub(crate) fn portable_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.ends_with(['.', ' '])
        && !name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
}

/// The relative path below the release folder for an archive entry `name`, `None` for
/// the release folder itself. Refuses anything outside it.
fn entry_path(name: &str, root: &str) -> Result<Option<PathBuf>, StageError> {
    let refused = || StageError::BadEntry(name.to_owned());
    let trimmed = name.trim_end_matches('/');
    let mut parts = trimmed.split('/');
    if parts.next() != Some(root) {
        return Err(refused());
    }
    let mut relative = PathBuf::new();
    for part in parts {
        if !portable_name(part) {
            return Err(refused());
        }
        relative.push(part);
    }
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(refused());
    }
    Ok((!relative.as_os_str().is_empty()).then_some(relative))
}

struct Budget {
    entries: usize,
    bytes: u64,
}

impl Budget {
    fn take(&mut self, size: u64) -> Result<(), StageError> {
        self.entries += 1;
        self.bytes = self.bytes.saturating_add(size);
        if self.entries > MAX_ENTRIES || self.bytes > MAX_UNPACKED_BYTES {
            return Err(StageError::TooLarge);
        }
        Ok(())
    }
}

fn write_file(
    destination: &Path,
    relative: &Path,
    reader: &mut dyn Read,
    size: u64,
    executable: bool,
) -> Result<(), StageError> {
    let path = destination.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(disk)?;
    }
    // A name that appears twice is refused rather than overwritten.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                StageError::BadEntry(relative.display().to_string())
            } else {
                disk(error)
            }
        })?;
    let copied = io::copy(&mut reader.take(size.saturating_add(1)), &mut file).map_err(disk)?;
    if copied != size {
        return Err(StageError::Unreadable(format!(
            "{} has the wrong size",
            relative.display()
        )));
    }
    file.flush().map_err(disk)?;
    drop(file);
    platform::set_executable(&path, executable).map_err(disk)
}

/// Extracts `archive` (`zip` or `tar.gz` after `platform`) into `destination`, which
/// must not exist, keeping only what is below the release folder `root`.
pub fn extract(
    archive: &Path,
    platform: &Platform,
    root: &str,
    destination: &Path,
) -> Result<(), StageError> {
    fs::create_dir(destination).map_err(disk)?;
    let file = File::open(archive).map_err(disk)?;
    let mut budget = Budget {
        entries: 0,
        bytes: 0,
    };
    if platform.archive_suffix == "zip" {
        extract_zip(file, root, destination, &mut budget)
    } else {
        extract_tar_gz(file, root, destination, &mut budget)
    }
}

fn extract_zip(
    file: File,
    root: &str,
    destination: &Path,
    budget: &mut Budget,
) -> Result<(), StageError> {
    let unreadable = |error: zip::result::ZipError| StageError::Unreadable(error.to_string());
    let mut archive = zip::ZipArchive::new(io::BufReader::new(file)).map_err(unreadable)?;
    if archive.len() > MAX_ENTRIES {
        return Err(StageError::TooLarge);
    }
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(unreadable)?;
        let name = entry.name().to_owned();
        if entry.is_symlink() || !(entry.is_dir() || entry.is_file()) {
            return Err(StageError::BadEntry(name));
        }
        budget.take(entry.size())?;
        let Some(relative) = entry_path(&name, root)? else {
            continue;
        };
        if entry.is_dir() {
            fs::create_dir_all(destination.join(&relative)).map_err(disk)?;
            continue;
        }
        let executable = entry.unix_mode().is_some_and(|mode| mode & 0o111 != 0);
        let size = entry.size();
        write_file(destination, &relative, &mut entry, size, executable)?;
    }
    Ok(())
}

fn extract_tar_gz(
    file: File,
    root: &str,
    destination: &Path,
    budget: &mut Budget,
) -> Result<(), StageError> {
    let unreadable = |error: io::Error| StageError::Unreadable(error.to_string());
    let decoder = flate2::read::GzDecoder::new(io::BufReader::new(file));
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries().map_err(unreadable)? {
        let mut entry = entry.map_err(unreadable)?;
        let name = String::from_utf8(entry.path_bytes().into_owned())
            .map_err(|_| StageError::BadEntry("a name that is not UTF-8".into()))?;
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            return Err(StageError::BadEntry(name));
        }
        let size = entry.size();
        budget.take(size)?;
        let Some(relative) = entry_path(&name, root)? else {
            continue;
        };
        if kind.is_dir() {
            fs::create_dir_all(destination.join(&relative)).map_err(disk)?;
            continue;
        }
        let executable = entry.header().mode().is_ok_and(|mode| mode & 0o111 != 0);
        write_file(destination, &relative, &mut entry, size, executable)?;
    }
    Ok(())
}

/// Checks a staged release: its `BUILD-INFO.json` names `version` and the platform's
/// target, the executable exists and matches `binary_sha256`, and every module file
/// matches its recorded size and SHA-256.
pub fn verify_staged(
    folder: &Path,
    version: &Version,
    platform: &Platform,
) -> Result<BuildInfo, StageError> {
    let info = BuildInfo::read(folder)?;
    if info.version != *version || info.target != platform.target {
        return Err(StageError::WrongBuild {
            expected: format!("{version} for {}", platform.target),
            found: format!("{} for {}", info.version, info.target),
        });
    }
    let binary = folder.join(&info.binary);
    let metadata = fs::symlink_metadata(&binary).map_err(|_| {
        StageError::NoBuildInfo(format!("the executable {} is missing", info.binary))
    })?;
    if !metadata.is_file() {
        return Err(StageError::BadEntry(info.binary.clone()));
    }
    if let Some(expected) = info.binary_sha256
        && sha256_file(&binary).map_err(disk)? != expected
    {
        return Err(StageError::FileMismatch(info.binary.clone()));
    }
    for (path, sha256, bytes) in info.module_files() {
        let relative = path
            .split('/')
            .try_fold(PathBuf::new(), |mut relative, part| {
                portable_name(part).then(|| {
                    relative.push(part);
                    relative
                })
            })
            .ok_or_else(|| StageError::BadEntry(path.clone()))?;
        let file = folder.join(&relative);
        let metadata =
            fs::symlink_metadata(&file).map_err(|_| StageError::FileMismatch(path.clone()))?;
        if !metadata.is_file() || bytes.is_some_and(|bytes| bytes != metadata.len()) {
            return Err(StageError::FileMismatch(path));
        }
        let expected = decode_hex(&sha256).ok_or_else(|| StageError::FileMismatch(path.clone()))?;
        if sha256_file(&file).map_err(disk)? != expected {
            return Err(StageError::FileMismatch(path));
        }
    }
    Ok(info)
}
