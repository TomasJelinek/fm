use super::SpoolSpace;
use crate::store::native::Root;
use rustix::fs::{self, Mode, OFlags};
use std::{fs::File, io, os::unix::fs::FileExt};

#[cfg(test)]
#[path = "spool_linux_tests.rs"]
mod tests;

pub(super) fn create(root: &Root) -> io::Result<File> {
    // O_EXCL prevents later linking of this O_TMPFILE inode into any namespace.
    // Creation is handle-relative, anonymous from the start, owned and mode0600.
    fs::openat(
        root.directory(),
        ".",
        OFlags::TMPFILE | OFlags::EXCL | OFlags::RDWR | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map(File::from)
    .map_err(|error| {
        io::Error::new(
            io::Error::from(error).kind(),
            format!("Cannot create an anonymous private spool on this filesystem: {error}"),
        )
    })
}

pub(super) fn read_at(file: &File, offset: u64, output: &mut [u8]) -> io::Result<usize> {
    file.read_at(output, offset)
}

/// Handle-relative: the verified root descriptor names the volume, never a path.
pub(super) fn space(root: &Root) -> io::Result<SpoolSpace> {
    let volume = fs::fstatvfs(root.directory())?;
    Ok(SpoolSpace {
        available: volume.f_bavail.saturating_mul(volume.f_frsize),
        total: volume.f_blocks.saturating_mul(volume.f_frsize),
    })
}
