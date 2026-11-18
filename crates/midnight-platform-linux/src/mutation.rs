//! Native mutations retain inode handles and parent directory anchors.
//!
//! Linux has no conditional unlink/rename by inode. We first move the selected
//! name, without replacement, into a newly created owner-only directory and
//! verify its identity there. Filesystems that cannot store POSIX modes (vfat,
//! exfat, NTFS, CIFS without Unix extensions, FUSE) keep their mount-provided
//! permissions on that owned directory, accepted only while no other user can
//! write to it or while it is exactly as open as its unprotected parent (see
//! `crate::ownership`), so staging never lets other users change more than the
//! parent does. A raced replacement, like the item a failed capture request still
//! moved (a lost reply on network or FUSE storage), is restored without overwrite;
//! if restoration collides it remains in staging and the error gives its path.
//! This prevents deleting a replacement raced into the original parent. It is
//! not an isolation boundary against root or another process of the same uid,
//! nor against writers already holding directory handles inside the selected
//! tree. Such writers can still race individual directory entries. We recheck
//! identities before unlink, never follow links, and reject mount crossings.
//! Cancellation and errors restore the remaining tree where possible; already
//! unlinked descendants cannot be restored. A crash may leave staging behind.
//! A completed change whose now empty staging directory cannot be removed (for
//! example because another program created an entry in it) is still reported as
//! completed, with that directory as a typed residue; its location is claimed
//! only while it still has the identity it was created with. A failed change
//! that also leaves its staging directory behind names it in an error of kind
//! Other, never as a plain conflict or cancellation. After a capture request that
//! returned an error, the selected item found at the staged name is restored to
//! its name; a staging directory whose staged name holds another entry is
//! reported as holding an entry that is not the selected item, and one whose
//! staged name cannot be inspected as possibly holding the item; neither is
//! described as unneeded. Staging contents are never removed recursively.
//! renameat2(RENAME_NOREPLACE) support is required. Recursive operations use
//! openat2 (Linux 5.6+) and fail explicitly if these kernel guarantees are absent.
//! Preparation retains two file descriptors per selected item; exhaustion fails
//! preparation. Traversal additionally uses descriptors proportional to depth.

use midnight_core::{
    Cancellation, DeleteCompletion, DeleteMode, EntryKind, EntryMetadata, FsError, MutationItem,
    MutationResidue, MutationService, PreparedDirectory, PreparedMutation, RenameCompletion,
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
pub(crate) mod discovery;
mod metadata;
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

/// Whether `entry`, opened below `parent` without crossing a mount
/// (RESOLVE_NO_XDEV), is still the root of a filesystem: the parent itself, or a
/// directory on another device (a Btrfs or bcachefs subvolume). Overlayfs without
/// xino reports a per-layer device for every non-directory, so a non-directory's
/// device alone never makes it a root.
fn filesystem_root(entry: Identity, parent: Identity) -> bool {
    entry == parent || (entry.kind == FileType::Directory && entry.device != parent.device)
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
        if filesystem_root(identity, parent_identity) {
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
    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        discovery::walk(&self._source, &self.item.path, cancel, visit)?;
        self.revalidate(cancel)
    }
    fn prepare_rename_replacement(
        &self,
        new_name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<midnight_core::PreparedRenameReplacement, FsError> {
        use midnight_core::{TransferIdentity, TransferService};
        self.revalidate(cancel)?;
        validate_name(new_name, &self.item.path)?;
        if self.item.kind != EntryKind::File {
            return Err(error(
                &self.item.path,
                ErrorKind::Unsupported,
                "Rename replacement requires two ordinary files",
            ));
        }
        let identity = |value: Identity| {
            let mut object = [0; 16];
            object[..8].copy_from_slice(&value.inode.to_le_bytes());
            TransferIdentity {
                device: value.device,
                object,
            }
        };
        let service = crate::LinuxFileSystem::new(None);
        let directory = service.prepare_move_destination(&self.parent_path, cancel)?;
        if directory.identity() != identity(self.parent_identity) {
            return Err(error(
                &self.parent_path,
                ErrorKind::InvalidInput,
                "the parent directory changed",
            ));
        }
        let movement = service.prepare_move_source(&self.item.path, cancel)?;
        if movement.capture.entry().identity != identity(self.identity) {
            return Err(error(
                &self.item.path,
                ErrorKind::InvalidInput,
                "the selected entry changed",
            ));
        }
        let target = directory.prepare_target(new_name, cancel)?.ok_or_else(|| {
            error(
                &self.item.path,
                ErrorKind::NotFound,
                "Rename destination disappeared",
            )
        })?;
        if target.entry().item.kind != EntryKind::File
            || target.entry().identity == movement.capture.entry().identity
        {
            return Err(error(
                &target.entry().item.path,
                ErrorKind::InvalidInput,
                "Cannot replace a directory, link, source itself or hard-link alias",
            ));
        }
        if !directory.supports_native_move_replacement(movement.capture.entry().identity) {
            return Err(error(
                &target.entry().item.path,
                ErrorKind::Unsupported,
                "Recoverable Rename replacement requires a qualified local filesystem and protected parent.",
            ));
        }
        self.revalidate(cancel)?;
        Ok(midnight_core::PreparedRenameReplacement {
            movement,
            directory,
            target,
        })
    }

    fn item(&self) -> &MutationItem {
        &self.item
    }
    fn metadata_snapshot(
        &self,
        cancel: &Cancellation,
    ) -> Result<midnight_core::MetadataSnapshot, FsError> {
        metadata::snapshot(self, cancel)
    }
    fn metadata_children(
        &self,
        cancel: &Cancellation,
        max_names: usize,
        max_bytes: usize,
    ) -> Result<Box<dyn midnight_core::MetadataChildren>, FsError> {
        metadata::children(self, cancel, max_names, max_bytes)
    }
    fn metadata_name_available(&self, name: &OsStr, cancel: &Cancellation) -> Result<(), FsError> {
        metadata::name_available(self, name, cancel)
    }
    fn edit_metadata(
        &self,
        patch: &midnight_core::MetadataPatch,
        name: Option<&OsStr>,
        cancel: &Cancellation,
    ) -> midnight_core::MetadataOutcome {
        metadata::edit(self, patch, name, cancel)
    }

    fn rename(&self, new_name: &OsStr, cancel: &Cancellation) -> Result<RenameCompletion, FsError> {
        validate_name(new_name, &self.item.path)?;
        self.revalidate(cancel)?;
        if new_name == self.name {
            return Ok(RenameCompletion {
                destination: self.item.path.clone(),
                residue: None,
            });
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
            // Published: the rename completed even if its staging stays behind.
            Ok(()) => Ok(RenameCompletion {
                destination,
                residue: staged.cleanup().err(),
            }),
            Err(failure) => Err(staged.restore(self, failure)),
        }
    }

    fn delete(
        &self,
        mode: DeleteMode,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(&Path),
    ) -> Result<DeleteCompletion, FsError> {
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
            Ok(()) => Ok(DeleteCompletion {
                residue: staged.cleanup().err(),
            }),
            Err(failure) => Err(staged.restore(self, failure)),
        }
    }
}

/// Rename, permanent Delete and Trash move the item through a private staging
/// directory beside it. The error for a staging directory that `problem`
/// prevents names the item, not the hidden staging directory.
fn staging_unavailable(prepared: &Prepared, problem: &str, cause: io::Error) -> FsError {
    FsError {
        operation: OPERATION,
        path: prepared.item.path.clone(),
        message: format!(
            "a private staging folder is needed beside the item, but it {problem}: {cause}"
        ),
        kind: cause.kind(),
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
            match mkdirat(&*prepared.parent, &name, Mode::RWXU) {
                Ok(()) => {}
                Err(rustix::io::Errno::EXIST) => continue,
                Err(e) => {
                    return Err(staging_unavailable(
                        prepared,
                        "could not be created",
                        e.into(),
                    ));
                }
            }
            let path = prepared.parent_path.join(&name);
            return Self::adopt(prepared, name, path);
        }
    }

    /// Pins the staging directory just created. Where the filesystem cannot store
    /// 0700, mount-provided permissions are accepted only as
    /// `crate::ownership::check_created_directory` allows (never wider than the
    /// parent). A refusal names the item; the directory is removed rather than left
    /// behind. One that cannot be removed is named in an error of kind Other, and a
    /// name another entry has taken over is reported as such and left untouched.
    fn adopt(prepared: &Prepared, name: OsString, path: PathBuf) -> Result<Self, FsError> {
        let mut pinned = None;
        let adopted = (|| -> io::Result<(File, Identity)> {
            let directory = File::from(openat(
                &*prepared.parent,
                &name,
                DIRECTORY_FLAGS | OFlags::NOFOLLOW,
                Mode::empty(),
            )?);
            let identity = Identity::from_stat(&fstat(&directory)?);
            pinned = Some((identity.device, identity.inode));
            crate::ownership::check_created_directory(&*prepared.parent, &directory)?;
            Ok((directory, identity))
        })();
        match adopted {
            Ok((directory, identity)) => Ok(Self {
                parent: prepared.parent.clone(),
                directory: Arc::new(directory),
                name,
                path,
                identity,
            }),
            Err(refusal) => {
                let mut failure = staging_unavailable(prepared, "cannot be used", refusal);
                match crate::ownership::remove_refused_directory(&*prepared.parent, &name, pinned) {
                    Ok(()) => {}
                    // Nothing has that name any more: no location to name.
                    Err(cleanup) if cleanup.kind() == ErrorKind::NotFound => {}
                    Err(cleanup) => {
                        // A location stays behind: never a plain refusal, as in `abandon`.
                        failure.kind = ErrorKind::Other;
                        if cleanup.kind() == ErrorKind::InvalidData {
                            failure.message.push_str(&format!(
                                "; another entry has taken the staging directory's name {} \
                                 and was left untouched",
                                path.display()
                            ));
                        } else {
                            failure.message.push_str(&format!(
                                "; the unused staging directory remains at {}: {cleanup}",
                                path.display()
                            ));
                        }
                    }
                }
                Err(failure)
            }
        }
    }

    fn capture(&self, prepared: &Prepared, cancel: &Cancellation) -> Result<(), FsError> {
        if let Err(failure) = prepared.revalidate(cancel) {
            return Err(self.abandon(prepared, failure, false));
        }
        if let Err(e) = renameat_with(
            &*prepared.parent,
            &prepared.name,
            &*self.directory,
            STAGED_NAME,
            RenameFlags::NOREPLACE,
        ) {
            return Err(self.abandon(prepared, native_error(&prepared.item.path, e), true));
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

    /// Removes the stage of a change that did not complete and returns `failure`. A
    /// capture request that returned an error may still have moved the item there (a
    /// lost reply on a network or FUSE filesystem): after such a request the selected
    /// item found at the staged name (identity-checked against the pinned inode) is
    /// restored to its name, as after any later failure. A stage that stays behind is
    /// named in `failure` and makes it kind Other, so it is never consumed as a plain
    /// conflict or cancellation. After a capture request the message says whether the
    /// staged name holds another entry or could not be inspected (then the item may be
    /// there); a stage is described as unused only when nothing holds the staged name.
    fn abandon(
        &self,
        prepared: &Prepared,
        mut failure: FsError,
        capture_requested: bool,
    ) -> FsError {
        let staged = capture_requested.then(|| {
            prepared.identity.verify(
                &self.directory,
                OsStr::new(STAGED_NAME),
                &prepared.item.path,
            )
        });
        if matches!(staged, Some(Ok(()))) {
            return self.restore(prepared, failure);
        }
        let Err(residue) = self.cleanup() else {
            return failure;
        };
        failure.kind = ErrorKind::Other;
        match staged {
            Some(Err(state)) if state.kind == ErrorKind::InvalidInput => {
                failure.message.push_str(&format!(
                    "; the staging directory remains at {} and holds an entry that is not the selected item: {}",
                    self.path.display(),
                    residue.error.message
                ));
            }
            Some(Err(state)) if state.kind != ErrorKind::NotFound => {
                failure.message.push_str(&format!(
                    "; the item may have been moved into {}: {}",
                    self.path.join(STAGED_NAME).display(),
                    residue.error.message
                ));
            }
            _ => failure.message.push_str(&format!("; {residue}")),
        }
        failure
    }

    fn restore(&self, prepared: &Prepared, mut failure: FsError) -> FsError {
        match renameat_with(
            &*self.directory,
            STAGED_NAME,
            &*prepared.parent,
            &prepared.name,
            RenameFlags::NOREPLACE,
        ) {
            Ok(()) => self.abandon(prepared, failure, false),
            Err(restore) => {
                failure.message.push_str(&format!(
                    "; remaining entry preserved at {} (restore failed: {restore})",
                    self.path.join(STAGED_NAME).display()
                ));
                // The entry stays in staging: never present this as a name conflict
                // (whose resolution would drop the preserved location) or a cancellation.
                failure.kind = ErrorKind::Other;
                failure
            }
        }
    }

    /// Removes the staging directory while it is still the one created here; rmdir
    /// never removes contents. A directory that stays behind is a residue whose
    /// location is confirmed only if it still has its identity after the failure.
    fn cleanup(&self) -> Result<(), MutationResidue> {
        #[cfg(test)]
        simulation::before_cleanup(&self.path);
        let residue = |cleanup_unconfirmed, error| MutationResidue {
            path: self.path.clone(),
            cleanup_unconfirmed,
            error,
        };
        self.verify().map_err(|changed| residue(true, changed))?;
        let failure = match unlinkat(&*self.parent, &self.name, AtFlags::REMOVEDIR) {
            Ok(()) => return Ok(()),
            Err(e) => native_error(&self.path, e),
        };
        #[cfg(test)]
        simulation::after_failed_removal(&self.path);
        Err(match self.verify() {
            Ok(()) => residue(false, failure),
            Err(changed) => residue(true, changed),
        })
    }

    fn verify(&self) -> Result<(), FsError> {
        let current = statat(&*self.parent, &self.name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|e| native_error(&self.path, e))?;
        if Identity::from_stat(&current) != self.identity {
            return Err(error(
                &self.path,
                ErrorKind::Other,
                "a different entry now has its name",
            ));
        }
        Ok(())
    }
}

/// Test-only interference with staging cleanup, on the calling thread only.
#[cfg(test)]
pub(crate) mod simulation {
    use std::{cell::RefCell, path::Path, thread::LocalKey};

    type Interference = Box<dyn FnOnce(&Path)>;
    type Hook = LocalKey<RefCell<Option<Interference>>>;

    thread_local! {
        static BEFORE_CLEANUP: RefCell<Option<Interference>> = const { RefCell::new(None) };
        static AFTER_FAILED_REMOVAL: RefCell<Option<Interference>> = const { RefCell::new(None) };
    }

    /// Calls `interfere` with the staging directory's path just before the first
    /// staging cleanup that `run` performs.
    pub(crate) fn with_interference_before_cleanup<T>(
        interfere: impl FnOnce(&Path) + 'static,
        run: impl FnOnce() -> T,
    ) -> T {
        with(&BEFORE_CLEANUP, Box::new(interfere), run)
    }

    /// Calls `interfere` with the staging directory's path right after the first
    /// failed removal of a staging directory that `run` performs, before that
    /// directory's binding is checked again.
    pub(crate) fn with_interference_after_failed_removal<T>(
        interfere: impl FnOnce(&Path) + 'static,
        run: impl FnOnce() -> T,
    ) -> T {
        with(&AFTER_FAILED_REMOVAL, Box::new(interfere), run)
    }

    fn with<T>(hook: &'static Hook, interfere: Interference, run: impl FnOnce() -> T) -> T {
        hook.with(|hook| *hook.borrow_mut() = Some(interfere));
        let result = run();
        hook.with(|hook| hook.borrow_mut().take());
        result
    }

    pub(super) fn before_cleanup(stage: &Path) {
        interfere(&BEFORE_CLEANUP, stage);
    }

    pub(super) fn after_failed_removal(stage: &Path) {
        interfere(&AFTER_FAILED_REMOVAL, stage);
    }

    fn interfere(hook: &'static Hook, stage: &Path) {
        if let Some(interfere) = hook.with(|hook| hook.borrow_mut().take()) {
            interfere(stage);
        }
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

/// Trash infrastructure must already be exactly owner-only (freedesktop trash).
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
    if !cancel.wait_if_paused() {
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
mod residue_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "mutation_pause_tests.rs"]
mod pause_tests;
