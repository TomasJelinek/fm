//! Native metadata attributes are read on backend workers, without opening contents.
use midnight_core::{CompressionStatus, EntryMetadata, FsError};
use std::{fs, os::windows::fs::MetadataExt, path::Path};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_REPARSE_POINT,
};

pub(crate) fn read(path: &Path) -> Result<EntryMetadata, FsError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| FsError::from_io("read metadata", path, error))?;
    Ok(from_native(&metadata))
}

pub(crate) fn from_native(metadata: &fs::Metadata) -> EntryMetadata {
    EntryMetadata {
        size: metadata.is_file().then_some(metadata.len()),
        modified: metadata.modified().ok(),
        localized_modified: metadata.modified().ok().and_then(crate::locale::modified),
        compression: compression_from_attributes(metadata.file_attributes()),
        native_attributes: Some(metadata.file_attributes()),
        unix_mode: None,
    }
}

fn compression_from_attributes(attributes: u32) -> CompressionStatus {
    if attributes & FILE_ATTRIBUTE_COMPRESSED != 0 {
        CompressionStatus::Compressed
    } else if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        // Reparse providers may implement other compression formats. Avoid
        // opening/retrieving their targets or claiming that they are plain files.
        CompressionStatus::Unknown
    } else {
        CompressionStatus::Uncompressed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::DirectoryService;

    #[test]
    fn reported_compression_is_distinct_from_archive_and_sparse_attributes() {
        for flags in [0x800, 0x820, 0xa00, 0xc00] {
            assert_eq!(
                compression_from_attributes(flags),
                CompressionStatus::Compressed
            );
        }
        for flags in [0, 0x20, 0x200, 0x220] {
            assert_eq!(
                compression_from_attributes(flags),
                CompressionStatus::Uncompressed
            );
        }
        assert_eq!(
            compression_from_attributes(0x400),
            CompressionStatus::Unknown
        );
    }

    #[test]
    fn native_metadata_reports_the_os_attribute_and_keeps_logical_size_and_contents() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("文档 e\u{301} 👩🏽‍💻.zip");
        let bytes = b"PK\x03\x04ordinary fixture";
        fs::write(&path, bytes).unwrap();
        let native = fs::symlink_metadata(&path).unwrap();
        let expected = if native.file_attributes() & 0x800 != 0 {
            CompressionStatus::Compressed
        } else {
            CompressionStatus::Uncompressed
        };
        let result = crate::WindowsFileSystem::new(None)
            .entry_metadata(&path)
            .unwrap();
        assert_eq!(result.compression, expected);
        assert_eq!(result.size, Some(bytes.len() as u64));
        assert_eq!(result.modified, native.modified().ok());
        assert_eq!(result.native_attributes, Some(native.file_attributes()));
        assert_eq!(result.unix_mode, None);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let missing = read(&directory.path().join("missing")).unwrap_err();
        assert_eq!(missing.kind, std::io::ErrorKind::NotFound);
    }
}
