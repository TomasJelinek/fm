//! Display kind only: keep the original link path and no-follow metadata.
use midnight_core::{Cancellation, Entry, EntryKind};
use rustix::fs::{AtFlags, CWD, FileType, StatxFlags, statx};

pub(super) fn classify(entry: &mut Entry, cancel: &Cancellation) {
    if entry.kind != EntryKind::Symlink || cancel.is_cancelled() {
        return;
    }
    // Only the target's type is read: nothing is opened, a target that is an
    // automount point is not mounted, and network filesystems answer from cached
    // attributes. Intermediate automounts and unresponsive servers along the link's
    // path can still delay the lookup. Failure leaves an ordinary symlink,
    // including broken/inaccessible targets.
    let target = statx(
        CWD,
        &entry.path,
        AtFlags::NO_AUTOMOUNT | AtFlags::STATX_DONT_SYNC,
        StatxFlags::TYPE,
    );
    let directory = target.is_ok_and(|stat| {
        StatxFlags::from_bits_retain(stat.stx_mask).contains(StatxFlags::TYPE)
            && FileType::from_raw_mode(u32::from(stat.stx_mode)) == FileType::Directory
    });
    if !cancel.is_cancelled() && directory {
        entry.kind = EntryKind::DirectoryLink;
    }
}
