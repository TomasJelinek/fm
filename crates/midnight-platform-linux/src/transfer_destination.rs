use super::*;
use midnight_core::{
    StagedTransferFile, TransferCapabilities, TransferPublication, TransferReplacementPolicy,
    TransferTarget,
};
use rustix::fs::{AtFlags, linkat, mkdirat};
use std::{
    ffi::OsString,
    io::{Read, Seek, SeekFrom, Write},
    os::fd::AsRawFd,
};

#[path = "transfer_preservation.rs"]
mod preservation;
#[path = "transfer_recovery.rs"]
pub(super) mod recovery;

const RECOVERY_PARENT: &str = "Private recovery requires a parent that other users cannot change: owned by you or root, and sticky or writable only by you and your private group. Choose Copy to retain the source; no Move fallback is performed.";
const COPY_PARENT: &str = "Copy creates folders only inside a parent that other users cannot change: owned by you or root, and sticky or writable only by you and your private group.";

/// The use of a directory created by this provider.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum NewDirectory {
    /// A copied folder. Where a filesystem cannot store POSIX modes, its
    /// mount-provided permissions are accepted (ownership is still required).
    Copy,
    /// Private Move staging or replacement recovery: exactly mode 0700.
    Private,
}

#[derive(Clone)]
pub(super) struct Directory {
    file: Arc<File>,
    path: PathBuf,
    identity: TransferIdentity,
    namespace: Arc<namespace::Bindings>,
}
impl Directory {
    pub(in crate::transfer) fn namespace_reserve_file(
        &self,
        name: &OsStr,
        change: &mut namespace::Change<'_>,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        if let Some(transaction) = change.as_mut() {
            let pin = self
                .namespace
                .private_file_pin(&self.file, &self.path, name, cancel)?;
            if !transaction.reserve_private_child(Some(pin), cancel)? {
                return Err(error(
                    &self.path.join(name),
                    io::ErrorKind::WouldBlock.into(),
                ));
            }
        }
        Ok(())
    }

    pub(in crate::transfer) fn namespace_announce(
        &self,
        name: &OsStr,
        change: &mut namespace::Change<'_>,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.namespace
            .announce(&self.file, &self.path, name, change, cancel)
    }
    pub(in crate::transfer) fn namespace_changed(&self, name: &OsStr) {
        self.namespace.changed(&self.file, &self.path, name);
    }
    pub(super) fn check_recovery_parent(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.check_protected_parent(cancel, RECOVERY_PARENT)
    }
    /// Other users must be unable to replace or remove our names in this
    /// directory; see `crate::ownership::protected_parent` for the exact rule.
    fn check_protected_parent(&self, cancel: &Cancellation, refusal: &str) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        let parent = self.file.metadata().map_err(|e| error(&self.path, e))?;
        if !crate::ownership::protected_parent(parent.uid(), parent.gid(), parent.mode()) {
            return Err(unsupported(&self.path, refusal));
        }
        Ok(())
    }
    pub(super) fn from_file(file: File, path: PathBuf) -> Result<Self, FsError> {
        Self::from_file_with(file, path, None)
    }
    /// `namespace` reuses bindings already computed for this same directory.
    pub(super) fn from_file_with(
        file: File,
        path: PathBuf,
        namespace: Option<Arc<namespace::Bindings>>,
    ) -> Result<Self, FsError> {
        if !file.metadata().map_err(|e| error(&path, e))?.is_dir() {
            return Err(invalid(&path, "destination is not a directory"));
        }
        let file = Arc::new(file);
        let namespace = match namespace {
            Some(namespace) => namespace,
            None => namespace::Bindings::for_parent(&file, &path)?,
        };
        Ok(Self {
            identity: identity(&file, &path)?,
            file,
            path,
            namespace,
        })
    }
}
impl TransferDirectory for Directory {
    fn native_child_namespace_scope(
        &self,
        name: &OsStr,
        kind: EntryKind,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn midnight_core::TransferNamespacePin>>, FsError> {
        self.revalidate(cancel)?;
        self.namespace
            .pin(&self.file, &self.path, name, kind, false, cancel)
            .map(|pin| Some(pin as Arc<dyn midnight_core::TransferNamespacePin>))
    }
    fn open_read_child(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.revalidate(cancel)?;
        super::source::Source::open(self.file.clone(), self.path.clone(), name, cancel)
            .map(|source| Box::new(source) as Box<dyn TransferSource>)
    }
    fn path(&self) -> &Path {
        &self.path
    }
    fn identity(&self) -> TransferIdentity {
        self.identity
    }
    fn capabilities(&self) -> TransferCapabilities {
        TransferCapabilities {
            directory_merge: true,
            conditional_overwrite: false,
            recoverable_replacement: recovery::supported(self),
        }
    }
    fn supports_native_move_replacement(&self, source: TransferIdentity) -> bool {
        super::move_source::supports_native_replacement(self, source)
    }
    fn prepare_target(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
        recovery::prepare_target(self, name, cancel)
    }
    fn open_merge_directory(
        &self,
        name: &OsStr,
        expected: &TransferEntry,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        recovery::open_merge(self, name, expected, cancel)
    }
    fn stage_file_for_target(
        &self,
        name: &OsStr,
        expected: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        let mut stage = self.stage_file(cancel)?;
        stage.prepare_replacement(name, expected, policy, cancel)?;
        Ok(stage)
    }
    fn stage_file_preserving_target(
        &self,
        name: &OsStr,
        expected: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        let preserved = preservation::Snapshot::capture(self, name, expected.as_ref(), cancel)?;
        let mut stage = self.create_stage(false, cancel)?;
        let file = stage.file().map_err(|e| error(&self.path, e))?;
        preserved.initialize(file, &self.path)?;
        expected.revalidate(cancel)?;
        stage.preserved = Some(preserved);
        stage.prepare_replacement(name, expected, policy, cancel)?;
        Ok(Box::new(stage))
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.path)?;
        let visible = File::from(
            openat(CWD, &self.path, DIRECTORY, Mode::empty())
                .map_err(|e| error(&self.path, e.into()))?,
        );
        if identity(&visible, &self.path)? != self.identity {
            return Err(invalid(&self.path, "destination directory changed"));
        }
        Ok(())
    }
    fn inspect(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Option<TransferEntry>, FsError> {
        self.revalidate(cancel)?;
        validate_name(name, &self.path)?;
        let path = self.path.join(name);
        match openat2(
            &*self.file,
            name,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
            RESOLVE,
        ) {
            Ok(fd) => entry(&File::from(fd), &path).map(Some),
            Err(rustix::io::Errno::NOENT) => Ok(None),
            Err(e) => Err(error(&path, e.into())),
        }
    }
    fn reject_source_ancestor(
        &self,
        source: TransferIdentity,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        let mut current = self.file.try_clone().map_err(|e| error(&self.path, e))?;
        loop {
            checked(cancel, &self.path)?;
            let id = identity(&current, &self.path)?;
            if id == source {
                return Err(invalid(
                    &self.path,
                    "destination is inside the source directory",
                ));
            }
            let parent = File::from(
                openat(&current, "..", DIRECTORY, Mode::empty())
                    .map_err(|e| error(&self.path, e.into()))?,
            );
            if identity(&parent, &self.path)? == id {
                break;
            }
            current = parent;
        }
        self.revalidate(cancel)
    }
    fn create_directory(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        namespace::change(cancel, |mut change| {
            self.create_directory_in(name, cancel, &mut change)
        })
    }
    fn stage_file(&self, cancel: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.make_stage(false, cancel)
    }
    fn stage_file_with_metadata(
        &self,
        policy: midnight_core::TransferMetadataPolicy,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.make_stage(
            policy == midnight_core::TransferMetadataPolicy::RequireModifiedTime,
            cancel,
        )
    }
}
impl Directory {
    fn make_stage(
        &self,
        metadata: bool,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferFile>, FsError> {
        self.create_stage(metadata, cancel)
            .map(|stage| Box::new(stage) as Box<dyn StagedTransferFile>)
    }
    fn create_stage(&self, metadata: bool, cancel: &Cancellation) -> Result<Stage, FsError> {
        self.revalidate(cancel)?;
        // O_TMPFILE creates an exclusive unnamed inode on the destination filesystem.
        // The file can never be substituted by a staged-path rename. No EXCL here:
        // O_TMPFILE|EXCL deliberately disables the later linkat publication operation.
        let file = File::from(
            openat(
                &*self.file,
                ".",
                OFlags::RDWR | OFlags::TMPFILE | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(|e| match e {
                // EISDIR: a kernel without O_TMPFILE treats it as O_DIRECTORY.
                rustix::io::Errno::OPNOTSUPP | rustix::io::Errno::ISDIR => unsupported(
                    &self.path,
                    "This filesystem cannot create anonymous O_TMPFILE copies; verified Copy on \
                     Linux needs a local filesystem such as ext4, XFS, Btrfs or tmpfs.",
                ),
                e => error(&self.path, e.into()),
            })?,
        );
        Ok(Stage {
            file: Some(file),
            directory: self.clone(),
            published: false,
            published_name: None,
            replacement: None,
            metadata,
            preserved: None,
        })
    }
}

struct Stage {
    metadata: bool,
    preserved: Option<preservation::Snapshot>,
    file: Option<File>,
    directory: Directory,
    published: bool,
    published_name: Option<OsString>,
    replacement: Option<recovery::Replacement>,
}
impl Stage {
    fn file(&self) -> io::Result<&File> {
        self.file
            .as_ref()
            .ok_or_else(|| io::Error::other("stage was cleaned up"))
    }
    fn file_mut(&mut self) -> io::Result<&mut File> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("stage was cleaned up"))
    }
}
impl Read for Stage {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.file_mut()?.read(b)
    }
}
impl Write for Stage {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.file_mut()?.write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file_mut()?.flush()
    }
}
impl Seek for Stage {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        self.file_mut()?.seek(p)
    }
}
impl StagedTransferFile for Stage {
    fn set_modified_time(
        &mut self,
        expected: std::time::SystemTime,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        if self.published || !self.metadata {
            return Err(unsupported(
                &self.directory.path,
                "modified time can only be set on an owned unpublished metadata stage",
            ));
        }
        super::metadata::set(
            self.file().map_err(|e| error(&self.directory.path, e))?,
            &self.directory.path,
            expected,
            cancel,
        )
    }
    fn modified_time(&self, cancel: &Cancellation) -> Result<std::time::SystemTime, FsError> {
        if !self.metadata {
            return Err(unsupported(
                &self.directory.path,
                "required modified time is unsupported by this stage",
            ));
        }
        super::metadata::modified(
            self.file().map_err(|e| error(&self.directory.path, e))?,
            &self.directory.path,
            cancel,
        )
    }
    fn rewind_for_validation(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn midnight_core::TransferReadGuard>, FsError> {
        super::validation::rewind(
            self.file().map_err(|e| error(&self.directory.path, e))?,
            &self.directory.path,
            cancel,
        )
    }
    fn revalidate_publication(&self, cancel: &Cancellation) -> Result<TransferIdentity, FsError> {
        let name = self
            .published_name
            .as_ref()
            .ok_or_else(|| invalid(&self.directory.path, "stage has no successful publication"))?;
        let own = identity(
            self.file().map_err(|e| error(&self.directory.path, e))?,
            &self.directory.path,
        )?;
        let visible = self
            .directory
            .inspect(name, cancel)?
            .ok_or_else(|| invalid(&self.directory.path, "published target disappeared"))?;
        if visible.item.kind != EntryKind::File || visible.identity != own {
            return Err(invalid(
                &self.directory.path,
                "published target name changed",
            ));
        }
        Ok(own)
    }
    fn prepare_replacement(
        &mut self,
        name: &OsStr,
        expected: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.bind_replacement(name, expected, policy, cancel)
    }
    fn publish_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        if let Some(snapshot) = &self.preserved {
            snapshot.verify(
                self.file().map_err(|e| error(&self.directory.path, e))?,
                &self.directory.path,
            )?;
        }
        self.replace(name, cancel, || {})
    }
    fn sync(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.directory.path)?;
        self.file()
            .and_then(File::sync_all)
            .map_err(|e| error(&self.directory.path, e))
    }
    fn len(&self) -> Result<u64, FsError> {
        self.file()
            .and_then(File::metadata)
            .map(|m| m.len())
            .map_err(|e| error(&self.directory.path, e))
    }
    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        namespace::change(cancel, |mut change| {
            self.publish_no_replace_in(name, cancel, &mut change)
        })
    }
    fn cleanup(&mut self) -> Result<(), FsError> {
        self.file.take();
        Ok(())
    }
}

impl Directory {
    pub(super) fn create_directory_in(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
        change: &mut namespace::Change<'_>,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.create_child_directory(name, cancel, change, NewDirectory::Copy)
            .map(|directory| Arc::new(directory) as Arc<dyn TransferDirectory>)
    }

    pub(super) fn create_child_directory(
        &self,
        name: &OsStr,
        cancel: &Cancellation,
        change: &mut namespace::Change<'_>,
        purpose: NewDirectory,
    ) -> Result<Self, FsError> {
        self.revalidate(cancel)?;
        validate_name(name, &self.path)?;
        // mkdirat does not return an inode handle. Until openat2 completes, protect
        // the new name against a different unprivileged UID replacing it. A sticky
        // directory is safe only when its owner is us or trusted root. Same-UID and
        // privileged namespace mutation remain outside this provider's contract.
        self.check_protected_parent(
            cancel,
            match purpose {
                NewDirectory::Copy => COPY_PARENT,
                NewDirectory::Private => RECOVERY_PARENT,
            },
        )?;
        let path = self.path.join(name);
        self.namespace_announce(name, change, cancel)?;
        mkdirat(&*self.file, name, Mode::RWXU).map_err(|e| error(&path, e.into()))?;
        self.namespace_changed(name);
        let mut pinned = None;
        let adopted = (|| {
            let file = File::from(
                openat2(
                    &*self.file,
                    name,
                    DIRECTORY | OFlags::NOFOLLOW,
                    Mode::empty(),
                    RESOLVE,
                )
                .map_err(|e| error(&path, e.into()))?,
            );
            let created = file.metadata().map_err(|e| error(&path, e))?;
            pinned = Some((created.dev(), created.ino()));
            let mode = crate::ownership::check_created_directory(&file).map_err(|failure| {
                unsupported(&path, &format!("The new folder cannot be used: {failure}"))
            })?;
            if purpose == NewDirectory::Private
                && mode != crate::ownership::CreatedDirectory::Private
            {
                return Err(unsupported(
                    &path,
                    "Private staging requires a filesystem that stores owner-only permissions.",
                ));
            }
            Self::from_file(file, path.clone())
        })();
        adopted.map_err(|mut failure| {
            // Nothing was published into the refused directory this call created.
            match crate::ownership::remove_refused_directory(&*self.file, name, pinned) {
                Ok(()) => self.namespace_changed(name),
                Err(cleanup) => failure
                    .message
                    .push_str(&format!("; the empty new folder remains: {cleanup}")),
            }
            failure
        })
    }
}

impl Stage {
    fn publish_no_replace_in(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
        change: &mut namespace::Change<'_>,
    ) -> Result<TransferPublication, FsError> {
        if self.published {
            return Err(invalid(&self.directory.path, "stage was already published"));
        }
        self.directory.revalidate(cancel)?;
        validate_name(name, &self.directory.path)?;
        let path = self.directory.path.join(name);
        let file = self.file().map_err(|e| error(&path, e))?;
        // /proc/self/fd identifies our live descriptor, not an untrusted filesystem
        // symlink. This documented O_TMPFILE publication form works without CAP_DAC_READ_SEARCH.
        let descriptor = format!("/proc/self/fd/{}", file.as_raw_fd());
        checked(cancel, &path)?;
        self.directory
            .namespace_reserve_file(name, change, cancel)?;
        self.directory.namespace_announce(name, change, cancel)?;
        linkat(
            CWD,
            descriptor.as_str(),
            &*self.directory.file,
            name,
            AtFlags::SYMLINK_FOLLOW,
        )
        .map_err(|e| error(&path, e.into()))?;
        self.published = true;
        self.published_name = Some(name.to_owned());
        self.directory.namespace_changed(name);
        match self.directory.file.sync_all() {
            Ok(()) => Ok(TransferPublication::Published(path)),
            Err(failure) => Ok(TransferPublication::PublishedDurabilityUnconfirmed {
                error: error(&path, failure),
                path,
            }),
        }
    }
}
