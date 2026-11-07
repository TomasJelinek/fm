use midnight_core::{
    ARCHIVE_ASSOCIATIONS, ArchiveFormat, Entry, EntryId, EntryKind, archive_association,
};
use std::{collections::HashSet, path::Path};

#[test]
fn known_associations_are_unique_and_preserve_native_document_opening_by_default() {
    let extensions: HashSet<_> = ARCHIVE_ASSOCIATIONS
        .iter()
        .map(|item| item.extension)
        .collect();
    assert_eq!(extensions.len(), ARCHIVE_ASSOCIATIONS.len());
    for extension in [
        "zip", "whl", "egg", "war", "ear", "aar", "aab", "nupkg", "snupkg",
    ] {
        let name = format!("文档.{extension}");
        let association = archive_association(Path::new(&name)).unwrap();
        assert_eq!(association.format, ArchiveFormat::Zip);
        assert!(association.enabled_by_default, "{extension}");
    }
    for extension in [
        "docx", "xlsx", "pptx", "odt", "ods", "odp", "epub", "cbz", "kmz", "kra", "ora", "jar",
        "pyz", "pyzw", "apk", "vsix", "xpi",
    ] {
        let name = format!("Document.{}", extension.to_ascii_uppercase());
        let association = archive_association(Path::new(&name)).unwrap();
        assert_eq!(association.format, ArchiveFormat::Zip);
        assert!(!association.enabled_by_default, "{extension}");
    }
    let compound = archive_association(Path::new("releases/source.TAR.GZ")).unwrap();
    assert_eq!(compound.format, ArchiveFormat::TarGzip);
    assert_eq!(compound.extension, "tar.gz");
    for name in [
        "document.doc",
        "payload.whl.exe",
        "archive.zip.tmp",
        "directory.zip/file",
    ] {
        assert!(archive_association(Path::new(name)).is_none(), "{name}");
    }
}

#[test]
fn only_regular_native_files_receive_backend_archive_hints() {
    for kind in [
        EntryKind::Directory,
        EntryKind::DirectoryLink,
        EntryKind::Symlink,
        EntryKind::Other,
    ] {
        assert!(
            Entry::new(EntryId(1), "directory.zip".into(), kind)
                .archive_association
                .is_none()
        );
    }
    assert!(
        Entry::new(EntryId(1), "package.whl".into(), EntryKind::File)
            .archive_association
            .is_some()
    );
}
