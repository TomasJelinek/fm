//! Native rename or verified-copy cleanup of the retained original directory link.
use super::*;
use midnight_core::{
    TransferDirectoryLinkMoveCapture, TransferDirectoryLinkRemoval,
    TransferNativeMoveOutcome as Outcome, TransferReplacementPolicy, TransferSourceRemoval,
};
use windows_sys::{
    Wdk::Storage::FileSystem::FILE_DIRECTORY_FILE, Win32::Foundation::ERROR_NOT_SAME_DEVICE,
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
        FILE_DIRECTORY_FILE,
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
        file,
        proof,
        policy,
        state: Mutex::new(State::Captured),
        _namespace_owner: owner,
    });
    capture.revalidate(cancel)?;
    Ok(capture)
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
    file: File,
    proof: Arc<source_links::OriginalLink>,
    policy: CopyLinkPolicy,
    state: Mutex<State>,
    _namespace_owner: super::super::namespace::NativeOwner,
}

enum State {
    Captured,
    Removed,
    Renamed {
        target: TransferIdentity,
        path: PathBuf,
        outcome: Outcome,
    },
}

impl Capture {
    fn confirm_renamed(
        &self,
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
                FILE_DIRECTORY_FILE,
            )
            .map_err(|e| error(path, e))?;
            // The file ID and full payload still belong to the stub at the new slot.
            self.proof.validate_binding(&visible, path)
        })();
        match verify {
            Ok(()) => Outcome::Moved {
                destination: path.to_owned(),
                recovery: Vec::new(),
                // Unless saving a folder it changed fails (`folder_flush::confirm`).
                durability_confirmed: true,
            },
            Err(error) => {
                // The native rename committed. Recoverable location is reported from the
                // held stub; neither a retry nor rollback through a changing path is safe.
                let recovery = destination::final_path(&self.file, VOLUME_NAME_GUID)
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
        let source = &self.entry().item.path;
        validate_name(name).map_err(|e| error(target.path(), e))?;
        let path = target.path().join(name);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        match &*state {
            State::Captured => {}
            State::Removed => return Err(invalid(source, "original directory link was removed")),
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
                return Ok(outcome.clone());
            }
        }
        if replace.is_some() {
            return Err(native_move::collision(
                &path,
                "native replacement of a directory link Move is unsupported",
            ));
        }
        self.revalidate(cancel)?;
        target.revalidate(cancel)?;
        if self.entry().identity.device != target.identity().device {
            return Ok(Outcome::CrossDevice);
        }
        let directory = native_move::pin_destination(target, cancel)?;
        if destination::final_path(directory.native_file(), VOLUME_NAME_GUID).is_err() {
            return Ok(Outcome::Unsupported);
        }
        let same_parent = self.parent_identity == directory.identity();
        let outcome = if same_parent && name == self.name {
            Outcome::SameEntry
        } else {
            let existing = directory.inspect(name, cancel)?;
            if let Some(other) = existing
                && (other.identity != self.entry().identity
                    || !same_parent
                    || native_move::exact_alias(&directory, &self.file, name, cancel)?)
            {
                return Err(native_move::collision(&path, "destination already exists"));
            }
            super::super::namespace::change(cancel, |mutation| {
                mutation.prepare_rename(&self.file, directory.native_file(), name, cancel)?;
                self.revalidate(cancel)?;
                directory.revalidate(cancel)?;
                checked(cancel, source)?;
                match mutation.rename(&self.file, directory.native_file(), name, cancel) {
                    Ok(()) => Ok(self.confirm_renamed(&directory, name, &path)),
                    Err(failure)
                        if failure.raw_os_error() == Some(ERROR_NOT_SAME_DEVICE as i32) =>
                    {
                        Ok(Outcome::CrossDevice)
                    }
                    Err(failure) => Err(error(&path, failure)),
                }
            })?
        };
        // A renamed link left its folder for the destination: save both before reporting it.
        let outcome = super::folder_flush::confirm(
            outcome,
            &[
                (directory.native_file(), directory.identity()),
                (self.parent.as_ref(), self.parent_identity),
            ],
        );
        if !matches!(outcome, Outcome::CrossDevice | Outcome::Unsupported) {
            *state = State::Renamed {
                target: target.identity(),
                path,
                outcome: outcome.clone(),
            };
        }
        Ok(outcome)
    }

    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let path = &self.entry().item.path;
        checked(cancel, path)?;
        self.parent_binding.revalidate(path, cancel)?;
        let current = native::open_relative(
            &self.parent,
            &self.name,
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            FILE_DIRECTORY_FILE,
        )
        .map_err(|e| error(path, e))?;
        self.proof.validate_for_move(&current)?;
        self.proof.validate_for_move(&self.file)
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
            super::super::namespace::change(cancel, |mutation| {
                mutation
                    .remove(&self.file, cancel, || {
                        self.revalidate(cancel)
                            .map_err(|error| io::Error::new(error.kind, error))?;
                        if cancel.is_cancelled() {
                            return Err(io::ErrorKind::Interrupted.into());
                        }
                        native::remove(&self.file)
                    })
                    .map_err(|e| error(path, e))
            })?;
            *state = State::Removed;
        }
        Ok(TransferDirectoryLinkRemoval::Removed {
            state: TransferSourceRemoval::DeletePending,
            durability_confirmed: false,
            recovery: Vec::new(),
            error: None,
        })
    }
}
