//! Fresh handle-only revisions for independent Move proof reads.
use super::destination::recovery::Revision;
use super::*;
use midnight_core::{TransferReadGuard, TransferRevisionPolicy};
use std::{
    io::{Seek, SeekFrom},
    path::PathBuf,
};

struct ReadGuard {
    file: File,
    revision: Revision,
    path: PathBuf,
    // Last: a returned independent readback guard closes its duplicate before retirement.
    _namespace_owner: super::namespace::NativeOwner,
}
impl TransferReadGuard for ReadGuard {
    /// Copy and Move verification only: a moved ChangeTime alone is no change (VIEW-05).
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.path)?;
        if !self.revision.matches(
            Revision::of(&self.file, &self.path)?,
            TransferRevisionPolicy::AcceptChangeTimeOnly,
        ) {
            return Err(invalid(
                &self.path,
                "retained file changed during validation",
            ));
        }
        Ok(())
    }
}

pub(super) fn rewind(
    file: &File,
    path: &Path,
    cancel: &Cancellation,
    track_identity: bool,
) -> Result<Box<dyn TransferReadGuard>, FsError> {
    checked(cancel, path)?;
    // Cloning retains the exact object and file position; it never resolves its old
    // pathname. Reads and seeks are serialized by the backend transfer gate.
    let mut held = file.try_clone().map_err(|e| error(path, e))?;
    let revision = Revision::of(&held, path)?;
    held.seek(SeekFrom::Start(0)).map_err(|e| error(path, e))?;
    // A FAT stage has opaque namespace ownership and no rename-stable identity.
    let owner = if track_identity {
        super::namespace::try_owner_for(&held, path)?
    } else {
        Default::default()
    };
    Ok(Box::new(ReadGuard {
        file: held,
        revision,
        path: path.to_owned(),
        _namespace_owner: owner,
    }))
}
