//! Navigation order and the focused file identity, prepared independently of its parser.
//!
//! Only paths or labels are retained for Space/Backspace navigation. Other entries are
//! captured on demand by [`crate::capture_prefetch`] when a file is about to be opened.
use midnight_core::{EntryKind, ViewerLaunch};
use midnight_plugin_host::{CapturedFile, RevisionPolicy};
use std::{
    ops::Range,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) type Captured = Result<CapturedFile, String>;

/// Captures one entry of an [`Order`] on a backend worker. The flag is the window's shared
/// close signal and is observed for the whole capture.
pub(crate) type CaptureEntry = Arc<dyn Fn(usize, &Arc<AtomicBool>) -> Captured + Send + Sync>;

/// Retained navigation descriptors per viewer window. A folder whose order exceeds it keeps
/// a contiguous window of entries around the focused file instead of refusing to open.
pub(crate) const ORDER_BUDGET: usize = 8 * 1024 * 1024;
/// Charged per native entry in addition to its encoded path bytes: the retained `PathBuf`
/// header, its allocation and slack for the per-entry slot.
const NATIVE_ENTRY: usize = 64;

pub(crate) struct Order {
    /// Native paths or provider labels in visible row order.
    pub files: Arc<[PathBuf]>,
    pub current: usize,
    /// The focused entry, captured before any viewer route is chosen.
    pub focused: Captured,
    /// Captures any other entry on demand; absent when the order has a single entry.
    pub capture: Option<CaptureEntry>,
}

/// The contiguous entries around `focus` whose total cost fits `budget`, grown alternately
/// after and before it. The focused entry is always retained, whatever its own cost.
pub(crate) fn window(
    len: usize,
    focus: usize,
    budget: usize,
    cost: impl Fn(usize) -> usize,
) -> Range<usize> {
    debug_assert!(focus < len);
    let mut range = focus..focus + 1;
    let mut used = cost(focus);
    let mut admit = |index: usize| match used.checked_add(cost(index)) {
        Some(total) if total <= budget => {
            used = total;
            true
        }
        _ => false,
    };
    let (mut after, mut before) = (true, true);
    while after || before {
        after = after && range.end < len && admit(range.end);
        if after {
            range.end += 1;
        }
        before = before && range.start > 0 && admit(range.start - 1);
        if before {
            range.start -= 1;
        }
    }
    range
}

/// In-process native capture. A viewer keeps reading a file whose change time alone moved
/// after capture (VIEW-05); identity, length and write time must still match.
pub(crate) fn native() -> crate::NativeCapture {
    Arc::new(|path, closed| {
        if closed.load(Ordering::Acquire) {
            return Err("Viewer closed".into());
        }
        CapturedFile::capture_with(path, RevisionPolicy::AcceptChangeTimeOnly)
            .map_err(|error| error.to_string())
    })
}

#[cfg(test)]
pub(crate) fn prepare(launch: &ViewerLaunch, closed: &Arc<AtomicBool>) -> Result<Order, String> {
    prepare_with(launch, closed, native())
}

pub(crate) fn prepare_with(
    launch: &ViewerLaunch,
    closed: &Arc<AtomicBool>,
    capture: crate::NativeCapture,
) -> Result<Order, String> {
    if launch.find.is_some() {
        return crate::capture_find::prepare(launch, closed, capture);
    }
    let normalized;
    let launch = if launch.tree.is_some() {
        normalized = crate::capture_tree::normalize(launch, closed)?;
        &normalized
    } else {
        launch
    };
    if launch.generated.is_some() {
        return crate::capture_generated::prepare(launch, closed);
    }
    if launch.archive.is_some() {
        return crate::capture_archive::prepare(launch, closed);
    }
    let mut rows = Vec::new();
    for entry in launch.entries.iter() {
        if closed.load(Ordering::Acquire) {
            return Err("Viewer closed".into());
        }
        if entry.kind != EntryKind::File {
            continue;
        }
        let Some(row) = launch.view.by_id.get(&entry.id) else {
            continue;
        };
        let path = entry
            .native_path()
            .ok_or("The captured native view contains a non-native file.")?;
        rows.push((*row, path));
    }
    rows.sort_unstable_by_key(|(row, _)| *row);
    let focus = rows
        .iter()
        .position(|(_, path)| launch.focused.as_path() == *path)
        .ok_or("The focused regular file is no longer in the captured view.")?;
    let range = window(rows.len(), focus, ORDER_BUDGET, |index| {
        rows[index].1.as_os_str().len().saturating_add(NATIVE_ENTRY)
    });
    let files: Arc<[PathBuf]> = rows[range.clone()]
        .iter()
        .map(|(_, path)| path.to_path_buf())
        .collect();
    let current = focus - range.start;
    let focused = capture(&files[current], closed);
    if closed.load(Ordering::Acquire) {
        return Err("Viewer closed".into());
    }
    let paths = files.clone();
    let entry: CaptureEntry = Arc::new(move |index, closed| {
        let path = paths
            .get(index)
            .ok_or("The requested file is outside the captured order.")?;
        capture(path, closed)
    });
    Ok(Order {
        capture: (files.len() > 1).then_some(entry),
        files,
        current,
        focused,
    })
}

#[cfg(test)]
mod tests {
    use super::window;
    use midnight_plugin_host::CapturedFile;
    use std::sync::{Arc, atomic::AtomicBool};

    /// VIEW-05: the viewer's in-process capture keeps reading after only the change time
    /// moved (the read-only attribute and NTFS ChangeTime on Windows, the mode and ctime
    /// on Linux); a content change is still refused.
    #[test]
    fn native_capture_reads_on_after_only_the_change_time_moved() {
        let root = tempfile::tempdir().unwrap();
        // No extension: endpoint protection on some machines rescans fresh `.txt` files.
        let path = root.path().join("fresh");
        std::fs::write(&path, b"content").unwrap();
        let captured = super::native()(&path, &Arc::new(AtomicBool::new(false))).unwrap();
        let exact = CapturedFile::capture(&path).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        let writable = std::fs::metadata(&path).unwrap().permissions();
        let mut read_only = writable.clone();
        read_only.set_readonly(true);
        std::fs::set_permissions(&path, read_only).unwrap();
        std::fs::set_permissions(&path, writable).unwrap();
        assert!(exact.revalidate().is_err(), "the change time did not move");
        captured.revalidate().unwrap();
        assert_eq!(captured.read_at(0, 7).unwrap(), b"content");
        std::fs::write(&path, b"changed").unwrap();
        assert!(captured.read_at(0, 7).is_err());
    }

    #[test]
    fn window_grows_alternately_around_the_focus_until_the_budget() {
        assert_eq!(window(10, 5, 5, |_| 1), 3..8);
        assert_eq!(window(10, 0, 4, |_| 1), 0..4, "a side at its bound yields");
        assert_eq!(window(10, 9, 3, |_| 1), 7..10);
        assert_eq!(window(4, 2, 100, |_| 1), 0..4, "everything fits");
        assert_eq!(window(3, 1, 0, |_| 7), 1..2, "the focus is always retained");
        // An oversized neighbor stops only its own side; the order stays contiguous.
        assert_eq!(
            window(6, 2, 4, |index| if index == 3 { 9 } else { 1 }),
            0..3
        );
        assert_eq!(
            window(3, 1, usize::MAX, |_| usize::MAX),
            1..2,
            "no overflow"
        );
    }
}
