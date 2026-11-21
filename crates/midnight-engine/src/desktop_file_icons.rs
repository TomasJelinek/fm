//! Visible tiles of files that carry their own artwork, such as programs, share the
//! serial desktop icon actor with type icons and command artwork. Each view replaces
//! and cancels the previous one: a tile scrolled out of view is not loaded.
use super::*;

/// Retained bytes of one tile identity; thumbnail identities are far shorter.
const MAX_IDENTITY_BYTES: usize = 512;
/// Retained bytes of one path, beyond the longest native path.
const MAX_PATH_BYTES: usize = 128 * 1024;

#[derive(Clone, Default)]
pub(super) struct View {
    pub generation: u64,
    pub targets: Vec<FileIconTarget>,
    pub cancel: Cancellation,
}

/// The targets of the current view still to load. Dropping it, when a newer view
/// replaces it, cancels the load in progress.
#[derive(Default)]
pub(super) struct Pending {
    pub generation: u64,
    pub targets: VecDeque<FileIconTarget>,
    pub cancel: Cancellation,
}

impl From<View> for Pending {
    fn from(view: View) -> Self {
        Self {
            generation: view.generation,
            targets: view.targets.into(),
            cancel: view.cancel,
        }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Tiles remembered as having no artwork of their own.
const MAX_ABSENT: usize = 4096;

/// Tiles whose files were read and had no artwork of their own, such as most
/// programs in System32. They are not read again while their identity, which names
/// the listed file revision, stays the same; the oldest are forgotten first.
#[derive(Default)]
pub(super) struct Absent {
    order: VecDeque<String>,
    identities: HashSet<String>,
}

impl Absent {
    pub fn contains(&self, identity: &str) -> bool {
        self.identities.contains(identity)
    }

    pub fn insert(&mut self, identity: String) {
        if self.identities.insert(identity.clone()) {
            self.order.push_back(identity);
            if self.order.len() > MAX_ABSENT
                && let Some(oldest) = self.order.pop_front()
            {
                self.identities.remove(&oldest);
            }
        }
    }
}

/// Bounded presentation metadata: at most `MAX_FILE_ICON_TARGETS` distinct, bounded
/// identities, absolute paths without NUL and edges that are drawn.
pub(super) fn valid_targets(targets: &[FileIconTarget]) -> bool {
    if targets.len() > MAX_FILE_ICON_TARGETS {
        return false;
    }
    let mut identities = HashSet::new();
    targets.iter().all(|target| {
        let path = target.path.as_os_str().as_encoded_bytes();
        !target.identity.is_empty()
            && target.identity.capacity() <= MAX_IDENTITY_BYTES
            && target.path.capacity() <= MAX_PATH_BYTES
            && target.path.is_absolute()
            && !path.contains(&0)
            && (1..=MAX_ICON_EDGE).contains(&target.edge)
            && identities.insert(target.identity.as_str())
    })
}

/// The file's own artwork at `edge`, read between two revalidations of a native
/// capture of the file: a file that changed meanwhile yields nothing, and no direct
/// path is read when the capture is refused.
pub(super) fn load(
    service: Option<&dyn DesktopServices>,
    capture: Option<&crate::archives::ArchiveCapture>,
    path: &Path,
    edge: u16,
    cancel: &Cancellation,
) -> Option<IconPixels> {
    if cancel.is_cancelled() {
        return None;
    }
    let service = service?;
    let source = (capture?)(path, None, cancel).ok()?;
    source.revalidate().ok()?;
    if cancel.is_cancelled() {
        return None;
    }
    let pixels = service.load_file_icon(path, edge, cancel).ok()??;
    source.revalidate().ok()?;
    let edge = u32::from(edge);
    (!cancel.is_cancelled()
        && super::command_icons::valid_pixels(&pixels)
        && pixels.width <= edge
        && pixels.height <= edge)
        .then_some(pixels)
}

#[cfg(test)]
#[path = "desktop_file_icons_tests.rs"]
mod tests;
