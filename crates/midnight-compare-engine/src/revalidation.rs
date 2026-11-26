//! Revalidation of the two captured comparison sources. Owned by the comparison worker.
//!
//! A captured revision includes the file's change time (NTFS ChangeTime, Linux ctime),
//! which also moves on metadata-only updates: endpoint data loss prevention, for example,
//! adds an alternate data stream to some freshly written files 0.1–2 s after creation. When
//! a fresh capture of the same path differs from the captured revision only in that change
//! time, its complete content is read and the fresh capture replaces the source only if the
//! content is exactly what the comparison retained. Any other revision difference, content
//! that differs or cannot be read, and image sources (the comparison retains their decoded
//! pixels, not their bytes) still report the file as changed.
use crate::{
    check_cancel,
    text::{self, ReadError, TextComparison, TextInput},
};
use midnight_plugin_host::CapturedFile;
use std::sync::atomic::AtomicBool;

/// Native revision layouts: prefix, payload bytes and the trailing change-time bytes.
/// Windows: volume, file ID, length, LastWriteTime and ChangeTime. Linux: device, inode,
/// length, mtime and ctime, each time as seconds and nanoseconds. Each layout has a
/// filesystem-plugin prefix and an in-process capture prefix. No other format qualifies.
const LAYOUTS: [(&[u8], usize, usize); 4] = [
    (b"native.windows.read.v1", 48, 8),
    (b"windows-file-v1", 48, 8),
    (b"native.linux.read.v1", 56, 16),
    (b"linux-file-v1", 56, 16),
];

/// Whether `current` is the revision `captured` with only a different change time: the
/// same known layout with equal identity, length and write time.
pub(crate) fn change_time_only(captured: &[u8], current: &[u8]) -> bool {
    LAYOUTS.iter().any(|&(prefix, payload, change)| {
        let kept = prefix.len() + payload - change;
        captured.len() == prefix.len() + payload
            && current.len() == captured.len()
            && captured.starts_with(prefix)
            && captured[..kept] == current[..kept]
            && captured[kept..] != current[kept..]
    })
}

/// A fresh capture of the source's path whose revision differs from `source` only in its
/// change time. Revalidation errors carry no revision, so the fresh capture supplies it.
fn drifted(
    source: &CapturedFile,
    recapture: impl FnOnce() -> Result<CapturedFile, String>,
) -> Option<CapturedFile> {
    let fresh = recapture().ok()?;
    change_time_only(&source.stable_version(), &fresh.stable_version()).then_some(fresh)
}

/// Revalidate both sources. A source whose revision drifted only in its change time is
/// replaced by its fresh capture when that capture's complete text equals the text retained
/// for its side; strict decoding makes equal text and encoding equal bytes. The content is
/// read like the initial read: at most 64 MiB in 256 KiB blocks, each admitted by the shared
/// I/O gate, with cancellation between blocks. Returns whether a source was replaced, which
/// invalidates work keyed to the previous capture.
pub(crate) fn revalidate(
    sources: &mut [CapturedFile; 2],
    retained: Option<&TextComparison>,
    recapture: impl Fn(usize) -> Result<CapturedFile, String>,
    confirmed_ssd: [bool; 2],
    cancel: &AtomicBool,
) -> Result<bool, String> {
    let mut replaced = false;
    for (side, source) in sources.iter_mut().enumerate() {
        let Err(error) = source.revalidate() else {
            continue;
        };
        let unchanged = retained.and_then(|text| {
            let fresh = drifted(source, || recapture(side))?;
            let input = text::read(&fresh, confirmed_ssd[side], cancel, &mut |_| {}).ok()?;
            text.retains(side, &input).then_some(fresh)
        });
        let Some(fresh) = unchanged else {
            check_cancel(cancel)?;
            return Err(format!("Compared file changed: {error}"));
        };
        *source = fresh;
        replaced = true;
    }
    Ok(replaced)
}

/// The initial text read. A read that failed because the source's revision drifted only in
/// its change time is repeated once on the fresh capture, which then replaces the source:
/// nothing was derived from the interrupted read. Any other failure stands, and content
/// that text comparison refuses stays a refusal (the pair is then compared as bytes).
pub(crate) fn read_text(
    source: &mut CapturedFile,
    recapture: impl FnOnce() -> Result<CapturedFile, String>,
    confirmed_ssd: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<TextInput, ReadError> {
    let error = match text::read(source, confirmed_ssd, cancel, progress) {
        Ok(input) => return Ok(input),
        Err(error) => error,
    };
    // A source that still validates failed for another reason, such as its encoding.
    let fresh = source
        .revalidate()
        .err()
        .and_then(|_| drifted(source, recapture));
    let Some(fresh) = fresh else {
        check_cancel(cancel).map_err(ReadError::Failed)?;
        return Err(error);
    };
    let input = text::read(&fresh, confirmed_ssd, cancel, progress)?;
    *source = fresh;
    Ok(input)
}

#[cfg(test)]
#[path = "revalidation_tests.rs"]
mod tests;
