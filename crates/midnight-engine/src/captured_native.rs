//! Worker-only validation shared by explicit desktop consumers of tree captures.
use midnight_core::*;
use std::{io, path::Path};

pub(crate) fn check_cancel(cancel: &Cancellation) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(error(
            io::ErrorKind::Interrupted,
            "Captured file action was canceled.",
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn resolve(
    source: &dyn CapturedClipboardFiles,
    cancel: &Cancellation,
) -> Result<FileClipboard, FsError> {
    check_cancel(cancel)?;
    let intent = source.intent();
    let files = source.resolve(cancel)?;
    check_cancel(cancel)?;
    if files.intent != intent {
        return Err(error(
            io::ErrorKind::InvalidData,
            "Captured file intent changed.",
        ));
    }
    files.validate()?;
    Ok(files)
}

pub(crate) fn error(kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io(
        "captured file action",
        Path::new(""),
        io::Error::new(kind, message),
    )
}

#[cfg(test)]
#[path = "captured_native_tests.rs"]
pub(crate) mod tests;
