//! Small trust-relevant files, such as plugin manifests, pinned by revision and by content.
use crate::{CapturedFile, Error, api::viewer};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{path::Path, sync::Mutex};

/// The complete bytes of a small trust-relevant file, such as a plugin manifest, and the
/// native revision they were read under. Callers parse and check `bytes`, never a later read.
///
/// The revision is exact, change time (NTFS ChangeTime, Linux ctime) included: whoever can
/// write the file can set its modification time back after changing it, and the change time
/// then still shows the change. The change time also moves on metadata-only updates, such as
/// an alternate data stream, attribute or ACL that endpoint protection or a backup agent
/// sets. So when `revalidate` finds that the change time alone moved (the same volume and
/// file ID, or device and inode, the same length and write time), it reads the file again
/// completely under a fresh capture of the same path and accepts it only when those bytes
/// equal `bytes`; the fresh capture then replaces the pinned one. Any other difference, a
/// fresh capture or read that fails, and different content are refused. A check that the
/// pinned revision passes reads no content.
///
/// LiveValidated, as every capture: this detects changes, it does not prevent them. Capture
/// and validation are backend work.
pub struct ManifestPin {
    bytes: Box<[u8]>,
    capture: Mutex<CapturedFile>,
    #[cfg(test)]
    rereads: AtomicUsize,
}

impl std::fmt::Debug for ManifestPin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManifestPin")
            .field("length", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

impl ManifestPin {
    /// Capture `path` (absolute) and read its complete content. A file longer than
    /// `max_bytes`, or than one bounded read (256 KiB), is refused.
    pub fn capture(path: &Path, max_bytes: u64) -> Result<Self, Error> {
        let capture = CapturedFile::capture(path)?;
        let bytes = read_complete(&capture, max_bytes)?;
        Ok(Self {
            bytes: bytes.into_boxed_slice(),
            capture: Mutex::new(capture),
            #[cfg(test)]
            rereads: AtomicUsize::new(0),
        })
    }

    /// The content read at capture, which the pin keeps proving on `revalidate`.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Check that the file at the pinned path is still the pinned one, as described on
    /// [`ManifestPin`]. The error of a refused change time keeps the revision check's
    /// message ("Source changed since capture ...") and adds why the content check failed.
    pub fn revalidate(&self) -> Result<(), Error> {
        let mut pinned = self
            .capture
            .lock()
            .map_err(|_| Error("Manifest pin is unusable after an interrupted check".into()))?;
        let Err(error) = pinned.revalidate() else {
            return Ok(());
        };
        let Ok(fresh) = CapturedFile::capture(pinned.path()) else {
            return Err(error);
        };
        if !pinned.change_time_alone_moved(&fresh) {
            return Err(error);
        }
        #[cfg(test)]
        self.rereads.fetch_add(1, Ordering::Relaxed);
        match read_complete(&fresh, self.bytes.len() as u64) {
            Ok(bytes) if *bytes == *self.bytes => {
                *pinned = fresh;
                Ok(())
            }
            Ok(_) => Err(Error(format!(
                "{error}; read again, its content differs from the validated content"
            ))),
            Err(reread) => Err(Error(format!("{error}; reading it again failed: {reread}"))),
        }
    }

    /// How many times `revalidate` read the content again.
    #[cfg(test)]
    pub(crate) fn rereads(&self) -> usize {
        self.rereads.load(Ordering::Relaxed)
    }
}

/// The capture's complete content, read once under its revision checks.
fn read_complete(capture: &CapturedFile, max_bytes: u64) -> Result<Vec<u8>, Error> {
    let budget = max_bytes.min(viewer::MAX_PAGE_BYTES as u64);
    let length = capture.length();
    if length > budget {
        return Err(Error(format!(
            "Manifest of {length} bytes exceeds its {budget}-byte budget"
        )));
    }
    if length == 0 {
        // No read to validate; the revision check still applies.
        capture.revalidate()?;
        return Ok(Vec::new());
    }
    let bytes = capture.read_at(0, length as u32)?;
    if bytes.len() as u64 != length {
        return Err(Error("Manifest read returned another length".into()));
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "manifest_pin_tests.rs"]
mod tests;
