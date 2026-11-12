//! Live-validated removal of the selected file in its private temporary directory.
//! Checks detect observed changes; Linux has no content/identity-conditional unlink.
use super::super::destination::recovery::Revision;
use super::*;
use midnight_core::{TransferMoveFinalizeOutcome, TransferMoveFinalizer, TransferSourceRemoval};

pub(super) fn qualify(file: &File, path: &Path) -> Result<(), FsError> {
    let metadata = file.metadata().map_err(|e| error(path, e))?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(unsupported(
            path,
            "Verified Move requires an owned single-link regular file; source removal refused",
        ));
    }
    Ok(())
}

pub(super) fn prepare(
    capture: &Capture,
    cancel: &Cancellation,
) -> Result<Box<dyn TransferMoveFinalizer>, FsError> {
    checked(cancel, &capture.entry.item.path)?;
    let lease = capture.read_lease(cancel)?;
    let state = capture.state.lock().unwrap_or_else(|e| e.into_inner());
    let Some(TransferMoveCaptureOutcome::Captured { source, recovery }) = &state.outcome else {
        return Err(invalid(
            &capture.entry.item.path,
            "source has no successful temporary staging",
        ));
    };
    if recovery.identity != Some(capture.entry.identity) {
        return Err(invalid(
            source,
            "staged source identity differs from the selected original",
        ));
    }
    let directory = state
        .retained
        .as_ref()
        .ok_or_else(|| invalid(source, "temporary source directory authority missing"))?
        .clone();
    let finalizer = Finalizer {
        source: source.clone(),
        receipt: recovery.clone(),
        directory,
        name: capture.name.clone(),
        file: capture._file.clone(),
        lease,
        revision: Revision::of(&capture._file, source)?,
        outcome: None,
    };
    finalizer.revalidate(cancel)?;
    Ok(Box::new(finalizer))
}

struct Finalizer {
    source: PathBuf,
    receipt: TransferRecovery,
    directory: destination::Directory,
    name: OsString,
    file: Arc<File>,
    lease: Arc<lease::ReadLease>,
    revision: Revision,
    outcome: Option<TransferMoveFinalizeOutcome>,
}

impl TransferMoveFinalizer for Finalizer {
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.source)?;
        self.lease.revalidate(&self.source)?;
        self.directory.revalidate(cancel)?;
        qualify(&self.file, &self.source)?;
        if self.receipt.identity != Some(identity(&self.file, &self.source)?)
            || Revision::of(&self.file, &self.source)? != self.revision
            || self
                .directory
                .inspect(&self.name, cancel)?
                .map(|entry| entry.identity)
                != self.receipt.identity
        {
            return Err(invalid(
                &self.source,
                "source revision or temporary binding changed; original retained",
            ));
        }
        self.lease.revalidate(&self.source)
    }

    fn finalize(&mut self, cancel: &Cancellation) -> TransferMoveFinalizeOutcome {
        if let Some(outcome) = &self.outcome {
            return outcome.clone();
        }
        let removal = self.revalidate(cancel).and_then(|()| {
            checked(cancel, &self.source)?;
            // No path reopen or referent traversal. The directory is private and
            // held; the selected inode, its revision and lease were just checked.
            // This is a live operation, not atomic compare-and-unlink. Arbitrary
            // descheduling across a forced lease timeout remains a documented limit.
            rustix::fs::unlinkat(
                self.directory.native_file(),
                &self.name,
                rustix::fs::AtFlags::empty(),
            )
            .map_err(|e| error(&self.source, e.into()))
        });
        let outcome = match removal {
            Ok(()) => TransferMoveFinalizeOutcome::Removed {
                source: self.source.clone(),
                recovery: self.receipt.clone(),
                state: TransferSourceRemoval::Unlinked,
                durability_confirmed: self.directory.native_file().sync_all().is_ok(),
            },
            Err(error) => TransferMoveFinalizeOutcome::Retained {
                source: self.source.clone(),
                recovery: self.receipt.clone(),
                error,
            },
        };
        self.outcome = Some(outcome.clone());
        outcome
    }
}
