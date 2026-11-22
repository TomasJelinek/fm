//! A Copy job's private staging folder in one destination folder (WIN-8; TRANSFERS.md,
//! "Verification and ownership"). The folder is created in one namespace turn before the
//! job's first stage there, as a `.midnight-recovery-*` directory is, and its private pin
//! covers everything below it: stages are created in it through its handle outside any
//! turn, each with the security a file created directly in the destination gets
//! (`security`), and each is published by one handle-relative no-replace rename out of it.
//! `finish` removes it, empty, through its handle in one turn; a folder that holds another
//! entry is kept, never listed or emptied, and reported by its exact path.
use super::*;
use std::{os::windows::io::OwnedHandle, sync::Mutex};

/// The staging of one destination folder, shared by the clones of its `Directory`.
#[derive(Default)]
pub(super) struct Staging(Mutex<State>);

#[derive(Default)]
enum State {
    /// No staging folder: one is created before the next stage.
    #[default]
    Unknown,
    /// Not local fixed NTFS, where Midnight creates no private folders (see
    /// `recovery::supported`): each stage is a `.me-*` entry of its own there.
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
/// are what the folder saves, and where `recovery::supported` does not hold.
pub(super) fn stage(
    directory: &Directory,
    cancel: &Cancellation,
) -> Result<Option<Stage>, FsError> {
    if midnight_platform_common::transfer_namespace::current_transfer_namespace_changes().is_none()
    {
        return Ok(None);
    }
    checked(cancel, &directory.path)?;
    let mut state = directory
        .staging
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if matches!(*state, State::Unknown) {
        *state = if recovery::supported(directory) {
            // A failed creation leaves no folder and fails this stage; the next one tries
            // again. There is no fallback to a stage of its own.
            State::Folder(Folder::create(directory, cancel)?)
        } else {
            State::Unsupported
        };
    }
    let State::Folder(folder) = &*state else {
        return Ok(None);
    };
    folder.stage(directory, cancel).map(Some)
}

/// Remove `directory`'s staging folder, if the job has one there (`finish_staging`).
pub(super) fn finish(directory: &Directory) -> Result<(), FsError> {
    let folder = {
        let mut state = directory
            .staging
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match std::mem::take(&mut *state) {
            State::Folder(folder) => folder,
            unchanged => {
                *state = unchanged;
                return Ok(());
            }
        }
    };
    folder.remove()
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
    fn stage(&self, directory: &Directory, cancel: &Cancellation) -> Result<Stage, FsError> {
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
                STAGE_ACCESS | descriptor.access(),
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

    /// Remove the folder through its handle in one turn; the kernel refuses one that still
    /// holds an entry, which is then kept and reported by its path.
    fn remove(self) -> Result<(), FsError> {
        let Self {
            private,
            destination,
            token,
            held,
        } = self;
        drop((held, destination, token));
        private.remove_empty_stage()
    }
}

impl Drop for Staging {
    /// A job that did not finish this folder's staging (`finish_staging`): remove the
    /// folder as far as possible. Nobody is left to report a failure to.
    fn drop(&mut self) {
        let state = self.0.get_mut().unwrap_or_else(|error| error.into_inner());
        if let State::Folder(folder) = std::mem::take(state) {
            let _ = folder.remove();
        }
    }
}
