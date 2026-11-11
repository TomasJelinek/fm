//! Read an immutable captured report on a backend worker, with a bounded modal preview.
use midnight_core::{
    Cancellation, DIAGNOSTIC_PREVIEW_BYTES, DiagnosticTextPreview, FsError, GeneratedTextSource,
};
use std::{io, path::Path};

pub(crate) fn read(
    source: &dyn GeneratedTextSource,
    cancel: &Cancellation,
) -> Result<DiagnosticTextPreview, FsError> {
    check(cancel)?;
    let total_bytes = source.length();
    let limit = total_bytes.min(DIAGNOSTIC_PREVIEW_BYTES as u64) as usize;
    let mut bytes = vec![0; limit];
    let mut offset = 0;
    while offset < limit {
        check(cancel)?;
        let end = (offset + 8 * 1024).min(limit);
        let count = source
            .read_at(offset as u64, &mut bytes[offset..end])
            .map_err(|error| FsError::from_io("read diagnostic report", Path::new(""), error))?;
        check(cancel)?;
        if count == 0 || count > end - offset {
            return Err(error(
                io::ErrorKind::UnexpectedEof,
                "The diagnostic report ended before its recorded length.",
            ));
        }
        offset += count;
    }
    check(cancel)?;
    if source.length() != total_bytes {
        return Err(error(
            io::ErrorKind::InvalidData,
            "The diagnostic report changed its recorded length.",
        ));
    }
    let truncated = total_bytes > limit as u64;
    if let Err(invalid) = std::str::from_utf8(&bytes) {
        if truncated && invalid.error_len().is_none() {
            // A byte budget can split the last scalar, but never replace malformed
            // interior bytes or expose a partial character to the UI.
            bytes.truncate(invalid.valid_up_to());
        } else {
            return Err(error(
                io::ErrorKind::InvalidData,
                "The diagnostic report is not UTF-8.",
            ));
        }
    }
    let text = String::from_utf8(bytes).map_err(|_| {
        error(
            io::ErrorKind::InvalidData,
            "The diagnostic report is not UTF-8.",
        )
    })?;
    check(cancel)?;
    Ok(DiagnosticTextPreview {
        text,
        total_bytes,
        truncated,
    })
}

fn check(cancel: &Cancellation) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(error(
            io::ErrorKind::Interrupted,
            "The diagnostic request was cancelled.",
        ))
    } else {
        Ok(())
    }
}

fn error(kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io(
        "read diagnostic report",
        Path::new(""),
        io::Error::new(kind, message),
    )
}

#[cfg(test)]
#[path = "diagnostic_text_tests.rs"]
mod tests;
