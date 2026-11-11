//! Display kind only: keep the original link path and no-follow metadata.
use midnight_core::{Cancellation, Entry, EntryKind};
use rustix::fs::{CWD, Mode, OFlags, openat};

pub(super) fn classify(entry: &mut Entry, cancel: &Cancellation) {
    if entry.kind != EntryKind::Symlink || cancel.is_cancelled() {
        return;
    }
    // O_PATH does not open contents; O_DIRECTORY rejects file/device targets.
    // Failure leaves an ordinary symlink, including broken/inaccessible targets.
    let target = openat(
        CWD,
        &entry.path,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    );
    if !cancel.is_cancelled() && target.is_ok() {
        entry.kind = EntryKind::DirectoryLink;
    }
}
