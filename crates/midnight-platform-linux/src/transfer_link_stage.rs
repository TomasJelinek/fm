//! Owned link stages. Linux cannot create an anonymous symbolic link, so a link is
//! created as `item` in a private, owner-only `.midnight-link-<random>` directory
//! beneath the destination, read back without following it, and published with a
//! no-replace rename. A consented replacement moves it into a private
//! `.midnight-recovery-<random>` directory and exchanges it with the target there,
//! so the displaced entry stays at `recovery/item`, as for files. The target text
//! is never rebased, resolved or opened.

use super::*;
use midnight_core::{
    StagedTransferLink, TransferLinkCleanupError, TransferLinkFormat, TransferLinkKind,
    TransferLinkRepresentation, TransferLinkStageError, TransferRecovery,
};
use recovery::{Revision, StageRemovalError};
use rustix::fs::{RenameFlags, readlinkat, renameat_with, symlinkat, unlinkat};

/// The staged link's name inside its private directory, and a displaced entry's
/// name inside a recovery directory.
const ITEM: &str = "item";

fn item() -> &'static OsStr {
    OsStr::new(ITEM)
}

impl Directory {
    pub(super) fn admit_link_representation(
        &self,
        representation: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        checked(cancel, &self.path)?;
        if representation.kind() != TransferLinkKind::LinuxSymbolicLink
            || representation.format() != TransferLinkFormat::LinuxSymlinkBytes
            || representation.linux_target().is_none()
        {
            return Err(unsupported(
                &self.path,
                "Linux recreates only Linux symbolic links",
            ));
        }
        // Refused before anything is created: the destination says why.
        if let Some(filesystem) = super::super::link_destination::cannot_store_links(&self.file) {
            return Err(unsupported(
                &self.path,
                &format!(
                    "This drive cannot store links ({filesystem}), so no link is created here"
                ),
            ));
        }
        self.revalidate(cancel)
    }

    pub(super) fn create_link_stage(
        &self,
        representation: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferLink>, TransferLinkStageError> {
        let Some(target) = representation.linux_target() else {
            return Err(TransferLinkStageError {
                error: unsupported(&self.path, "Linux recreates only Linux symbolic links"),
                stage: None,
            });
        };
        let created = namespace::change(cancel, |mut change| {
            Ok(self.create_link_stage_in(target, cancel, &mut change))
        });
        match created {
            Err(error) => Err(TransferLinkStageError { error, stage: None }),
            Ok(Ok(stage)) => Ok(Box::new(stage)),
            Ok(Err((error, stage))) => Err(TransferLinkStageError {
                error,
                stage: stage.map(|stage| stage as Box<dyn StagedTransferLink>),
            }),
        }
    }

    /// On failure, the stage is returned when something remains to clean up.
    fn create_link_stage_in(
        &self,
        target: &[u8],
        cancel: &Cancellation,
        change: &mut namespace::Change<'_>,
    ) -> Result<LinkStage, (FsError, Option<Box<LinkStage>>)> {
        self.revalidate(cancel).map_err(|error| (error, None))?;
        let stage = recovery::create_named_directory(self, cancel, ".midnight-link-", change)
            .map_err(|error| (error, None))?;
        let path = stage.path.join(ITEM);
        let created = stage
            .namespace_reserve_file(item(), change, cancel)
            .and_then(|()| stage.namespace_announce(item(), change, cancel))
            .and_then(|()| checked(cancel, &path))
            .and_then(|()| {
                symlinkat(target, &*stage.file, ITEM).map_err(|failure| match failure {
                    rustix::io::Errno::PERM | rustix::io::Errno::OPNOTSUPP => unsupported(
                        &path,
                        "This filesystem cannot store symbolic links, so the link cannot be \
                         recreated here",
                    ),
                    failure => error(&path, failure.into()),
                })
            });
        if let Err(failure) = created {
            // Nothing was created in the stage this turn made; remove it in this turn.
            let mut owned = LinkStage::new(self.clone(), stage);
            return Err(match owned.remove_stage(change) {
                Ok(()) => (failure, None),
                Err(cleanup) => (with_cleanup(failure, &cleanup), Some(Box::new(owned))),
            });
        }
        stage.namespace_changed(item());
        let mut owned = LinkStage::new(self.clone(), stage);
        match owned.staged_identity() {
            Ok(identity) => {
                owned.link = Some(identity);
                Ok(owned)
            }
            // A link exists that cannot be identified: cleanup authority only.
            Err(failure) => Err((failure, Some(Box::new(owned)))),
        }
    }
}

fn with_cleanup(mut failure: FsError, cleanup: &FsError) -> FsError {
    failure.message.push_str(&format!(
        "; the private link stage could not be removed: {}",
        cleanup.message
    ));
    failure
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Location {
    /// `stage/item`, unpublished.
    Staged,
    /// Moved into a recovery directory for an exchange that did not complete.
    Recovery,
    /// At its final name.
    Published,
}

struct LinkReplacement {
    name: OsString,
    expected: Arc<dyn TransferTarget>,
    revision: Revision,
}

struct LinkStage {
    parent: Directory,
    /// The private stage directory, while it exists.
    stage: Option<Directory>,
    /// The created link's identity; None when it could not be read.
    link: Option<TransferIdentity>,
    location: Location,
    published: Option<OsString>,
    replacement: Option<LinkReplacement>,
    cleanup_refused: bool,
    /// Cleanup found the unpublished link gone, or removed it: only the stage remains.
    link_gone: bool,
    /// Cleanup could not verify the stage or its entry as its own, or that the link is
    /// absent (a substituted stage, an unknown entry, an unreadable slot).
    cleanup_uncertain: bool,
}

impl LinkStage {
    fn new(parent: Directory, stage: Directory) -> Self {
        Self {
            parent,
            stage: Some(stage),
            link: None,
            location: Location::Staged,
            published: None,
            replacement: None,
            cleanup_refused: false,
            link_gone: false,
            cleanup_uncertain: false,
        }
    }

    fn stage_directory(&self) -> Result<&Directory, FsError> {
        self.stage
            .as_ref()
            .filter(|_| self.location == Location::Staged)
            .ok_or_else(|| invalid(&self.parent.path, "the link stage is no longer available"))
    }

    fn staged_identity(&self) -> Result<TransferIdentity, FsError> {
        let stage = self.stage_directory()?;
        let path = stage.path.join(ITEM);
        let metadata = open_link(stage, item())?
            .metadata()
            .map_err(|failure| error(&path, failure))?;
        if !metadata.file_type().is_symlink() {
            return Err(invalid(&path, "the staged entry is not a link"));
        }
        Ok(identity_of(&metadata))
    }

    /// Check that `name` in `directory` is still our link with exactly `expected`.
    fn verify_link(
        &self,
        directory: &Directory,
        name: &OsStr,
        expected: Option<&TransferLinkRepresentation>,
    ) -> Result<TransferIdentity, FsError> {
        let path = directory.path.join(name);
        let file = open_link(directory, name)?;
        let metadata = file.metadata().map_err(|failure| error(&path, failure))?;
        let identity = identity_of(&metadata);
        if !metadata.file_type().is_symlink() || Some(identity) != self.link {
            return Err(invalid(&path, "the staged link changed"));
        }
        if let Some(expected) = expected
            && read_representation(&file, &path)? != *expected
        {
            return Err(invalid(&path, "the staged link text changed"));
        }
        Ok(identity)
    }

    /// The stage is still the protected parent's child under its private name.
    fn check_stage_binding(&self, stage: &Directory, cancel: &Cancellation) -> Result<(), FsError> {
        self.parent.revalidate(cancel)?;
        let name = stage
            .path
            .file_name()
            .ok_or_else(|| invalid(&stage.path, "the link stage needs a basename"))?;
        if stage.path.parent() != Some(self.parent.path.as_path())
            || self
                .parent
                .inspect(name, cancel)?
                .map(|entry| entry.identity)
                != Some(stage.identity)
        {
            return Err(invalid(&stage.path, "the private link stage changed"));
        }
        stage.revalidate(cancel)
    }

    /// Remove the empty private stage in `change`. A stage that remains is kept for
    /// a later cleanup and reported; a removal whose parent sync failed is done.
    fn remove_stage(&mut self, change: &mut namespace::Change<'_>) -> Result<(), FsError> {
        let Some(stage) = self.stage.take() else {
            return Ok(());
        };
        match self.parent.remove_empty_stage_checked(stage, change) {
            Ok(()) | Err(StageRemovalError::Unsynced(_)) => Ok(()),
            Err(StageRemovalError::Retained(failure, stage)) => {
                self.stage = Some(*stage);
                Err(failure)
            }
        }
    }

    fn publish_in(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
        change: &mut namespace::Change<'_>,
    ) -> Result<TransferPublication, FsError> {
        self.parent.revalidate(cancel)?;
        validate_name(name, &self.parent.path)?;
        let stage = self.stage_directory()?.clone();
        let path = self.parent.path.join(name);
        self.parent.namespace_reserve_file(name, change, cancel)?;
        self.parent.namespace_announce(name, change, cancel)?;
        stage.namespace_announce(item(), change, cancel)?;
        checked(cancel, &path)?;
        renameat_with(
            &*stage.file,
            ITEM,
            &*self.parent.file,
            name,
            RenameFlags::NOREPLACE,
        )
        .map_err(|failure| rename_error(&path, failure))?;
        self.location = Location::Published;
        self.published = Some(name.to_owned());
        self.parent.namespace_changed(name);
        stage.namespace_changed(item());
        // The stage's removal certifies its close only if it holds the stage's last handle.
        drop(stage);
        let synced = self.parent.file.sync_all();
        // The empty stage goes in this turn; one that remains is left to cleanup.
        let _ = self.remove_stage(change);
        Ok(match synced {
            Ok(()) => TransferPublication::Published(path),
            Err(failure) => TransferPublication::PublishedDurabilityUnconfirmed {
                error: error(&path, failure),
                path,
            },
        })
    }

    fn replace_in(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
        change: &mut namespace::Change<'_>,
    ) -> Result<TransferPublication, FsError> {
        let (expected, expected_revision) = match &self.replacement {
            Some(replacement) if replacement.name == name => {
                (replacement.expected.clone(), replacement.revision)
            }
            Some(_) => {
                return Err(invalid(
                    &self.parent.path,
                    "replacement basename differs from consent",
                ));
            }
            None => {
                return Err(invalid(
                    &self.parent.path,
                    "the link stage has no replacement consent",
                ));
            }
        };
        if !recovery::supported(&self.parent) {
            return Err(unsupported(
                &self.parent.path,
                "keeping a replaced item recoverable requires a local ext filesystem",
            ));
        }
        let stage = self.stage_directory()?.clone();
        self.parent.revalidate(cancel)?;
        expected.revalidate(cancel)?;
        let expected_identity = expected.entry().identity;
        let path = self.parent.path.join(name);
        // The name holds the consented file or link until the exchange.
        self.parent
            .namespace_reserve_entry(name, expected.entry().item.kind, change, cancel)?;
        self.parent.namespace_announce(name, change, cancel)?;
        let recovery = recovery::create_recovery(&self.parent, cancel, change)?;
        let retained = recovery.path.join(ITEM);
        let moved = recovery
            .namespace_reserve_file(item(), change, cancel)
            .and_then(|()| recovery.namespace_announce(item(), change, cancel))
            .and_then(|()| stage.namespace_announce(item(), change, cancel))
            .and_then(|()| checked(cancel, &retained))
            .and_then(|()| {
                renameat_with(
                    &*stage.file,
                    ITEM,
                    &*recovery.file,
                    ITEM,
                    RenameFlags::NOREPLACE,
                )
                .map_err(|failure| error(&retained, failure.into()))
            });
        if let Err(failure) = moved {
            // Nothing entered the new recovery directory.
            return self.remove_unused_recovery(recovery, failure, change);
        }
        self.location = Location::Recovery;
        stage.namespace_changed(item());
        recovery.namespace_changed(item());
        // The verified link now belongs to persistent recovery until the exchange.
        let link_receipt = TransferRecovery {
            path: retained.clone(),
            identity: self.link,
        };
        let ready = self
            .parent
            .revalidate(cancel)
            .and_then(|()| expected.revalidate(cancel))
            .and_then(|()| checked(cancel, &path))
            .and_then(|()| self.verify_link(&recovery, item(), None));
        if let Err(error) = ready {
            return Ok(TransferPublication::RecoveryRequired {
                published: None,
                recovery: vec![link_receipt],
                error,
            });
        }
        if let Err(failure) = renameat_with(
            &*recovery.file,
            ITEM,
            &*self.parent.file,
            name,
            RenameFlags::EXCHANGE,
        ) {
            return Ok(TransferPublication::RecoveryRequired {
                published: None,
                recovery: vec![link_receipt],
                error: error(&path, failure.into()),
            });
        }
        self.location = Location::Published;
        self.published = Some(name.to_owned());
        self.parent.namespace_changed(name);
        recovery.namespace_changed(item());
        let displaced = open_link(&recovery, item())
            .and_then(|file| Ok((identity(&file, &retained)?, Revision::of(&file, &retained)?)));
        let (displaced_identity, revision) = match displaced {
            Ok(value) => value,
            Err(error) => {
                return Ok(TransferPublication::RecoveryRequired {
                    published: Some(path),
                    recovery: vec![TransferRecovery {
                        path: retained,
                        identity: None,
                    }],
                    error,
                });
            }
        };
        let receipt = TransferRecovery {
            path: retained,
            identity: Some(displaced_identity),
        };
        if displaced_identity != expected_identity
            || !expected_revision.after_rename_matches(revision)
        {
            return Ok(TransferPublication::RecoveryRequired {
                published: Some(path.clone()),
                recovery: vec![receipt],
                error: invalid(
                    &path,
                    "a concurrent destination change was retained in recovery",
                ),
            });
        }
        if let Err(failure) = recovery
            .file
            .sync_all()
            .and_then(|()| self.parent.file.sync_all())
        {
            return Ok(TransferPublication::RecoveryRequired {
                published: Some(path.clone()),
                recovery: vec![receipt],
                error: error(&path, failure),
            });
        }
        // The stage's removal certifies its close only if it holds the stage's last handle.
        drop(stage);
        let _ = self.remove_stage(change);
        Ok(TransferPublication::Replaced {
            path,
            recovery: receipt,
        })
    }

    /// A replacement failed before anything entered `recovery`: remove the empty
    /// directory, or report it instead of leaking it silently.
    fn remove_unused_recovery(
        &self,
        recovery: Directory,
        mut failure: FsError,
        change: &mut namespace::Change<'_>,
    ) -> Result<TransferPublication, FsError> {
        let residual = TransferRecovery {
            path: recovery.path.clone(),
            identity: Some(recovery.identity),
        };
        match self.parent.remove_empty_stage_checked(recovery, change) {
            Ok(()) => Err(failure),
            Err(StageRemovalError::Unsynced(sync)) => {
                failure.message.push_str(&format!(
                    "; the empty recovery directory was removed, but that removal may not \
                     be durable: {sync}"
                ));
                Err(failure)
            }
            Err(StageRemovalError::Retained(cleanup, _)) => {
                failure
                    .message
                    .push_str(&format!("; empty recovery directory cleanup: {cleanup}"));
                Ok(TransferPublication::RecoveryRequired {
                    published: None,
                    recovery: vec![residual],
                    error: failure,
                })
            }
        }
    }

    fn cleanup_in(&mut self, change: &mut namespace::Change<'_>) -> Result<(), FsError> {
        let cancel = Cancellation::default();
        if let Some(stage) = self.stage.clone()
            && self.location == Location::Staged
        {
            if let Err(failure) = self.check_stage_binding(&stage, &cancel) {
                self.cleanup_uncertain = true;
                return Err(failure);
            }
            let path = stage.path.join(ITEM);
            match open_link(&stage, item()) {
                // The unpublished link was never created or is already gone.
                Err(failure) if failure.kind == io::ErrorKind::NotFound => self.link_gone = true,
                Err(failure) => {
                    self.cleanup_uncertain = true;
                    return Err(failure);
                }
                Ok(file) => {
                    let metadata = file.metadata().map_err(|failure| {
                        self.cleanup_uncertain = true;
                        error(&path, failure)
                    })?;
                    // Remove only our own link: never an unknown or substituted entry.
                    if self.link.is_none()
                        || !metadata.file_type().is_symlink()
                        || Some(identity_of(&metadata)) != self.link
                    {
                        self.cleanup_uncertain = true;
                        return Err(invalid(
                            &path,
                            "the private link stage holds an unknown entry, which was kept",
                        ));
                    }
                    stage.namespace_announce(item(), change, &cancel)?;
                    unlinkat(&*stage.file, ITEM, rustix::fs::AtFlags::empty())
                        .map_err(|failure| error(&path, failure.into()))?;
                    self.link_gone = true;
                    stage.namespace_changed(item());
                }
            }
        }
        self.remove_stage(change)
    }

    /// What remains of the stage after a cleanup that did not complete: its own link
    /// where it still is, otherwise the stage folder itself (whatever it holds, which is
    /// never named by an identity cleanup did not verify).
    fn residuals(&self) -> Vec<TransferRecovery> {
        let link = self.location == Location::Staged && !self.link_gone && !self.cleanup_uncertain;
        self.stage
            .iter()
            .map(|stage| TransferRecovery {
                path: if link {
                    stage.path.join(ITEM)
                } else {
                    stage.path.clone()
                },
                identity: if link {
                    self.link
                } else {
                    Some(stage.identity)
                },
            })
            .collect()
    }
}

impl StagedTransferLink for LinkStage {
    fn link_representation(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferLinkRepresentation, FsError> {
        let stage = self.stage_directory()?;
        checked(cancel, &stage.path)?;
        let path = stage.path.join(ITEM);
        let file = open_link(stage, item())?;
        let metadata = file.metadata().map_err(|failure| error(&path, failure))?;
        if !metadata.file_type().is_symlink() || Some(identity_of(&metadata)) != self.link {
            return Err(invalid(&path, "the staged link changed"));
        }
        read_representation(&file, &path)
    }

    fn sync(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        let stage = self.stage_directory()?;
        checked(cancel, &stage.path)?;
        stage
            .file
            .sync_all()
            .map_err(|failure| error(&stage.path, failure))
    }

    fn revalidate_stage(
        &self,
        expected: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<TransferIdentity, FsError> {
        let stage = self.stage_directory()?;
        self.check_stage_binding(stage, cancel)?;
        let identity = self.verify_link(stage, item(), Some(expected))?;
        checked(cancel, &stage.path)?;
        Ok(identity)
    }

    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        if self.published.is_some() {
            return Err(invalid(
                &self.parent.path,
                "the link was already copied to its destination",
            ));
        }
        namespace::change(cancel, |mut change| {
            self.publish_in(name, cancel, &mut change)
        })
    }

    fn revalidate_publication(
        &self,
        expected: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<TransferIdentity, FsError> {
        let name = self.published.as_ref().ok_or_else(|| {
            invalid(
                &self.parent.path,
                "the link has not been copied to its destination",
            )
        })?;
        self.parent.revalidate(cancel)?;
        self.verify_link(&self.parent, name, Some(expected))
    }

    fn prepare_replacement(
        &mut self,
        name: &OsStr,
        expected: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        if policy != TransferReplacementPolicy::RecoverableCurrentEntry
            || !recovery::supported(&self.parent)
        {
            return Err(unsupported(
                &self.parent.path,
                "replacing an entry with a link needs recoverable replacement on a local ext \
                 filesystem",
            ));
        }
        self.stage_directory()?;
        validate_name(name, &self.parent.path)?;
        let path = self.parent.path.join(name);
        if expected.entry().item.path != path
            || !matches!(
                expected.entry().item.kind,
                EntryKind::File | EntryKind::Symlink
            )
        {
            return Err(unsupported(
                &path,
                "a link replaces only a consented file or link",
            ));
        }
        self.parent.revalidate(cancel)?;
        expected.revalidate(cancel)?;
        let visible = open_link(&self.parent, name)?;
        if identity(&visible, &path)? != expected.entry().identity {
            return Err(invalid(&path, "replacement destination identity changed"));
        }
        let revision = Revision::of(&visible, &path)?;
        expected.revalidate(cancel)?;
        self.replacement = Some(LinkReplacement {
            name: name.to_owned(),
            expected,
            revision,
        });
        Ok(())
    }

    fn publish_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        if self.published.is_some() {
            return Err(invalid(
                &self.parent.path,
                "the link was already copied to its destination",
            ));
        }
        namespace::change(cancel, |mut change| {
            self.replace_in(name, cancel, &mut change)
        })
    }

    fn cleanup(&mut self) -> Result<(), TransferLinkCleanupError> {
        if self.cleanup_refused {
            return Err(TransferLinkCleanupError {
                error: invalid(
                    &self.parent.path,
                    "the link stage cleanup was already refused",
                ),
                recovery: self.residuals(),
                cleanup_unconfirmed: true,
            });
        }
        let result = namespace::change(&Cancellation::default(), |mut change| {
            Ok(self.cleanup_in(&mut change))
        })
        .and_then(|result| result);
        result.map_err(|error| {
            self.cleanup_refused = true;
            TransferLinkCleanupError {
                error,
                recovery: self.residuals(),
                cleanup_unconfirmed: self.link.is_none() || self.cleanup_uncertain,
            }
        })
    }
}

/// Open `name` in `directory` as the entry itself, never following it.
fn open_link(directory: &Directory, name: &OsStr) -> Result<File, FsError> {
    let path = directory.path.join(name);
    openat2(
        &*directory.file,
        name,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
        RESOLVE,
    )
    .map(File::from)
    .map_err(|failure| error(&path, failure.into()))
}

fn read_representation(file: &File, path: &Path) -> Result<TransferLinkRepresentation, FsError> {
    let target = readlinkat(file, "", Vec::new()).map_err(|failure| error(path, failure.into()))?;
    TransferLinkRepresentation::linux_symbolic_link(target.as_bytes()).map_err(|mut failure| {
        failure.path = path.to_owned();
        failure
    })
}

/// A rename that cannot exclude replacement is refused, never retried as one that can.
fn rename_error(path: &Path, failure: rustix::io::Errno) -> FsError {
    if failure == rustix::io::Errno::INVAL {
        unsupported(
            path,
            "This filesystem cannot create the link at its destination without replacing an existing name",
        )
    } else {
        error(path, failure.into())
    }
}
