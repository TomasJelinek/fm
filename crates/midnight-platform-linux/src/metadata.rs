//! Linux reports compression only when statx explicitly supports that attribute.
use midnight_core::{CompressionStatus, EntryMetadata, FsError, PropertyAccount, PropertyDetails};
use rustix::fs::{
    AtFlags, CWD, FileType, Statx, StatxAttributes, StatxFlags, StatxTimestamp, statx,
};
use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Properties details of `path` itself, never following a final link: owner and
/// group with their account names, the birth time only when the filesystem reports
/// one (statx `STATX_BTIME`), the modified and accessed times and the mode.
pub(crate) fn details(path: &Path) -> Result<PropertyDetails, FsError> {
    match statx(
        CWD,
        path,
        AtFlags::SYMLINK_NOFOLLOW | AtFlags::NO_AUTOMOUNT,
        StatxFlags::BASIC_STATS | StatxFlags::BTIME,
    ) {
        Ok(stat) => Ok(decode_details(&stat)),
        // Without statx no birth time is available; the rest still is.
        Err(rustix::io::Errno::NOSYS) => fallback_details(path),
        Err(error) => Err(FsError::from_io("read properties", path, error.into())),
    }
}

fn decode_details(stat: &Statx) -> PropertyDetails {
    let mask = StatxFlags::from_bits_retain(stat.stx_mask);
    let time = |flag: StatxFlags, value: StatxTimestamp| {
        mask.contains(flag)
            .then(|| timestamp(value.tv_sec, value.tv_nsec))
            .flatten()
    };
    PropertyDetails {
        owner: mask.contains(StatxFlags::UID).then(|| PropertyAccount {
            id: stat.stx_uid,
            name: crate::ownership::user_name(stat.stx_uid),
        }),
        group: mask.contains(StatxFlags::GID).then(|| PropertyAccount {
            id: stat.stx_gid,
            name: crate::ownership::group_name(stat.stx_gid),
        }),
        created: time(StatxFlags::BTIME, stat.stx_btime),
        modified: time(StatxFlags::MTIME, stat.stx_mtime),
        accessed: time(StatxFlags::ATIME, stat.stx_atime),
        unix_mode: mask
            .contains(StatxFlags::MODE)
            .then_some(u32::from(stat.stx_mode)),
    }
}

fn fallback_details(path: &Path) -> Result<PropertyDetails, FsError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| FsError::from_io("read properties", path, error))?;
    Ok(PropertyDetails {
        owner: Some(PropertyAccount {
            id: metadata.uid(),
            name: crate::ownership::user_name(metadata.uid()),
        }),
        group: Some(PropertyAccount {
            id: metadata.gid(),
            name: crate::ownership::group_name(metadata.gid()),
        }),
        created: None,
        modified: metadata.modified().ok(),
        accessed: metadata.accessed().ok(),
        unix_mode: Some(metadata.mode()),
    })
}

pub(crate) fn read(path: &Path) -> Result<EntryMetadata, FsError> {
    match statx(
        CWD,
        path,
        AtFlags::SYMLINK_NOFOLLOW | AtFlags::NO_AUTOMOUNT,
        StatxFlags::BASIC_STATS,
    ) {
        Ok(stat) => Ok(decode(&stat)),
        // Older kernels can still supply ordinary metadata. Unknown compression
        // is preferable to guessing from sparse blocks, file suffixes or size.
        Err(rustix::io::Errno::NOSYS) => fallback_metadata(path),
        Err(error) => Err(FsError::from_io("read metadata", path, error.into())),
    }
}

fn decode(stat: &Statx) -> EntryMetadata {
    let mask = StatxFlags::from_bits_retain(stat.stx_mask);
    let is_file = mask.contains(StatxFlags::TYPE)
        && FileType::from_raw_mode(u32::from(stat.stx_mode)) == FileType::RegularFile;
    let compression = if !stat
        .stx_attributes_mask
        .contains(StatxAttributes::COMPRESSED)
    {
        CompressionStatus::Unknown
    } else if stat.stx_attributes.contains(StatxAttributes::COMPRESSED) {
        CompressionStatus::Compressed
    } else {
        CompressionStatus::Uncompressed
    };
    EntryMetadata {
        size: (is_file && mask.contains(StatxFlags::SIZE)).then_some(stat.stx_size),
        modified: mask
            .contains(StatxFlags::MTIME)
            .then(|| timestamp(stat.stx_mtime.tv_sec, stat.stx_mtime.tv_nsec))
            .flatten(),
        compression,
        localized_modified: mask
            .contains(StatxFlags::MTIME)
            .then(|| timestamp(stat.stx_mtime.tv_sec, stat.stx_mtime.tv_nsec))
            .flatten()
            .and_then(crate::locale::modified),
        native_attributes: None,
        unix_mode: mask
            .contains(StatxFlags::MODE)
            .then_some(u32::from(stat.stx_mode)),
    }
}

fn fallback_metadata(path: &Path) -> Result<EntryMetadata, FsError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| FsError::from_io("read metadata", path, error))?;
    Ok(EntryMetadata {
        size: metadata.is_file().then_some(metadata.len()),
        modified: metadata.modified().ok(),
        localized_modified: metadata.modified().ok().and_then(crate::locale::modified),
        unix_mode: Some(metadata.mode()),
        ..EntryMetadata::default()
    })
}

pub(crate) fn timestamp(seconds: i64, nanoseconds: u32) -> Option<SystemTime> {
    if nanoseconds >= 1_000_000_000 {
        return None;
    }
    let whole = Duration::from_secs(seconds.unsigned_abs());
    let base = if seconds < 0 {
        UNIX_EPOCH.checked_sub(whole)?
    } else {
        UNIX_EPOCH.checked_add(whole)?
    };
    base.checked_add(Duration::from_nanos(u64::from(nanoseconds)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{CompressionStatus, DirectoryService};
    use rustix::fs::{AtFlags, CWD, FileType, StatxAttributes, StatxFlags, statx};
    use std::{
        fs,
        time::{Duration, UNIX_EPOCH},
    };

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf, Statx) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("文档 e\u{301} 👩🏽‍💻.zip");
        fs::write(&path, b"ordinary archive-named file").unwrap();
        let stat = statx(
            CWD,
            &path,
            AtFlags::SYMLINK_NOFOLLOW | AtFlags::NO_AUTOMOUNT,
            StatxFlags::BASIC_STATS,
        )
        .unwrap();
        (directory, path, stat)
    }

    #[test]
    fn compression_requires_supported_native_attribute_not_sparse_size_ratio() {
        let (_directory, _path, mut stat) = fixture();
        stat.stx_attributes = StatxAttributes::COMPRESSED;
        stat.stx_attributes_mask = StatxAttributes::COMPRESSED;
        assert_eq!(decode(&stat).compression, CompressionStatus::Compressed);
        stat.stx_attributes_mask = StatxAttributes::empty();
        assert_eq!(decode(&stat).compression, CompressionStatus::Unknown);
        stat.stx_attributes_mask = StatxAttributes::COMPRESSED;
        stat.stx_attributes = StatxAttributes::empty();
        stat.stx_size = 1_000_000;
        stat.stx_blocks = 0;
        assert_eq!(decode(&stat).compression, CompressionStatus::Uncompressed);
    }

    #[test]
    fn logical_lengths_type_masks_and_negative_timestamps_are_preserved() {
        let (_directory, _path, mut stat) = fixture();
        stat.stx_mask = (StatxFlags::TYPE | StatxFlags::SIZE | StatxFlags::MTIME).bits();
        stat.stx_mode = FileType::RegularFile.as_raw_mode() as u16;
        stat.stx_size = 12345;
        stat.stx_blocks = 0;
        stat.stx_mtime.tv_sec = -1;
        stat.stx_mtime.tv_nsec = 500_000_000;
        let metadata = decode(&stat);
        assert_eq!(metadata.size, Some(12345));
        assert_eq!(
            metadata.modified,
            UNIX_EPOCH.checked_sub(Duration::from_millis(500))
        );
        stat.stx_mode = FileType::Directory.as_raw_mode() as u16;
        assert_eq!(decode(&stat).size, None);
        stat.stx_mask = 0;
        assert_eq!(decode(&stat).size, None);
        assert_eq!(decode(&stat).modified, None);
        assert_eq!(decode(&stat).unix_mode, None);
        stat.stx_mask = StatxFlags::MTIME.bits();
        stat.stx_mtime.tv_nsec = 1_000_000_000;
        assert_eq!(decode(&stat).modified, None);
    }

    #[test]
    fn native_metadata_keeps_literal_names_and_never_follows_final_symlinks() {
        use std::os::unix::fs::symlink;
        let (directory, path, stat) = fixture();
        let result = crate::LinuxFileSystem::new(None)
            .entry_metadata(&path)
            .unwrap();
        assert_eq!(
            result.size,
            Some(b"ordinary archive-named file".len() as u64)
        );
        let expected = if !stat
            .stx_attributes_mask
            .contains(StatxAttributes::COMPRESSED)
        {
            CompressionStatus::Unknown
        } else if stat.stx_attributes.contains(StatxAttributes::COMPRESSED) {
            CompressionStatus::Compressed
        } else {
            CompressionStatus::Uncompressed
        };
        assert_eq!(result.compression, expected);
        assert_eq!(result.native_attributes, None);
        assert_eq!(result.unix_mode, Some(u32::from(stat.stx_mode)));
        let link = directory.path().join("link");
        symlink(&path, &link).unwrap();
        assert_eq!(read(&link).unwrap().size, None);
        fs::remove_file(&path).unwrap();
        assert_eq!(read(&link).unwrap().size, None);
        assert_eq!(read(&path).unwrap_err().kind, std::io::ErrorKind::NotFound);
    }

    #[test]
    fn property_details_name_the_owner_and_report_birth_time_only_when_provided() {
        use std::os::unix::fs::MetadataExt;
        let (directory, path, mut stat) = fixture();
        let read = details(&path).unwrap();
        let metadata = fs::symlink_metadata(&path).unwrap();
        let owner = read.owner.clone().unwrap();
        assert_eq!(owner.id, metadata.uid());
        assert_eq!(read.group.as_ref().unwrap().id, metadata.gid());
        assert_eq!(read.unix_mode, Some(metadata.mode()));
        assert_eq!(read.modified, metadata.modified().ok());
        assert_eq!(read.accessed, metadata.accessed().ok());
        // The birth time is the filesystem's own, or absent.
        assert_eq!(read.created, metadata.created().ok());
        stat.stx_mask &= !StatxFlags::BTIME.bits();
        assert_eq!(decode_details(&stat).created, None);
        stat.stx_mask = StatxFlags::BTIME.bits();
        stat.stx_btime.tv_sec = 1_700_000_000;
        stat.stx_btime.tv_nsec = 5;
        let decoded = decode_details(&stat);
        assert_eq!(
            decoded.created,
            UNIX_EPOCH.checked_add(Duration::new(1_700_000_000, 5))
        );
        assert_eq!(
            (decoded.owner, decoded.group, decoded.unix_mode),
            (None, None, None)
        );
        // A final link is described itself, never its target.
        let link = directory.path().join("link");
        std::os::unix::fs::symlink("missing target", &link).unwrap();
        let mode = details(&link).unwrap().unix_mode.unwrap();
        assert_eq!(FileType::from_raw_mode(mode), FileType::Symlink);
    }

    #[test]
    fn account_names_come_from_the_account_files_or_stay_numeric() {
        assert_eq!(crate::ownership::user_name(0).as_deref(), Some("root"));
        assert_eq!(crate::ownership::group_name(0).as_deref(), Some("root"));
        assert_eq!(crate::ownership::user_name(u32::MAX - 7), None);
    }

    #[test]
    fn fifo_metadata_needs_no_reader_or_writer_and_has_no_regular_file_size() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("metadata-only FIFO 文档");
        rustix::fs::mkfifoat(CWD, &path, rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR).unwrap();
        let result = read(&path).unwrap();
        assert_eq!(result.size, None);
        assert!(result.modified.is_some());
    }
}
