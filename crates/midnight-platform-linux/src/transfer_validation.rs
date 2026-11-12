//! Fresh handle-only revisions for independent Move proof reads.
use super::destination::recovery::Revision;
use super::*;
use midnight_core::TransferReadGuard;
use std::{
    io::{Seek, SeekFrom},
    path::PathBuf,
};

struct ReadGuard {
    file: File,
    revision: Revision,
    path: PathBuf,
}
impl TransferReadGuard for ReadGuard {
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.path)?;
        if Revision::of(&self.file, &self.path)? != self.revision {
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
) -> Result<Box<dyn TransferReadGuard>, FsError> {
    checked(cancel, path)?;
    // Cloning retains the exact object and file position; it never resolves its old
    // pathname. Reads and seeks are serialized by the backend transfer gate.
    let mut held = file.try_clone().map_err(|e| error(path, e))?;
    let revision = Revision::of(&held, path)?;
    held.seek(SeekFrom::Start(0)).map_err(|e| error(path, e))?;
    Ok(Box::new(ReadGuard {
        file: held,
        revision,
        path: path.to_owned(),
    }))
}
