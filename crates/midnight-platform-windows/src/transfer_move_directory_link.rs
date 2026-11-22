//! Native rename or verified-copy cleanup of the retained original directory link.
use super::*;
use midnight_core::{
    DeferredNativeMove, TransferDirectoryLinkMoveCapture, TransferDirectoryLinkRemoval,
    TransferNativeMoveOutcome as Outcome, TransferReplacementPolicy, TransferSourceRemoval,
};
use windows_sys::{
    Wdk::Storage::FileSystem::{FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE},
    Win32::Foundation::ERROR_NOT_SAME_DEVICE,
};

pub(super) fn prepare(
    parent: Arc<File>,
    parent_path: PathBuf,
    name: &OsStr,
    proof: Arc<source_links::OriginalLink>,
    policy: CopyLinkPolicy,
    cancel: &Cancellation,
) -> Result<Arc<dyn TransferDirectoryLinkMoveCapture>, FsError> {
    let path = &proof.entry.item.path;
    checked(cancel, path)?;
    let parent_binding = source_links::MoveParent::capture(&parent, &parent_path, path, cancel)?;
    let file = native::open_relative(
        &parent,
        name,
        FILE_READ_ATTRIBUTES | DELETE,
        FILE_SHARE_READ,
        FILE_OPEN,
        link_options(&proof),
    )
    .map_err(|e| error(path, e))?;
    if destination::final_path(&file, VOLUME_NAME_GUID).is_err() {
        return Err(unsupported(
            path,
            "Move requires a local original directory link",
        ));
    }
    let parent_identity = identity(&parent, path)?;
    let owner = super::super::namespace::owner(&[parent_identity, proof.entry.identity]);
    let capture = Arc::new(Capture {
        parent,
        parent_identity,
        parent_path,
        parent_binding,
        name: name.to_owned(),
        file: Mutex::new(Some(file)),
        proof,
        policy,
        state: Mutex::new(State::Captured),
        _namespace_owner: owner,
    });
    capture.revalidate(cancel)?;
    Ok(capture)
}

/// How the original link is opened: a link to a folder as a folder, any other as a file.
fn link_options(proof: &source_links::OriginalLink) -> u32 {
    if proof.directory() {
        FILE_DIRECTORY_FILE
    } else {
        FILE_NON_DIRECTORY_FILE
    }
}

pub(super) fn deferred_source(
    original: File,
    proof: Arc<source_links::OriginalLink>,
) -> Box<dyn TransferSource> {
    let mut entry = proof.entry.clone();
    entry.item.kind = EntryKind::Directory;
    Box::new(DeferredSource {
        original,
        proof,
        entry,
    })
}

struct DeferredSource {
    original: File,
    proof: Arc<source_links::OriginalLink>,
    entry: TransferEntry,
}

impl TransferSource for DeferredSource {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }

    fn native_identity(&self) -> Option<TransferIdentity> {
        None
    }

    fn native_original_identity(&self) -> Option<TransferIdentity> {
        Some(self.proof.entry.identity)
    }

    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.entry.item.path)?;
        self.proof.validate_for_move(&self.original)
    }

    fn read_chunk(&mut self, _: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        Err(unsupported(
            &self.entry.item.path,
            "directory link Move has no projected file reader",
        ))
    }

    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        Err(unsupported(
            &self.entry.item.path,
            "directory link Move requires a qualified Copy projection",
        ))
    }
}

struct Capture {
    parent: Arc<File>,
    parent_identity: TransferIdentity,
    parent_path: PathBuf,
    parent_binding: source_links::MoveParent,
    name: OsString,
    /// The link's only handle with DELETE access, which also denies others delete
    /// sharing. Its removal closes it (lock order: `state`, then this).
    file: Mutex<Option<File>>,
    proof: Arc<source_links::OriginalLink>,
    policy: CopyLinkPolicy,
    state: Mutex<State>,
    _namespace_owner: super::super::namespace::NativeOwner,
}

enum State {
    Captured,
    /// Whether the removal is confirmed: the link's name is gone and its folder saved.
    Removed {
        confirmed: bool,
    },
    Renamed {
        target: TransferIdentity,
        path: PathBuf,
        outcome: Outcome,
    },
}

impl Capture {
    /// The held link handle, or an error once its removal closed it.
    fn held<'a>(&self, held: &'a Option<File>) -> Result<&'a File, FsError> {
        held.as_ref().ok_or_else(|| {
            invalid(
                &self.entry().item.path,
                "original directory link was removed",
            )
        })
    }

    fn confirm_renamed(
        &self,
        file: &File,
        directory: &destination::Directory,
        name: &OsStr,
        path: &Path,
    ) -> Outcome {
        let verify = (|| {
            directory.revalidate(&Cancellation::default())?;
            let visible = native::open_relative(
                directory.native_file(),
                name,
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_OPEN,
                link_options(&self.proof),
            )
            .map_err(|e| error(path, e))?;
            // The file ID and full payload still belong to the stub at the new slot.
            self.proof.validate_binding(&visible, path)
        })();
        match verify {
            Ok(()) => Outcome::Moved {
                destination: path.to_owned(),
                recovery: Vec::new(),
                // Unless saving a folder it changed fails (`folder_flush::settle`).
                durability_confirmed: true,
            },
            Err(error) => {
                // The native rename committed. Recoverable location is reported from the
                // held stub; neither a retry nor rollback through a changing path is safe.
                let recovery = destination::final_path(file, VOLUME_NAME_GUID)
                    .ok()
                    .map(|path| TransferRecovery {
                        path,
                        identity: Some(self.entry().identity),
                    })
                    .into_iter()
                    .collect();
                Outcome::RecoveryRequired {
                    destination: None,
                    recovery,
                    error,
                }
            }
        }
    }
}

impl TransferDirectoryLinkMoveCapture for Capture {
    fn entry(&self) -> &TransferEntry {
        &self.proof.entry
    }

    fn prepare_copy_projection(
        &self,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.revalidate(cancel)?;
        let source = source::Source::open_with_links(
            self.parent.clone(),
            self.parent_path.clone(),
            &self.name,
            self.policy,
            cancel,
        )?;
        if source.native_original_identity() != Some(self.entry().identity) {
            return Err(invalid(
                &self.entry().item.path,
                "Copy projection has a different original directory link",
            ));
        }
        self.revalidate(cancel)?;
        Ok(Box::new(source))
    }

    fn try_native_move(
        &self,
        target: &dyn TransferDirectory,
        name: &OsStr,
        replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        cancel: &Cancellation,
    ) -> Result<Outcome, FsError> {
        self.rename(target, name, replace, cancel, false)
            .map(|(outcome, _)| outcome)
    }

    fn try_native_move_deferred(
        &self,
        target: &dyn TransferDirectory,
        name: &OsStr,
        replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        cancel: &Cancellation,
    ) -> Result<DeferredNativeMove, FsError> {
        self.rename(target, name, replace, cancel, true)
    }

    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let held = self.file.lock().unwrap_or_else(|error| error.into_inner());
        self.revalidate_held(self.held(&held)?, cancel)
    }

    fn remove_original(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferDirectoryLinkRemoval, FsError> {
        let path = &self.entry().item.path;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if matches!(&*state, State::Renamed { .. }) {
            return Err(invalid(
                path,
                "original directory link already entered native Move",
            ));
        }
        if matches!(&*state, State::Captured) {
            let mut held = self.file.lock().unwrap_or_else(|error| error.into_inner());
            let file = self.held(&held)?;
            super::super::namespace::change(cancel, |mutation| {
                mutation
                    .remove(file, cancel, || {
                        self.revalidate_held(file, cancel)
                            .map_err(|error| io::Error::new(error.kind, error))?;
                        if cancel.is_cancelled() {
                            return Err(io::ErrorKind::Interrupted.into());
                        }
                        native::remove(file)
                    })
                    .map_err(|e| error(path, e))
            })?;
            // Windows takes the link out of its folder when this, its only deleting
            // handle, closes; then its absence is checked and the folder saved, before the
            // removal is reported (TRANSFERS.md, "Move durability on Windows").
            drop(held.take());
            drop(held);
            *state = State::Removed {
                confirmed: super::folder_flush::removed(
                    &self.parent,
                    self.parent_identity,
                    &self.name,
                ),
            };
        }
        let State::Removed { confirmed } = *state else {
            unreachable!("the link was removed above or before");
        };
        Ok(TransferDirectoryLinkRemoval::Removed {
            state: if confirmed {
                TransferSourceRemoval::Unlinked
            } else {
                TransferSourceRemoval::DeletePending
            },
            durability_confirmed: confirmed,
            recovery: Vec::new(),
            error: None,
        })
    }
}

impl Capture {
    /// The native rename of the link (`try_native_move`); `deferred` leaves saving the
    /// folders it changed to the returned confirmation (`folder_flush::settle`).
    fn rename(
        &self,
        target: &dyn TransferDirectory,
        name: &OsStr,
        replace: Option<(Arc<dyn TransferTarget>, TransferReplacementPolicy)>,
        cancel: &Cancellation,
        deferred: bool,
    ) -> Result<DeferredNativeMove, FsError> {
        let source = &self.entry().item.path;
        validate_name(name).map_err(|e| error(target.path(), e))?;
        let path = target.path().join(name);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        match &*state {
            State::Captured => {}
            State::Removed { .. } => {
                return Err(invalid(source, "original directory link was removed"));
            }
            State::Renamed {
                target: prior,
                path: previous,
                outcome,
            } => {
                if *prior != target.identity() || *previous != path {
                    return Err(invalid(
                        source,
                        "native Move already finished for another slot",
                    ));
                }
                return Ok((outcome.clone(), None));
            }
        }
        if replace.is_some() {
            return Err(native_move::collision(
                &path,
                "native replacement of a directory link Move is unsupported",
            ));
        }
        let held = self.file.lock().unwrap_or_else(|error| error.into_inner());
        let file = self.held(&held)?;
        self.revalidate_held(file, cancel)?;
        target.revalidate(cancel)?;
        if self.entry().identity.device != target.identity().device {
            return Ok((Outcome::CrossDevice, None));
        }
        let directory = native_move::pin_destination(target, cancel)?;
        if destination::final_path(directory.native_file(), VOLUME_NAME_GUID).is_err() {
            return Ok((Outcome::Unsupported, None));
        }
        let same_parent = self.parent_identity == directory.identity();
        let outcome = if same_parent && name == self.name {
            Outcome::SameEntry
        } else {
            let existing = directory.inspect(name, cancel)?;
            if let Some(other) = existing
                && (other.identity != self.entry().identity
                    || !same_parent
                    || native_move::exact_alias(&directory, file, name, cancel)?)
            {
                return Err(native_move::collision(&path, "destination already exists"));
            }
            super::super::namespace::change(cancel, |mutation| {
                mutation.prepare_rename(file, directory.native_file(), name, cancel)?;
                self.revalidate_held(file, cancel)?;
                directory.revalidate(cancel)?;
                checked(cancel, source)?;
                match mutation.rename(file, directory.native_file(), name, cancel) {
                    Ok(()) => Ok(self.confirm_renamed(file, &directory, name, &path)),
                    Err(failure)
                        if failure.raw_os_error() == Some(ERROR_NOT_SAME_DEVICE as i32) =>
                    {
                        Ok(Outcome::CrossDevice)
                    }
                    Err(failure) => Err(error(&path, failure)),
                }
            })?
        };
        if matches!(outcome, Outcome::CrossDevice) {
            return Ok((outcome, None));
        }
        // A renamed link left its folder for the destination: save both, before reporting
        // it or when the engine confirms it.
        let folders = vec![
            super::folder_flush::Held::Directory(directory),
            super::folder_flush::Held::Handle(self.parent.clone(), self.parent_identity),
        ];
        let (outcome, confirmation) = super::folder_flush::settle(outcome, folders, deferred);
        *state = State::Renamed {
            target: target.identity(),
            path,
            outcome: outcome.clone(),
        };
        Ok((outcome, confirmation))
    }

    /// `revalidate` with the held link handle.
    fn revalidate_held(&self, file: &File, cancel: &Cancellation) -> Result<(), FsError> {
        let path = &self.entry().item.path;
        checked(cancel, path)?;
        self.parent_binding.revalidate(path, cancel)?;
        let current = native::open_relative(
            &self.parent,
            &self.name,
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            link_options(&self.proof),
        )
        .map_err(|e| error(path, e))?;
        self.proof.validate_for_move(&current)?;
        self.proof.validate_for_move(file)
    }
}
