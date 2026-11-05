//! Linux reports compression only when statx explicitly supports that attribute.
use fm_core::{CompressionStatus, DirectoryService, EntryMetadata, FsError};
use rustix::fs::{AtFlags, CWD, FileType, Statx, StatxAttributes, StatxFlags, statx};
use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

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
        Err(rustix::io::Errno::NOSYS) => {
            fm_platform_common::LocalServices::default().entry_metadata(path)
        }
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
    }
}

fn timestamp(seconds: i64, nanoseconds: u32) -> Option<SystemTime> {
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
    use fm_core::CompressionStatus;
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
        let link = directory.path().join("link");
        symlink(&path, &link).unwrap();
        assert_eq!(read(&link).unwrap().size, None);
        fs::remove_file(&path).unwrap();
        assert_eq!(read(&link).unwrap().size, None);
        assert_eq!(read(&path).unwrap_err().kind, std::io::ErrorKind::NotFound);
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
