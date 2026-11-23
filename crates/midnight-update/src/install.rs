//! The installation folder: whether this copy can update itself, the `.update` work
//! folder inside it, the instance lock, the staged release and the update markers.
//!
//! Layout of `<installation>/.update/`:
//! - `instances.lock`: every running copy holds a shared lock; the updater waits for an
//!   exclusive one, so no copy runs from the folder while its files move.
//! - `staging/<release folder>/`: the verified new version, `staging/ready.json` names it.
//! - `backup/`: the previous version's files while the new one proves it starts.
//! - `failed/`: a new version's files after a rollback, removed at the next start.
//! - `updater/`: a copy of the previous executable that runs the swap.
//! - `pending.json`, `confirmed`, `result.json`, `update.log`: see `apply`.
use crate::{
    archive::{BUILD_INFO, BuildInfo},
    release::Platform,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

pub const WORK_FOLDER: &str = ".update";

/// Where the running copy is installed and whether it can replace itself.
#[derive(Clone, Debug)]
pub struct Installation {
    pub folder: PathBuf,
    pub build: BuildInfo,
}

/// Why this copy cannot install an update itself; the release page is offered instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotUpdatable {
    /// No `BUILD-INFO.json` beside the executable: a development build or a package
    /// that another installer manages.
    NotAReleaseFolder,
    /// `BUILD-INFO.json` names another target or executable.
    Mismatch(String),
    /// The folder cannot be changed by this user (installed by an administrator).
    ReadOnly(PathBuf),
}

impl std::fmt::Display for NotUpdatable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAReleaseFolder => write!(
                f,
                "This copy is not an extracted release folder, so it cannot update itself."
            ),
            Self::Mismatch(detail) => write!(f, "This copy cannot update itself: {detail}"),
            Self::ReadOnly(folder) => write!(
                f,
                "This copy is installed in a folder you cannot change ({}). Download the new version from the release page, or ask an administrator to replace it.",
                folder.display()
            ),
        }
    }
}

impl Installation {
    /// The installation of `executable`: its folder must hold a `BUILD-INFO.json`
    /// for this platform that names this executable, and must be writable.
    pub fn detect(executable: &Path, platform: &Platform) -> Result<Self, NotUpdatable> {
        let folder = executable
            .parent()
            .ok_or(NotUpdatable::NotAReleaseFolder)?
            .to_path_buf();
        if !folder.join(BUILD_INFO).is_file() {
            return Err(NotUpdatable::NotAReleaseFolder);
        }
        let build =
            BuildInfo::read(&folder).map_err(|error| NotUpdatable::Mismatch(error.to_string()))?;
        if build.target != platform.target {
            return Err(NotUpdatable::Mismatch(format!(
                "it was built for {}",
                build.target
            )));
        }
        let running = executable.file_name().and_then(|name| name.to_str());
        if running != Some(build.binary.as_str()) {
            return Err(NotUpdatable::Mismatch(format!(
                "BUILD-INFO.json names {}, not the running executable",
                build.binary
            )));
        }
        if !writable(&folder) {
            return Err(NotUpdatable::ReadOnly(folder));
        }
        Ok(Self { folder, build })
    }

    pub fn work(&self) -> PathBuf {
        work_folder(&self.folder)
    }

    pub fn staging(&self) -> PathBuf {
        self.work().join("staging")
    }
}

pub fn work_folder(installation: &Path) -> PathBuf {
    installation.join(WORK_FOLDER)
}

/// Whether this user can create and remove files in `folder`, tried for real.
fn writable(folder: &Path) -> bool {
    let work = work_folder(folder);
    if fs::create_dir_all(&work).is_err() {
        return false;
    }
    let probe = work.join(format!("write-test-{}", std::process::id()));
    let created = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .is_ok();
    let _ = fs::remove_file(&probe);
    // The folder itself must allow renaming its entries; a test entry proves it.
    let entry = folder.join(format!(".update-test-{}", std::process::id()));
    let renamed = fs::create_dir(&entry).is_ok() && {
        let moved = work.join("rename-test");
        let ok = fs::rename(&entry, &moved).is_ok();
        let _ = fs::remove_dir(&moved);
        ok
    };
    let _ = fs::remove_dir(&entry);
    created && renamed
}

/// The shared lock every running copy holds on its installation.
pub struct InstanceLock {
    _file: File,
}

impl InstanceLock {
    /// Takes the shared lock in a release folder (one with `BUILD-INFO.json`) that this
    /// user can change; `None` elsewhere or while an updater holds the folder.
    pub fn acquire(installation: &Path) -> Option<Self> {
        if !installation.join(BUILD_INFO).is_file() {
            return None;
        }
        let work = work_folder(installation);
        fs::create_dir_all(&work).ok()?;
        let file = lock_file(&work).ok()?;
        file.try_lock_shared().ok()?;
        Some(Self { _file: file })
    }
}

pub(crate) fn lock_file(work: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(work.join("instances.lock"))
}

/// The staged release that is ready to install.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Staged {
    pub version: String,
    /// The release folder's name inside `staging/`.
    pub folder: String,
    pub page_url: String,
}

impl Staged {
    pub fn version(&self) -> Option<Version> {
        Version::parse(&self.version).ok()
    }
}

pub fn read_staged(installation: &Installation) -> Option<Staged> {
    let text = fs::read(installation.staging().join("ready.json")).ok()?;
    let staged: Staged = serde_json::from_slice(&text).ok()?;
    installation
        .staging()
        .join(&staged.folder)
        .is_dir()
        .then_some(staged)
}

pub(crate) fn write_staged(installation: &Installation, staged: &Staged) -> io::Result<()> {
    write_json(&installation.staging().join("ready.json"), staged)
}

pub(crate) fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let text = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    let partial = path.with_extension("tmp");
    {
        let mut file = File::create(&partial)?;
        file.write_all(&text)?;
        file.sync_all()?;
    }
    fs::rename(&partial, path)
}

/// Removes a folder tree, tolerating its absence.
pub(crate) fn remove_tree(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}
