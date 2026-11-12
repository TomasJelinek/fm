//! Recoverable replacement exchanges names, not expected identities. Displaced
//! objects remain in a private directory; even an unexpected link/tree is never deleted.
use super::*;
use midnight_core::TransferRecovery;
use rustix::fs::{RenameFlags, renameat_with};
use std::ffi::OsString;

impl Directory {
    pub(in crate::transfer) fn source_stage(&self, cancel: &Cancellation) -> Result<Self, FsError> {
        create_named_directory(self, cancel, ".midnight-move-")
    }
    pub(in crate::transfer) fn remove_empty_stage(&self, stage: Self) -> Result<(), FsError> {
        let cleanup = Cancellation::default();
        self.check_recovery_parent(&cleanup)?;
        stage.revalidate(&cleanup)?;
        let name = stage
            .path
            .file_name()
            .ok_or_else(|| invalid(&stage.path, "temporary directory needs a basename"))?;
        if stage.path.parent() != Some(self.path.as_path())
            || self.inspect(name, &cleanup)?.map(|entry| entry.identity) != Some(stage.identity)
        {
            return Err(invalid(&stage.path, "temporary directory identity changed"));
        }
        // The parent is held and protected; the private random name is checked
        // without following links. Linux rmdir refuses nonempty directories.
        // As with creation, same-UID/privileged namespace mutation is not isolated.
        rustix::fs::unlinkat(&*self.file, name, AtFlags::REMOVEDIR)
            .map_err(|e| error(&stage.path, e.into()))?;
        self.file.sync_all().map_err(|e| error(&stage.path, e))
    }
    pub(in crate::transfer) fn native_file(&self) -> &File {
        &self.file
    }
}

pub(in crate::transfer) fn supported(directory: &Directory) -> bool {
    // Qualify local ext filesystems first. Never extend local commit/error claims to
    // NFS/SMB/unknown mounts, where a failed request can have executed on the server.
    rustix::fs::fstatfs(&*directory.file)
        .map(|stat| stat.f_type == 0xef53)
        .unwrap_or(false)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::transfer) struct Revision {
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    mode: u32,
}
impl Revision {
    pub(in crate::transfer) fn of(file: &File, path: &Path) -> Result<Self, FsError> {
        let metadata = file.metadata().map_err(|e| error(path, e))?;
        Ok(Self {
            size: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
            mode: metadata.mode(),
        })
    }
    pub(in crate::transfer) fn after_rename_matches(self, other: Self) -> bool {
        // Our exchange itself updates ctime. It cannot be a post-exchange version token.
        self.size == other.size && self.modified == other.modified && self.mode == other.mode
    }
}

struct Target {
    directory: Directory,
    name: OsString,
    file: File,
    entry: TransferEntry,
    revision: Revision,
}
impl TransferTarget for Target {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.directory.revalidate(cancel)?;
        let visible = open_target(&self.directory, &self.name)?;
        if identity(&visible, &self.entry.item.path)? != self.entry.identity
            || Revision::of(&self.file, &self.entry.item.path)? != self.revision
        {
            return Err(invalid(
                &self.entry.item.path,
                "consented destination changed",
            ));
        }
        Ok(())
    }
}

fn open_target(directory: &Directory, name: &OsStr) -> Result<File, FsError> {
    let path = directory.path.join(name);
    openat2(
        &*directory.file,
        name,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
        RESOLVE,
    )
    .map(File::from)
    .map_err(|e| error(&path, e.into()))
}

pub(super) fn prepare_target(
    directory: &Directory,
    name: &OsStr,
    cancel: &Cancellation,
) -> Result<Option<Arc<dyn TransferTarget>>, FsError> {
    directory.revalidate(cancel)?;
    validate_name(name, &directory.path)?;
    let file = match open_target(directory, name) {
        Ok(file) => file,
        Err(failure) if failure.kind == io::ErrorKind::NotFound => return Ok(None),
        Err(failure) => return Err(failure),
    };
    let path = directory.path.join(name);
    let (entry, revision) = target_snapshot(&file, &path, || {})?;
    Ok(Some(Arc::new(Target {
        entry,
        revision,
        file,
        name: name.to_owned(),
        directory: directory.clone(),
    })))
}

fn target_snapshot(
    file: &File,
    path: &Path,
    after_display_read: impl FnOnce(),
) -> Result<(TransferEntry, Revision), FsError> {
    let before = Revision::of(file, path)?;
    let display = entry(file, path)?;
    after_display_read();
    let revision = Revision::of(file, path)?;
    if before != revision {
        return Err(invalid(
            path,
            "destination changed while capturing conflict metadata",
        ));
    }
    Ok((display, revision))
}

pub(super) fn open_merge(
    directory: &Directory,
    name: &OsStr,
    expected: &TransferEntry,
    cancel: &Cancellation,
) -> Result<Arc<dyn TransferDirectory>, FsError> {
    directory.revalidate(cancel)?;
    validate_name(name, &directory.path)?;
    let path = directory.path.join(name);
    if expected.item.kind != EntryKind::Directory || expected.item.path != path {
        return Err(invalid(
            &path,
            "merging requires the consented ordinary directory",
        ));
    }
    let file = File::from(
        openat2(
            &*directory.file,
            name,
            DIRECTORY | OFlags::NOFOLLOW,
            Mode::empty(),
            RESOLVE,
        )
        .map_err(|e| error(&path, e.into()))?,
    );
    if identity(&file, &path)? != expected.identity {
        return Err(invalid(&path, "merge destination identity changed"));
    }
    Ok(Arc::new(Directory::from_file(file, path)?))
}

pub(super) struct Replacement {
    name: OsString,
    expected: Arc<dyn TransferTarget>,
    revision: Revision,
}

impl Stage {
    pub(super) fn bind_replacement(
        &mut self,
        name: &OsStr,
        expected: Arc<dyn TransferTarget>,
        policy: TransferReplacementPolicy,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        if policy != TransferReplacementPolicy::RecoverableCurrentEntry
            || !supported(&self.directory)
        {
            return Err(unsupported(
                &self.directory.path,
                "identity-conditional replacement is unsupported",
            ));
        }
        if self.published || self.file.is_none() {
            return Err(invalid(
                &self.directory.path,
                "stage is no longer available",
            ));
        }
        validate_name(name, &self.directory.path)?;
        let path = self.directory.path.join(name);
        if expected.entry().item.path != path || expected.entry().item.kind != EntryKind::File {
            return Err(unsupported(
                &path,
                "replacement requires a consented ordinary file",
            ));
        }
        self.directory.revalidate(cancel)?;
        expected.revalidate(cancel)?;
        let visible = open_target(&self.directory, name)?;
        if identity(&visible, &path)? != expected.entry().identity {
            return Err(invalid(&path, "replacement destination identity changed"));
        }
        let revision = Revision::of(&visible, &path)?;
        expected.revalidate(cancel)?;
        self.replacement = Some(Replacement {
            name: name.to_owned(),
            expected,
            revision,
        });
        Ok(())
    }

    pub(super) fn replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
        before_exchange: impl FnOnce(),
    ) -> Result<TransferPublication, FsError> {
        if self.published || self.file.is_none() {
            return Err(invalid(
                &self.directory.path,
                "stage is no longer available",
            ));
        }
        let replacement = self
            .replacement
            .as_ref()
            .ok_or_else(|| invalid(&self.directory.path, "stage has no replacement consent"))?;
        if replacement.name != name {
            return Err(invalid(
                &self.directory.path,
                "replacement basename differs from consent",
            ));
        }
        if !supported(&self.directory) {
            return Err(unsupported(
                &self.directory.path,
                "recovery publication requires a local ext filesystem",
            ));
        }
        self.directory.revalidate(cancel)?;
        replacement.expected.revalidate(cancel)?;
        let expected_identity = replacement.expected.entry().identity;
        let expected_revision = replacement.revision;
        let recovery = create_recovery(&self.directory, cancel)?;
        let retained = recovery.path.join("item");
        let file = self.file().map_err(|e| error(&retained, e))?;
        let staged_identity = identity(file, &retained)?;
        let descriptor = format!("/proc/self/fd/{}", file.as_raw_fd());
        checked(cancel, &retained)?;
        linkat(
            CWD,
            descriptor.as_str(),
            &*recovery.file,
            "item",
            AtFlags::SYMLINK_FOLLOW,
        )
        .map_err(|e| error(&retained, e.into()))?;
        // The linked verified bytes now belong to persistent recovery. No subsequent error
        // or Drop may silently remove this link. Retrying requires an explicit new job.
        self.published = true;
        let path = self.directory.path.join(name);
        let stage_receipt = TransferRecovery {
            path: retained.clone(),
            identity: Some(staged_identity),
        };
        let preparation = self
            .directory
            .revalidate(cancel)
            .and_then(|()| replacement.expected.revalidate(cancel));
        if let Err(error) = preparation {
            return Ok(TransferPublication::RecoveryRequired {
                published: None,
                recovery: vec![stage_receipt],
                error,
            });
        }
        before_exchange();
        if let Err(error) = checked(cancel, &path) {
            return Ok(TransferPublication::RecoveryRequired {
                published: None,
                recovery: vec![stage_receipt],
                error,
            });
        }
        // Unlike O_TMPFILE, renameat2 takes a source name. The private directory
        // excludes other UIDs, not this UID/root. Detect a substituted source link;
        // never report an exchange of an unverified inode as a successful copy.
        let linked_identity =
            open_target(&recovery, OsStr::new("item")).and_then(|file| identity(&file, &retained));
        match linked_identity {
            Ok(linked) if linked == staged_identity => {}
            Ok(linked) => {
                return Ok(TransferPublication::RecoveryRequired {
                    published: None,
                    recovery: vec![TransferRecovery {
                        path: retained,
                        identity: Some(linked),
                    }],
                    error: invalid(&path, "the protected recovery stage identity changed"),
                });
            }
            Err(error) => {
                let entries = if error.kind == io::ErrorKind::NotFound {
                    vec![]
                } else {
                    vec![TransferRecovery {
                        path: retained,
                        identity: None,
                    }]
                };
                return Ok(TransferPublication::RecoveryRequired {
                    published: None,
                    recovery: entries,
                    error,
                });
            }
        }
        if let Err(failure) = renameat_with(
            &*recovery.file,
            "item",
            &*self.directory.file,
            name,
            RenameFlags::EXCHANGE,
        ) {
            return Ok(TransferPublication::RecoveryRequired {
                published: None,
                recovery: vec![stage_receipt],
                error: error(&path, failure.into()),
            });
        }
        self.published_name = Some(name.to_owned());
        let displaced = open_target(&recovery, OsStr::new("item"));
        let displaced = match displaced {
            Ok(file) => file,
            Err(error) => {
                // The exchanged object is retained even if metadata lookup fails. The
                // original consent pin still identifies only the expected object, so use
                // a conservative unknown identity rather than falsely claiming a match.
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
        let inspect = identity(&displaced, &retained)
            .and_then(|id| Revision::of(&displaced, &retained).map(|revision| (id, revision)));
        let (displaced_identity, revision) = match inspect {
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
            .and_then(|()| self.directory.file.sync_all())
        {
            return Ok(TransferPublication::RecoveryRequired {
                published: Some(path.clone()),
                recovery: vec![receipt],
                error: error(&path, failure),
            });
        }
        Ok(TransferPublication::Replaced {
            path,
            recovery: receipt,
        })
    }
}

fn create_recovery(directory: &Directory, cancel: &Cancellation) -> Result<Directory, FsError> {
    create_named_directory(directory, cancel, ".midnight-recovery-")
}

fn create_named_directory(
    directory: &Directory,
    cancel: &Cancellation,
    prefix: &str,
) -> Result<Directory, FsError> {
    for _ in 0..16 {
        let mut random = [0u8; 16];
        File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut random))
            .map_err(|e| error(&directory.path, e))?;
        let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let name = OsString::from(format!("{prefix}{suffix}"));
        match directory.create_directory(&name, cancel) {
            Ok(created) => {
                let file = File::from(
                    openat2(
                        &*directory.file,
                        &name,
                        DIRECTORY | OFlags::NOFOLLOW,
                        Mode::empty(),
                        RESOLVE,
                    )
                    .map_err(|e| error(&directory.path, e.into()))?,
                );
                let metadata = file.metadata().map_err(|e| error(&directory.path, e))?;
                if identity(&file, &directory.path)? != created.identity()
                    || metadata.uid() != rustix::process::geteuid().as_raw()
                    || metadata.mode() & 0o077 != 0
                {
                    return Err(unsupported(
                        &directory.path,
                        "recovery directory lost private ownership",
                    ));
                }
                return Directory::from_file(file, directory.path.join(name));
            }
            Err(failure) if failure.kind == io::ErrorKind::AlreadyExists => continue,
            Err(failure) => return Err(failure),
        }
    }
    Err(invalid(
        &directory.path,
        "exclusive recovery directory names collided",
    ))
}

#[cfg(test)]
#[path = "transfer_recovery_tests.rs"]
mod tests;
