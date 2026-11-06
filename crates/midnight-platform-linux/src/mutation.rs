//! Native mutations retain inode handles and parent directory anchors.
//!
//! Linux has no conditional unlink/rename by inode. We first move the selected
//! name, without replacement, into a newly created owner-only directory and
//! verify its identity there. A raced replacement is restored without overwrite;
//! if restoration collides it remains in staging and the error gives its path.
//! This prevents deleting a replacement raced into the original parent. It is
//! not an isolation boundary against root or another process of the same uid,
//! nor against writers already holding directory handles inside the selected
//! tree. Such writers can still race individual directory entries. We recheck
//! identities before unlink, never follow links, and reject mount crossings.
//! Cancellation and errors restore the remaining tree where possible; already
//! unlinked descendants cannot be restored. A crash may leave staging behind.
//! renameat2(RENAME_NOREPLACE) support is required. Recursive operations use
//! openat2 (Linux 5.6+) and fail explicitly if these kernel guarantees are absent.
//! Preparation retains two file descriptors per selected item; exhaustion fails
//! preparation. Traversal additionally uses descriptors proportional to depth.

use midnight_core::{
    Cancellation, DeleteMode, EntryKind, EntryMetadata, FsError, MutationItem, MutationService,
    PreparedDirectory, PreparedMutation,
};
use rustix::fs::{
    AtFlags, CWD, FileType, Mode, OFlags, RenameFlags, ResolveFlags, Stat, fstat, mkdirat, openat,
    openat2, renameat_with, statat, unlinkat,
};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, ErrorKind},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

mod create;
mod trash;
mod tree;

const OPERATION: &str = "mutate entry";
const STAGED_NAME: &str = "item";
const DIRECTORY_FLAGS: OFlags = OFlags::PATH.union(OFlags::DIRECTORY).union(OFlags::CLOEXEC);

impl MutationService for crate::LinuxFileSystem {
    fn prepare_create(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn PreparedDirectory>, FsError> {
        Ok(Arc::new(create::Directory::prepare(path, cancel)?))
    }

    fn prepare(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn PreparedMutation>, FsError> {
        let home = self.home.clone().or_else(crate::home_directory);
        let trash = trash::home_trash(home.as_deref(), std::env::var_os("XDG_DATA_HOME"));
        Ok(Arc::new(Prepared::new(path, trash, cancel)?))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    kind: FileType,
}

impl Identity {
    fn from_stat(stat: &Stat) -> Self {
        Self {
            device: stat.st_dev,
            inode: stat.st_ino,
            kind: FileType::from_raw_mode(stat.st_mode),
        }
    }

    fn of(file: &File, path: &Path) -> Result<Self, FsError> {
        Ok(Self::from_stat(
            &fstat(file).map_err(|error| native_error(path, error))?,
        ))
    }

    fn verify(self, parent: &File, name: &OsStr, path: &Path) -> Result<(), FsError> {
        let current = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| native_error(path, error))?;
        if self != Self::from_stat(&current) {
            return Err(error(
                path,
                ErrorKind::InvalidInput,
                "the selected entry changed",
            ));
        }
        Ok(())
    }
}

struct Prepared {
    item: MutationItem,
    parent_path: PathBuf,
    name: OsString,
    parent: Arc<File>,
    parent_identity: Identity,
    identity: Identity,
    // O_PATH pins the inode even after unlink, preventing inode-number reuse.
    _source: File,
    trash_root: Option<PathBuf>,
}

impl Prepared {
    fn new(
        path: &Path,
        trash_root: Option<PathBuf>,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        check_cancel(cancel, path)?;
        if path
            .as_os_str()
            .as_bytes()
            .split(|byte| *byte == b'/')
            .any(|component| component == b"." || component == b"..")
        {
            return Err(error(
                path,
                ErrorKind::InvalidInput,
                "dot path components are not mutable",
            ));
        }
        let path = std::path::absolute(path).map_err(|e| FsError::from_io(OPERATION, path, e))?;
        let name = path
            .file_name()
            .ok_or_else(|| {
                error(
                    &path,
                    ErrorKind::InvalidInput,
                    "filesystem roots are not mutable",
                )
            })?
            .to_owned();
        validate_name(&name, &path)?;
        let parent_path = path
            .parent()
            .expect("absolute non-root has a parent")
            .to_owned();
        let parent = File::from(
            openat(CWD, &parent_path, DIRECTORY_FLAGS, Mode::empty())
                .map_err(|e| native_error(&parent_path, e))?,
        );
        let parent_identity = Identity::of(&parent, &parent_path)?;
        let source = File::from(
            openat2(
                &parent,
                &name,
                OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV,
            )
            .map_err(|e| native_error(&path, e))?,
        );
        let stat = fstat(&source).map_err(|e| native_error(&path, e))?;
        let identity = Identity::from_stat(&stat);
        if identity == parent_identity || identity.device != parent_identity.device {
            return Err(error(
                &path,
                ErrorKind::InvalidInput,
                "filesystem roots are not mutable",
            ));
        }
        let kind = match identity.kind {
            FileType::Directory => EntryKind::Directory,
            FileType::RegularFile => EntryKind::File,
            FileType::Symlink => EntryKind::Symlink,
            _ => EntryKind::Other,
        };
        let metadata = EntryMetadata {
            size: (kind == EntryKind::File).then_some(stat.st_size as u64),
            modified: modified_time(stat.st_mtime, stat.st_mtime_nsec),
            localized_modified: modified_time(stat.st_mtime, stat.st_mtime_nsec)
                .and_then(crate::locale::modified),
            unix_mode: Some(stat.st_mode),
            ..EntryMetadata::default()
        };
        check_cancel(cancel, &path)?;
        Ok(Self {
            item: MutationItem {
                path,
                kind,
                metadata,
            },
            parent_path,
            name,
            parent: Arc::new(parent),
            parent_identity,
            identity,
            _source: source,
            trash_root,
        })
    }

    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        check_cancel(cancel, &self.item.path)?;
        let current = File::from(
            openat(CWD, &self.parent_path, DIRECTORY_FLAGS, Mode::empty())
                .map_err(|e| native_error(&self.parent_path, e))?,
        );
        if Identity::of(&current, &self.parent_path)? != self.parent_identity {
            return Err(error(
                &self.item.path,
                ErrorKind::InvalidInput,
                "the parent directory changed",
            ));
        }
        self.identity
            .verify(&self.parent, &self.name, &self.item.path)
    }
}

impl PreparedMutation for Prepared {
    fn item(&self) -> &MutationItem {
        &self.item
    }

    fn rename(&self, new_name: &OsStr, cancel: &Cancellation) -> Result<PathBuf, FsError> {
        validate_name(new_name, &self.item.path)?;
        self.revalidate(cancel)?;
        if new_name == self.name {
            return Ok(self.item.path.clone());
        }
        let staged = Staged::create(self)?;
        staged.capture(self, cancel)?;
        let destination = self.parent_path.join(new_name);
        let result = check_cancel(cancel, &self.item.path).and_then(|()| {
            self.identity
                .verify(&staged.directory, OsStr::new(STAGED_NAME), &self.item.path)?;
            renameat_with(
                &*staged.directory,
                STAGED_NAME,
                &*self.parent,
                new_name,
                RenameFlags::NOREPLACE,
            )
            .map_err(|e| native_error(&destination, e))
        });
        match result {
            Ok(()) => {
                staged.cleanup()?;
                Ok(destination)
            }
            Err(failure) => Err(staged.restore(self, failure)),
        }
    }

    fn delete(
        &self,
        mode: DeleteMode,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(&Path),
    ) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        if mode == DeleteMode::Trash {
            return trash::move_entry(self, cancel, progress);
        }
        let staged = Staged::create(self)?;
        staged.capture(self, cancel)?;
        match tree::delete(
            &staged.directory,
            self.identity,
            &self.item.path,
            cancel,
            progress,
        ) {
            Ok(()) => staged.cleanup(),
            Err(failure) => Err(staged.restore(self, failure)),
        }
    }
}

struct Staged {
    parent: Arc<File>,
    directory: Arc<File>,
    name: OsString,
    path: PathBuf,
    identity: Identity,
}

impl Staged {
    fn create(prepared: &Prepared) -> Result<Self, FsError> {
        loop {
            let name = OsString::from(unique_name(".midnight-mutation"));
            let path = prepared.parent_path.join(&name);
            match mkdirat(&*prepared.parent, &name, Mode::RWXU) {
                Ok(()) => {}
                Err(rustix::io::Errno::EXIST) => continue,
                Err(e) => return Err(native_error(&path, e)),
            }
            let directory = File::from(
                openat(
                    &*prepared.parent,
                    &name,
                    DIRECTORY_FLAGS | OFlags::NOFOLLOW,
                    Mode::empty(),
                )
                .map_err(|e| native_error(&path, e))?,
            );
            verify_private_directory(&directory, &path)?;
            let identity = Identity::of(&directory, &path)?;
            return Ok(Self {
                parent: prepared.parent.clone(),
                directory: Arc::new(directory),
                name,
                path,
                identity,
            });
        }
    }

    fn capture(&self, prepared: &Prepared, cancel: &Cancellation) -> Result<(), FsError> {
        if let Err(failure) = prepared.revalidate(cancel) {
            let _ = self.cleanup();
            return Err(failure);
        }
        if let Err(e) = renameat_with(
            &*prepared.parent,
            &prepared.name,
            &*self.directory,
            STAGED_NAME,
            RenameFlags::NOREPLACE,
        ) {
            let _ = self.cleanup();
            return Err(native_error(&prepared.item.path, e));
        }
        if let Err(failure) = prepared.identity.verify(
            &self.directory,
            OsStr::new(STAGED_NAME),
            &prepared.item.path,
        ) {
            return Err(self.restore(prepared, failure));
        }
        Ok(())
    }

    fn restore(&self, prepared: &Prepared, mut failure: FsError) -> FsError {
        match renameat_with(
            &*self.directory,
            STAGED_NAME,
            &*prepared.parent,
            &prepared.name,
            RenameFlags::NOREPLACE,
        ) {
            Ok(()) => {
                if let Err(cleanup) = self.cleanup() {
                    failure.message.push_str(&format!("; {cleanup}"));
                }
            }
            Err(restore) => {
                failure.message.push_str(&format!(
                    "; remaining entry preserved at {} (restore failed: {restore})",
                    self.path.join(STAGED_NAME).display()
                ));
            }
        }
        failure
    }

    fn cleanup(&self) -> Result<(), FsError> {
        self.identity.verify(&self.parent, &self.name, &self.path)?;
        unlinkat(&*self.parent, &self.name, AtFlags::REMOVEDIR)
            .map_err(|e| native_error(&self.path, e))
    }
}

fn validate_name(name: &OsStr, path: &Path) -> Result<(), FsError> {
    let bytes = name.as_bytes();
    if bytes.is_empty()
        || bytes == b"."
        || bytes == b".."
        || bytes.contains(&0)
        || bytes.contains(&b'/')
    {
        Err(error(
            path,
            ErrorKind::InvalidInput,
            "enter a single valid Unix basename",
        ))
    } else {
        Ok(())
    }
}

fn verify_private_directory(directory: &File, path: &Path) -> Result<(), FsError> {
    let stat = fstat(directory).map_err(|e| native_error(path, e))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
        || stat.st_uid != rustix::process::geteuid().as_raw()
        || stat.st_mode & 0o777 != 0o700
    {
        return Err(error(
            path,
            ErrorKind::PermissionDenied,
            "mutation storage must be an owned directory with permissions 0700",
        ));
    }
    Ok(())
}

fn unique_name(prefix: &str) -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{prefix}-{}-{nanos:x}-{:x}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn modified_time(seconds: i64, nanos: u64) -> Option<SystemTime> {
    let time = if seconds >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(seconds as u64))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs(seconds.unsigned_abs()))
    }?;
    time.checked_add(Duration::from_nanos(nanos))
}

fn check_cancel(cancel: &Cancellation, path: &Path) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(error(
            path,
            ErrorKind::Interrupted,
            "operation cancelled; completed deletions remain",
        ))
    } else {
        Ok(())
    }
}

fn native_error(path: &Path, error: rustix::io::Errno) -> FsError {
    FsError::from_io(OPERATION, path, error.into())
}

fn error(path: &Path, kind: ErrorKind, message: &str) -> FsError {
    FsError::from_io(OPERATION, path, io::Error::new(kind, message))
}

#[cfg(test)]
mod create_tests;
#[cfg(test)]
mod tests;
