//! A Copy job's private staging folder in one destination folder (WIN-8; TRANSFERS.md,
//! "Verification and ownership"). The folder is created in one namespace turn before the
//! job's first stage there, as a `.midnight-recovery-*` directory is, and its private pin
//! covers everything below it: stages are created in it through its handle outside any
//! turn, each with the security a file created directly in the destination gets
//! (`security`), and each is published by one handle-relative no-replace rename out of it.
//! A destination folder that refuses Midnight its security, new folders or new files gets
//! no staging folder: each stage there is a `.me-*` entry of its own, as outside a job.
//! `finish` removes the folder, empty, through its handle in one turn; a folder that holds
//! another entry is kept, never listed or emptied, and reported by its exact path. A
//! removal that fails otherwise keeps the folder held for the job's later attempts.
use super::*;
use std::{
    os::windows::io::OwnedHandle,
    sync::{Mutex, MutexGuard},
};

/// The staging of one destination folder, shared by the clones of its `Directory`.
pub(super) struct Staging {
    state: Mutex<State>,
    /// The job's staging folders whose removal failed, shared by every folder of the job.
    left: Arc<Left>,
    /// Whether this is the folder the job started from, whose `finish` comes at the job's
    /// end: it reports a folder whose removal is still refused then.
    root: bool,
}

impl Default for Staging {
    /// The staging of the folder a job starts from.
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            left: Arc::default(),
            root: true,
        }
    }
}

impl Staging {
    /// The staging of a folder below `parent` in the same job.
    pub(super) fn below(parent: &Self) -> Self {
        Self {
            state: Mutex::default(),
            left: parent.left.clone(),
            root: false,
        }
    }
}

#[derive(Default)]
enum State {
    /// No staging folder: one is created before the next stage.
    #[default]
    Unknown,
    /// Not local fixed NTFS, where Midnight creates no private folders (see
    /// `recovery::supported`), or a folder that refuses what the staging folder needs:
    /// each stage is a `.me-*` entry of its own there.
    Unsupported,
    Folder(Folder),
}

struct Folder {
    /// The `.midnight-stage-*` folder: owner-only, non-inheritable DACL; held with DELETE
    /// and without delete sharing, so its name stays fixed while the job holds it.
    private: Directory,
    /// The destination folder itself, reopened with READ_CONTROL: each stage's security
    /// comes from its current descriptor.
    destination: File,
    /// The token whose owner, primary group and default DACL a new file gets.
    token: OwnedHandle,
    /// The folder's namespace parent authority, so that each publication out of the folder
    /// finds it pooled instead of capturing the folder's ancestry again. Only a cache: a
    /// capture that fails here fails the publication's own, which reports it.
    held: Option<super::super::namespace::HeldParent>,
}

/// What a stage of a job needs besides the access of an ordinary content-only stage.
const STAGE_ACCESS: u32 =
    FILE_READ_DATA | FILE_WRITE_DATA | FILE_READ_ATTRIBUTES | DELETE | SYNCHRONIZE;

/// A stage of the job in `directory`'s staging folder, created first if the job has none
/// there. None where stages stay `.me-*` entries of their own: outside a job, whose turns
/// are what the folder saves, where `recovery::supported` does not hold, and where the
/// folder refuses the staging folder access.
pub(super) fn stage(
    directory: &Directory,
    cancel: &Cancellation,
) -> Result<Option<Stage>, FsError> {
    stage_with(directory, 0, cancel)
}

/// `stage` whose handle also has `access` (FILE_WRITE_ATTRIBUTES for a job that preserves
/// times and attributes).
pub(super) fn stage_with(
    directory: &Directory,
    access: u32,
    cancel: &Cancellation,
) -> Result<Option<Stage>, FsError> {
    if midnight_platform_common::transfer_namespace::current_transfer_namespace_changes().is_none()
    {
        return Ok(None);
    }
    checked(cancel, &directory.path)?;
    let mut state = directory
        .staging
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if matches!(*state, State::Unknown) {
        *state = if recovery::supported(directory) {
            match Folder::create(directory, cancel) {
                Ok(folder) => State::Folder(folder),
                // A folder that lets Midnight create files but not read its security or
                // create folders, such as a write-only drop folder, or one that refuses new
                // files: a stage of its own works, or fails before the copy as it always
                // did. The refusal created nothing.
                Err(failure) if failure.kind == io::ErrorKind::PermissionDenied => {
                    State::Unsupported
                }
                // Anything else (a cancellation, a refused turn, a full volume) leaves no
                // folder and fails this stage; the next one tries again.
                Err(failure) => return Err(failure),
            }
        } else {
            State::Unsupported
        };
    }
    let State::Folder(folder) = &*state else {
        return Ok(None);
    };
    folder.stage(directory, access, cancel).map(Some)
}

/// Remove `directory`'s staging folder, if the job has one there (`finish_staging`), and
/// retry the job's folders whose removal failed before.
pub(super) fn finish(directory: &Directory) -> Result<(), FsError> {
    let staging = &directory.staging;
    let folder = {
        let mut state = staging
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match std::mem::take(&mut *state) {
            State::Folder(folder) => Some(folder),
            unchanged => {
                *state = unchanged;
                None
            }
        }
    };
    if let Some(folder) = folder {
        staging.left.remove(folder.into_private())?;
    }
    staging.left.retry(staging.root)
}

impl Folder {
    fn create(directory: &Directory, cancel: &Cancellation) -> Result<Self, FsError> {
        // ReOpenFile cannot reopen directories: an empty handle-relative name reopens the
        // retained object, never its display path. Compare its identity before trusting it.
        let destination = native::reopen(
            &directory.file,
            READ_CONTROL | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        )
        .map_err(|e| error(&directory.path, e))?;
        if identity(&destination, &directory.path)? != directory.identity {
            return Err(invalid(
                &directory.path,
                "destination directory changed before staging",
            ));
        }
        // Each publication out of the folder adds a file to the destination: one that
        // refuses that gets no staging folder, so its files are refused before their copy
        // rather than after it. The access is checked only, not held.
        native::reopen(
            &directory.file,
            FILE_ADD_FILE | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        )
        .map_err(|e| error(&directory.path, e))?;
        let token = security::process_token().map_err(|e| error(&directory.path, e))?;
        let private = super::super::namespace::change(cancel, |mutation| {
            recovery::staging_folder_in(directory, cancel, mutation)
        })?;
        let held = super::super::namespace::hold_parent(&private.file, &private.path, cancel).ok();
        Ok(Self {
            private,
            destination,
            token,
            held,
        })
    }

    /// Create a stage in this folder through its handle, outside any namespace turn: the
    /// folder's pin, reserved when it was created, covers it.
    fn stage(
        &self,
        directory: &Directory,
        access: u32,
        cancel: &Cancellation,
    ) -> Result<Stage, FsError> {
        let mut descriptor =
            security::FileDescriptor::inherited_from(&self.destination, &self.token)
                .map_err(|e| error(&directory.path, e))?;
        for _ in 0..16 {
            checked(cancel, &self.private.path)?;
            let name = stage_name(&self.private.path)?;
            let path = self.private.path.join(&name);
            let created = descriptor.create(
                &self.private.file,
                &name,
                STAGE_ACCESS | access | descriptor.access(),
                FILE_SHARE_READ,
            );
            let file = match created {
                Ok(file) => file,
                Err(failure) if failure.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(failure) => return Err(error(&path, failure)),
            };
            let namespace_owner = super::super::namespace::owner_for(&file, &path);
            let mut stage = Stage {
                file,
                directory: directory.clone(),
                published: false,
                published_name: None,
                cleaned: false,
                path,
                replacement: None,
                metadata: false,
                attributes: access & FILE_WRITE_ATTRIBUTES != 0,
                preserved: None,
                _namespace_owner: namespace_owner,
            };
            initialize_stage(&mut stage, |stage| {
                descriptor
                    .complete(&stage.file)
                    .map_err(|e| error(&stage.path, e))
            })?;
            return Ok(stage);
        }
        Err(error(
            &self.private.path,
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "exclusive staging names collided repeatedly",
            ),
        ))
    }

    /// The folder alone, for its removal: what only staging needs is released first.
    fn into_private(self) -> Directory {
        let Self {
            private,
            destination,
            token,
            held,
        } = self;
        drop((held, destination, token));
        private
    }
}

/// What one attempt to remove a staging folder left.
enum Removal {
    Removed,
    /// The kernel refused a folder that still holds an entry: kept on disk, never listed
    /// or emptied, and no longer held.
    Released(FsError),
    /// Any other failure, such as the job's namespace coordination refusing the turn's
    /// change while a peer is latched: nothing changed, and the folder is still held.
    Held(Directory, FsError),
}

/// Remove an empty staging folder through its handle in one turn.
fn remove_folder(private: Directory) -> Removal {
    let mut held = Some(private);
    let removed = super::super::namespace::change(&Cancellation::default(), |mutation| {
        let removed = held
            .as_ref()
            .map_or(Ok(()), |private| private.remove_empty_in(mutation));
        if removed.is_ok() {
            // Closed in the turn, so that its finish certifies the removal.
            held = None;
        }
        removed
    });
    match (removed, held) {
        (Ok(()), _) => Removal::Removed,
        (Err(failure), Some(private)) if failure.kind != io::ErrorKind::DirectoryNotEmpty => {
            // Named by the folder it leaves, whatever step refused.
            let failure = FsError {
                path: private.path.clone(),
                ..failure
            };
            Removal::Held(private, failure)
        }
        (Err(failure), _) => Removal::Released(failure),
    }
}

/// A staging folder whose removal failed, held for the job's later attempts.
struct Leftover {
    private: Directory,
    failure: FsError,
    /// Whether the job was told; a refusal is told only if it lasts to the job's end.
    reported: bool,
}

/// The job's staging folders whose removal failed. Every later `finish` of the job retries
/// them, and the last folder of the job to go tries once more.
#[derive(Default)]
struct Left(Mutex<Vec<Leftover>>);

impl Left {
    fn lock(&self) -> MutexGuard<'_, Vec<Leftover>> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Remove `private`. A refusal by the job's namespace coordination is no failure of the
    /// job: the folder is held for a later attempt. Another failure is reported, and the
    /// folder held all the same.
    fn remove(&self, private: Directory) -> Result<(), FsError> {
        let (private, failure) = match remove_folder(private) {
            Removal::Removed => return Ok(()),
            Removal::Released(failure) => return Err(failure),
            Removal::Held(private, failure) => (private, failure),
        };
        let reported = failure.kind != io::ErrorKind::WouldBlock;
        self.lock().push(Leftover {
            private,
            failure: failure.clone(),
            reported,
        });
        if reported { Err(failure) } else { Ok(()) }
    }

    /// Retry the held folders. One that now holds an entry is released and reported; the
    /// others stay held. At the job's end (`last`), a refusal not yet told is reported.
    fn retry(&self, last: bool) -> Result<(), FsError> {
        let pending = std::mem::take(&mut *self.lock());
        let mut outcome = Ok(());
        let mut kept = Vec::with_capacity(pending.len());
        for leftover in pending {
            // One report per call; the rest wait for the next.
            if outcome.is_err() {
                kept.push(leftover);
                continue;
            }
            match remove_folder(leftover.private) {
                Removal::Removed => {}
                Removal::Released(failure) => outcome = Err(failure),
                Removal::Held(private, failure) => kept.push(Leftover {
                    private,
                    failure,
                    reported: leftover.reported,
                }),
            }
        }
        if last
            && outcome.is_ok()
            && let Some(leftover) = kept.iter_mut().find(|leftover| !leftover.reported)
        {
            leftover.reported = true;
            outcome = Err(leftover.failure.clone());
        }
        self.lock().extend(kept);
        outcome
    }
}

impl Drop for Left {
    /// The last folder of the job is gone: one more attempt each. Nobody is left to report
    /// a failure to.
    fn drop(&mut self) {
        let left = std::mem::take(self.0.get_mut().unwrap_or_else(|error| error.into_inner()));
        for leftover in left {
            let _ = remove_folder(leftover.private);
        }
    }
}

impl Drop for Staging {
    /// A job that did not finish this folder's staging (`finish_staging`): remove the
    /// folder as far as possible. Nobody is left to report a failure to; a folder that
    /// cannot be removed now is held for the job's later attempts.
    fn drop(&mut self) {
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        if let State::Folder(folder) = std::mem::take(state) {
            let _ = self.left.remove(folder.into_private());
        }
    }
}
