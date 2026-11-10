//! Metadata-only qualification shared by native Move and automatic drag intent.
use super::*;
use rustix::fs::{AtFlags, StatxFlags, statx};

pub(super) fn local_filesystem(file: &File, path: &Path) -> Result<bool, FsError> {
    let stat = rustix::fs::fstatfs(file).map_err(|error| super::error(path, error.into()))?;
    // Remote failures can have committed on the server. Do not advertise native
    // rename semantics from a mount identifier alone on unknown filesystems.
    Ok(matches!(
        stat.f_type as u64,
        0xef53 | 0x5846_5342 | 0x9123_683e | 0x0102_1994 // ext, XFS, Btrfs, tmpfs
    ))
}

pub(super) fn mount(file: &File, path: &Path) -> Result<Option<u64>, FsError> {
    match statx(file, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID) {
        Ok(stat) => Ok((stat.stx_mask & StatxFlags::MNT_ID.bits() != 0).then_some(stat.stx_mnt_id)),
        Err(rustix::io::Errno::NOSYS | rustix::io::Errno::OPNOTSUPP) => Ok(None),
        Err(failure) => Err(error(path, failure.into())),
    }
}

pub(super) fn drop_device(path: &Path, cancel: &Cancellation) -> Result<Option<u64>, FsError> {
    checked(cancel, path)?;
    let path = absolute(path)?;
    let file = File::from(
        openat(
            CWD,
            &path,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|failure| error(&path, failure.into()))?,
    );
    let result = if local_filesystem(&file, &path)? {
        // Mount identity, not st_dev: bind mounts can share a device but reject
        // rename across their boundary. This hint never grants mutation rights.
        mount(&file, &path)?
    } else {
        None
    };
    checked(cancel, &path)?;
    Ok(result)
}
