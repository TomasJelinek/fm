//! Whether other accounts can change an installation, and the two ways a user can make it
//! private without administrator rights: securing the folder or copying it into the profile.
//!
//! Plugins load from a shared installation: whoever can change its files can already replace
//! the application executable, so refusing the plugins would add almost no protection. The
//! application reports the condition once instead. All functions here read or change the
//! file system and run on backend workers.
use crate::{Error, trust};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

/// Shared objects listed by name; the count covers the rest.
const EXAMPLES: usize = 16;
/// Objects inspected or copied at most; the release has a few thousand.
const MAX_ENTRIES: usize = 200_000;

/// What other accounts can change in an installation folder, and what would refuse plugins.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstallationSharing {
    /// The inspected installation folder.
    pub folder: PathBuf,
    /// The first folder above it that other accounts can rename or replace.
    pub shared_ancestor: Option<PathBuf>,
    /// The folder itself and objects in it that other accounts can change (first few).
    pub shared: Vec<PathBuf>,
    /// All such objects in the folder, the folder included.
    pub shared_count: usize,
    /// Objects that make plugins refuse to load: another account's, a link or another
    /// redirection, an unsupported permission entry (first few, with the reason).
    pub refused: Vec<String>,
    /// False when the folder had more objects than are inspected.
    pub complete: bool,
}

impl InstallationSharing {
    /// Nobody but the user, the system and administrators can change the installation.
    pub fn is_private(&self) -> bool {
        self.shared_ancestor.is_none() && self.shared_count == 0 && self.refused.is_empty()
    }
}

/// Inspect `folder`, every folder above it and every object in it, without following links.
pub fn installation_sharing(folder: &Path) -> Result<InstallationSharing, Error> {
    let (own, shared_ancestor) = trust::inspect_split(folder)?;
    let mut report = InstallationSharing {
        folder: folder.to_owned(),
        shared_ancestor,
        complete: true,
        ..Default::default()
    };
    if own == trust::Access::Shared {
        report.shared.push(folder.to_owned());
        report.shared_count = 1;
    }
    let entries = trust::EntryCheck::new()?;
    walk(folder, &mut |path, kind| {
        if kind.is_symlink() {
            note(&mut report.refused, format!("{} is a link", path.display()));
            return Ok(false);
        }
        match entries.inspect(path) {
            Ok(trust::Access::Private) => {}
            Ok(trust::Access::Shared) => {
                report.shared_count += 1;
                if report.shared.len() < EXAMPLES {
                    report.shared.push(path.to_owned());
                }
            }
            Err(error) => note(&mut report.refused, error.0),
        }
        Ok(true)
    })
    .map(|complete| report.complete = complete)?;
    Ok(report)
}

fn note(list: &mut Vec<String>, text: String) {
    if list.len() < EXAMPLES {
        list.push(text);
    }
}

/// Visit every object below `folder` (not `folder` itself) without following links; `visit`
/// returns whether to descend into a folder. Returns false when [`MAX_ENTRIES`] stopped it.
fn walk(
    folder: &Path,
    visit: &mut dyn FnMut(&Path, fs::FileType) -> Result<bool, Error>,
) -> Result<bool, Error> {
    let mut pending = vec![folder.to_owned()];
    let mut count = 0;
    while let Some(directory) = pending.pop() {
        let mut names = Vec::new();
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            names.push((entry.path(), entry.file_type()?));
        }
        names.sort_by(|left, right| left.0.cmp(&right.0));
        for (path, kind) in names {
            count += 1;
            if count > MAX_ENTRIES {
                return Ok(false);
            }
            if visit(&path, kind)? && kind.is_dir() {
                pending.push(path);
            }
        }
    }
    Ok(true)
}

/// Make the installation `folder` private, then inspect it again.
///
/// Windows: only for a folder the user owns. The folder inherits its permissions from its
/// parent again when the parent passes on nothing that other accounts can change (this also
/// repairs entries a move kept from a former location); otherwise it gets its own protected
/// permissions: the user, SYSTEM and Administrators with full control. Every object below then
/// inherits them, keeping its own entries except those that let other accounts change it
/// (such as the restricted workers' read and execute grants). Linux: removes the group and
/// other write bits from the folder and every object in it that the user owns, each through a
/// descriptor opened without following links and only at its canonical location.
///
/// Folders above the installation are never changed; the returned report names one that other
/// accounts can still rename or replace, and objects that could not be changed.
pub fn secure_installation(folder: &Path) -> Result<SecuredInstallation, Error> {
    trust::repair::require_owner(folder)?;
    // Each object is changed only at its final path below this one, so that a folder swapped
    // for a junction or link meanwhile cannot redirect the change out of the installation.
    let canonical = folder.canonicalize()?;
    let mut failures = Vec::new();
    // Objects the folder-wide change could not reach are repaired one by one below.
    if let Err(error) = trust::repair::secure_folder(folder) {
        note(&mut failures, format!("{}: {}", folder.display(), error.0));
    }
    walk(folder, &mut |path, kind| {
        if kind.is_symlink() {
            return Ok(false);
        }
        let expected = canonical.join(
            path.strip_prefix(folder)
                .map_err(|_| Error("Installation entry outside its folder".into()))?,
        );
        if let Err(error) = trust::repair::secure_entry(path, &expected) {
            note(&mut failures, format!("{}: {}", path.display(), error.0));
            // Nothing below an object that could not be secured is visited.
            return Ok(false);
        }
        Ok(true)
    })?;
    Ok(SecuredInstallation {
        sharing: installation_sharing(folder)?,
        failures,
    })
}

/// The result of [`secure_installation`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecuredInstallation {
    /// The installation after the change.
    pub sharing: InstallationSharing,
    /// Objects whose permissions could not be changed, with the reason (first few).
    pub failures: Vec<String>,
}

/// Where [`copy_installation`] puts a private copy in the user's profile:
/// `%LOCALAPPDATA%\Programs\Midnight Explorer` on Windows and
/// `$XDG_DATA_HOME/midnight-explorer/app` (by default `~/.local/share/...`) on Linux.
pub fn profile_installation_folder() -> Option<PathBuf> {
    let absolute = |path: PathBuf| path.is_absolute().then_some(path);
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .and_then(absolute)
            .map(|base| base.join("Programs").join("Midnight Explorer"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .and_then(absolute)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| PathBuf::from(home).join(".local").join("share"))
                    .and_then(absolute)
            })
            .map(|base| base.join("midnight-explorer").join("app"))
    }
}

/// Why [`copy_installation`] did not copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopyInstallationError {
    /// The destination exists; nothing was changed. Ask before replacing it.
    DestinationExists(PathBuf),
    /// The copy failed; the text says why. A partial copy is removed.
    Failed(String),
}

impl std::fmt::Display for CopyInstallationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DestinationExists(path) => write!(formatter, "{} already exists", path.display()),
            Self::Failed(reason) => formatter.write_str(reason),
        }
    }
}

impl From<Error> for CopyInstallationError {
    fn from(error: Error) -> Self {
        Self::Failed(error.0)
    }
}

impl From<io::Error> for CopyInstallationError {
    fn from(error: io::Error) -> Self {
        Self::Failed(error.to_string())
    }
}

/// The result of [`copy_installation`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopiedInstallation {
    /// The new installation folder.
    pub folder: PathBuf,
    /// The copied files, each verified by size and SHA-256.
    pub files: u64,
    /// Their total size in bytes.
    pub bytes: u64,
    /// Whether other accounts can change the copy; restart from it only when private.
    pub sharing: InstallationSharing,
}

/// Copy the complete installation `source` to `destination`, verify every copied file by
/// size and SHA-256 and inspect the copy's permissions. The copy is made in a new sibling
/// folder first, so each file inherits the destination's permissions (a same-volume move
/// would keep the source's); it replaces an existing `destination` only when `replace` is
/// set, and is removed when anything fails. The source is never changed.
pub fn copy_installation(
    source: &Path,
    destination: &Path,
    replace: bool,
) -> Result<CopiedInstallation, CopyInstallationError> {
    let failed = |text: String| CopyInstallationError::Failed(text);
    if !source.is_absolute() || !destination.is_absolute() {
        return Err(failed("Installation paths must be absolute".into()));
    }
    let name = destination
        .file_name()
        .ok_or_else(|| failed("The destination has no folder name".into()))?;
    let parent = destination
        .parent()
        .ok_or_else(|| failed("The destination has no parent folder".into()))?;
    fs::create_dir_all(parent)?;
    let canonical_source = source.canonicalize()?;
    let canonical_parent = parent.canonicalize()?;
    let canonical_destination = canonical_parent.join(name);
    if canonical_destination.starts_with(&canonical_source)
        || canonical_source.starts_with(&canonical_destination)
    {
        return Err(failed(
            "Midnight Explorer is already installed in this folder".into(),
        ));
    }
    let exists = match fs::symlink_metadata(&canonical_destination) {
        Ok(_) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    if exists && !replace {
        return Err(CopyInstallationError::DestinationExists(
            destination.to_owned(),
        ));
    }
    let mut staging_name = name.to_owned();
    staging_name.push(format!(".partial-{}", std::process::id()));
    let staging = canonical_parent.join(staging_name);
    if fs::symlink_metadata(&staging).is_ok() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir(&staging)?;
    let published = trust::repair::copy_permissions(&canonical_source, &staging)
        .and_then(|()| copy_tree(&canonical_source, &staging))
        .and_then(|counts| {
            publish(&staging, &canonical_destination, exists)?;
            Ok(counts)
        });
    let (files, bytes) = match published {
        Ok(counts) => counts,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error.into());
        }
    };
    Ok(CopiedInstallation {
        sharing: installation_sharing(&canonical_destination)?,
        folder: canonical_destination,
        files,
        bytes,
    })
}

/// Rename the verified staging copy into place, moving an existing destination aside first
/// and removing it after; the previous installation is restored when the rename fails.
fn publish(staging: &Path, destination: &Path, exists: bool) -> Result<(), Error> {
    if !exists {
        return Ok(fs::rename(staging, destination)?);
    }
    let mut previous_name = destination.file_name().unwrap_or_default().to_owned();
    previous_name.push(format!(".previous-{}", std::process::id()));
    let previous = destination.with_file_name(previous_name);
    fs::rename(destination, &previous).map_err(|error| {
        Error(format!(
            "{} cannot be replaced (is Midnight Explorer running from it?): {error}",
            destination.display()
        ))
    })?;
    if let Err(error) = fs::rename(staging, destination) {
        let _ = fs::rename(&previous, destination);
        return Err(error.into());
    }
    // The new copy is complete; a previous file still in use stays behind harmlessly.
    let _ = fs::remove_dir_all(&previous);
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(u64, u64), Error> {
    let mut files = 0;
    let mut bytes = 0;
    let complete = walk(source, &mut |path, kind| {
        let relative = path
            .strip_prefix(source)
            .map_err(|_| Error("Installation entry outside its folder".into()))?;
        let target = destination.join(relative);
        if kind.is_symlink() {
            return Err(Error(format!(
                "{} is a link; copy the installation without it",
                path.display()
            )));
        }
        if kind.is_dir() {
            fs::create_dir(&target)?;
            trust::repair::copy_permissions(path, &target)?;
            return Ok(true);
        }
        if !kind.is_file() {
            return Err(Error(format!("{} is not a regular file", path.display())));
        }
        bytes += copy_file(path, &target)?;
        files += 1;
        Ok(false)
    })?;
    if !complete {
        return Err(Error("The installation has too many files to copy".into()));
    }
    Ok((files, bytes))
}

/// Copy one file while hashing it, then read the copy back and compare size and SHA-256.
fn copy_file(source: &Path, target: &Path) -> Result<u64, Error> {
    let mut input = File::open(source)?;
    let mut output = File::options().write(true).create_new(true).open(target)?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    let mut length = 0_u64;
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
        length += count as u64;
    }
    output.sync_all()?;
    drop(output);
    let expected = hash.finalize();
    trust::repair::copy_permissions(source, target)?;
    let mut copy = File::open(target)?;
    let mut hash = Sha256::new();
    let mut copied = 0_u64;
    loop {
        let count = copy.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        copied += count as u64;
    }
    if copied != length || hash.finalize() != expected || input.metadata()?.len() != length {
        return Err(Error(format!(
            "The copy of {} does not match the original",
            source.display()
        )));
    }
    Ok(length)
}

#[cfg(test)]
#[path = "installation_sharing_tests.rs"]
mod tests;
