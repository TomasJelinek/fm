//! Reparse-point classification shared by every native operation.
//!
//! Only name-surrogate reparse tags (`IsReparseTagNameSurrogate`: symbolic links,
//! junctions and volume mount points, WSL symbolic links, ...) redirect the namespace.
//! Those are links: they are leaves and are never followed. Every other tag (Cloud Files
//! and OneDrive placeholders, deduplication, WOF, HSM, ...) marks an ordinary file or
//! directory whose data a filesystem filter supplies, so it is classified by its
//! directory attribute. Content of such a file must be read through a filter-mediated
//! handle (see [`reopen_content`]) because a `FILE_OPEN_REPARSE_POINT` handle can expose
//! the raw stub instead of the data.
//!
//! FAT32, exFAT and some redirectors do not implement `FileAttributeTagInformation`
//! (the fastfat driver answers `STATUS_INVALID_PARAMETER`). Their attributes come from
//! the basic class every filesystem implements; such filesystems have no reparse points,
//! and a reparse point whose tag cannot be read fails closed as a link.
use midnight_core::EntryKind;
use std::{
    fs::File,
    io,
    mem::size_of,
    os::windows::io::{AsRawHandle, FromRawHandle},
};
use windows_sys::Win32::{
    Foundation::INVALID_HANDLE_VALUE,
    Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
        FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FileAttributeTagInfo, FileBasicInfo, GetFileInformationByHandleEx, ReOpenFile,
    },
};

/// The name-surrogate bit of a reparse tag (`IsReparseTagNameSurrogate`, winnt.h).
const NAME_SURROGATE: u32 = 0x2000_0000;

/// Native attributes plus the reparse tag, which is meaningful only when the
/// attributes contain `FILE_ATTRIBUTE_REPARSE_POINT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Native {
    pub(crate) attributes: u32,
    pub(crate) tag: u32,
}

impl Native {
    /// Combines enumeration-record data. Directory records carry the reparse tag in
    /// `EaSize` whenever `FILE_ATTRIBUTE_REPARSE_POINT` is set.
    pub(crate) fn new(attributes: u32, tag: u32) -> Self {
        Self {
            attributes,
            tag: if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                tag
            } else {
                0
            },
        }
    }

    /// One `FileAttributeTagInfo` query on an already open (normally no-follow) handle,
    /// or, where a filesystem does not implement that class, one basic-attribute query.
    pub(crate) fn query(file: &File) -> io::Result<Self> {
        Self::query_with(|| tag_information(file), || attributes(file))
    }

    /// Classification policy of [`Self::query`] over injectable native queries.
    fn query_with(
        tagged: impl FnOnce() -> io::Result<Self>,
        basic: impl FnOnce() -> io::Result<u32>,
    ) -> io::Result<Self> {
        match tagged() {
            Err(error) if crate::mutation::native::unsupported(&error) => {
                Ok(Self::untagged(basic()?))
            }
            result => result,
        }
    }

    /// Attributes whose reparse tag cannot be read. Filesystems without the tag class
    /// (FAT32, exFAT) have no reparse points; should one still report the reparse
    /// attribute, it fails closed: it is a link, never traversed or read through.
    fn untagged(attributes: u32) -> Self {
        Self {
            attributes,
            tag: if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                NAME_SURROGATE
            } else {
                0
            },
        }
    }

    /// Classifies an open handle whose attributes were already read (for example from
    /// std metadata), querying the reparse tag only for reparse points.
    pub(crate) fn of(file: &File, attributes: u32) -> io::Result<Self> {
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
            Ok(Self::new(attributes, 0))
        } else {
            Self::query(file)
        }
    }

    pub(crate) fn directory(self) -> bool {
        self.attributes & FILE_ATTRIBUTE_DIRECTORY != 0
    }

    fn reparse(self) -> bool {
        self.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    /// A namespace-redirecting link (symbolic link, junction, mount point, ...).
    pub(crate) fn link(self) -> bool {
        self.reparse() && self.tag & NAME_SURROGATE != 0
    }

    /// An ordinary entry whose data a reparse filter supplies (cloud placeholder,
    /// deduplicated or WOF-compressed file, ...).
    pub(crate) fn filtered(self) -> bool {
        self.reparse() && !self.link()
    }

    pub(crate) fn kind(self) -> EntryKind {
        match (self.directory(), self.link()) {
            (true, true) => EntryKind::DirectoryLink,
            (false, true) => EntryKind::Symlink,
            (true, false) => EntryKind::Directory,
            (false, false) => EntryKind::File,
        }
    }
}

fn tag_information(file: &File) -> io::Result<Native> {
    let mut information = FILE_ATTRIBUTE_TAG_INFO::default();
    // SAFETY: file owns a live handle; the writable structure has its exact native size.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&mut information as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(Native::new(
        information.FileAttributes,
        information.ReparseTag,
    ))
}

/// The attributes of an open handle through `FileBasicInfo`, which every filesystem
/// (FAT32 and exFAT included) implements. One query; no reparse tag.
pub(crate) fn attributes(file: &File) -> io::Result<u32> {
    let mut information = FILE_BASIC_INFO::default();
    // SAFETY: file owns a live handle; the writable structure has its exact native size.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileBasicInfo,
            (&mut information as *mut FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(information.FileAttributes)
}

/// Reopens the exact object `pinned` refers to with new access, never by pathname.
///
/// `pinned` was opened without following its final reparse point and `native` is its
/// classification. A non-surrogate reparse *file* is reopened through the filter stack
/// (without `FILE_FLAG_OPEN_REPARSE_POINT`), so a cloud placeholder hydrates and
/// deduplicated or WOF-compressed data is returned instead of the raw stub. Every other
/// object, including every link, keeps no-follow semantics. Callers verify identity.
///
/// The mediated reopen acts on whatever reparse point the object carries *now*: should
/// another process replace a filtered file's reparse point with a symbolic link after
/// classification, the open follows that link before the caller's identity check
/// rejects the result. It therefore requests identification-level impersonation only,
/// so a named pipe reached that way cannot impersonate this process.
pub(crate) fn reopen_content(
    pinned: &File,
    native: Native,
    access: u32,
    share: u32,
) -> io::Result<File> {
    if mediated(native) {
        return crate::mutation::native::reopen_mediated(pinned, access, share);
    }
    // SAFETY: pinned is a live handle; the OS performs new access and sharing checks.
    let handle = unsafe { ReOpenFile(pinned.as_raw_handle(), access, share, no_follow(native)) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful ReOpenFile transfers one owned handle to the same object.
    Ok(unsafe { File::from_raw_handle(handle) })
}

/// Only a filtered file is reopened through its filter; links never are.
fn mediated(native: Native) -> bool {
    native.filtered() && !native.directory()
}

fn no_follow(native: Native) -> u32 {
    let directory = if native.directory() {
        FILE_FLAG_BACKUP_SEMANTICS
    } else {
        0
    };
    directory | FILE_FLAG_OPEN_REPARSE_POINT
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYMLINK: u32 = 0xA000_000C;
    const MOUNT_POINT: u32 = 0xA000_0003;
    const LX_SYMLINK: u32 = 0xA000_001D;
    const CLOUD_DIRECTORY: u32 = 0x9000_701A;
    const CLOUD_FILE: u32 = 0x9000_001A;
    const DEDUP: u32 = 0x8000_0013;
    const WOF: u32 = 0x8000_0017;
    const APPEXECLINK: u32 = 0x8000_001B;
    const DIRECTORY: u32 = FILE_ATTRIBUTE_DIRECTORY;
    const REPARSE: u32 = FILE_ATTRIBUTE_REPARSE_POINT;
    // ReadOnly | Archive | Compressed, as reported for a OneDrive Files On-Demand root.
    const ONEDRIVE_ROOT: u32 = 0x1 | DIRECTORY | 0x20 | REPARSE | 0x800;

    #[test]
    fn only_name_surrogate_tags_are_links() {
        for (attributes, tag, kind) in [
            (REPARSE, SYMLINK, EntryKind::Symlink),
            (DIRECTORY | REPARSE, SYMLINK, EntryKind::DirectoryLink),
            (DIRECTORY | REPARSE, MOUNT_POINT, EntryKind::DirectoryLink),
            (REPARSE, LX_SYMLINK, EntryKind::Symlink),
        ] {
            let native = Native::new(attributes, tag);
            assert!(native.link() && !native.filtered(), "{tag:#x}");
            assert_eq!(native.kind(), kind, "{tag:#x}");
        }
    }

    #[test]
    fn placeholder_dedup_and_wof_tags_are_ordinary_filtered_entries() {
        for (attributes, tag, kind) in [
            (ONEDRIVE_ROOT, CLOUD_DIRECTORY, EntryKind::Directory),
            (DIRECTORY | REPARSE, CLOUD_DIRECTORY, EntryKind::Directory),
            (0x20 | REPARSE, CLOUD_FILE, EntryKind::File),
            (REPARSE, DEDUP, EntryKind::File),
            (REPARSE | 0x800, WOF, EntryKind::File),
            (REPARSE, APPEXECLINK, EntryKind::File),
        ] {
            let native = Native::new(attributes, tag);
            assert!(!native.link() && native.filtered(), "{tag:#x}");
            assert_eq!(native.kind(), kind, "{tag:#x}");
        }
    }

    #[test]
    fn a_tag_without_the_reparse_attribute_is_ignored() {
        // Directory records reuse the tag field as EaSize for ordinary entries.
        for (attributes, kind) in [(0x20, EntryKind::File), (DIRECTORY, EntryKind::Directory)] {
            let native = Native::new(attributes, SYMLINK);
            assert_eq!(native.tag, 0);
            assert!(!native.link() && !native.filtered());
            assert_eq!(native.kind(), kind);
        }
    }

    /// FAT32/exFAT reject `FileAttributeTagInfo` (fastfat: STATUS_INVALID_PARAMETER).
    /// Before the fallback every classification failed there, so deletion, discovery,
    /// F7 and Copy probes could not even start on such a volume.
    #[test]
    fn an_unsupported_tag_class_falls_back_to_basic_attributes() {
        // ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED, ERROR_INVALID_PARAMETER (fastfat)
        // and an explicit unsupported kind.
        let unsupported: [fn() -> io::Error; 4] = [
            || io::Error::from_raw_os_error(1),
            || io::Error::from_raw_os_error(50),
            || io::Error::from_raw_os_error(87),
            || io::ErrorKind::Unsupported.into(),
        ];
        for error in unsupported {
            for (attributes, kind) in [
                (0x20 | 0x1, EntryKind::File),
                (DIRECTORY, EntryKind::Directory),
            ] {
                let native = Native::query_with(|| Err(error()), || Ok(attributes)).unwrap();
                assert_eq!(native.attributes, attributes, "{}", error());
                assert_eq!((native.tag, native.kind()), (0, kind), "{}", error());
                assert!(!native.link() && !native.filtered(), "{}", error());
            }
        }
    }

    #[test]
    fn an_unreadable_tag_fails_closed_as_a_link() {
        for (attributes, kind) in [
            (REPARSE, EntryKind::Symlink),
            (DIRECTORY | REPARSE, EntryKind::DirectoryLink),
        ] {
            let native =
                Native::query_with(|| Err(io::Error::from_raw_os_error(87)), || Ok(attributes))
                    .unwrap();
            assert!(native.link() && !native.filtered());
            assert_eq!(native.kind(), kind);
        }
    }

    #[test]
    fn other_tag_query_errors_and_a_failed_basic_query_are_reported() {
        let mut basic_calls = 0;
        let denied = Native::query_with(
            || Err(io::Error::from_raw_os_error(5)),
            || {
                basic_calls += 1;
                Ok(0x20)
            },
        )
        .unwrap_err();
        assert_eq!(denied.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(basic_calls, 0, "only an unsupported class falls back");
        let failed = Native::query_with(
            || Err(io::Error::from_raw_os_error(87)),
            || Err(io::Error::from_raw_os_error(5)),
        )
        .unwrap_err();
        assert_eq!(failed.kind(), io::ErrorKind::PermissionDenied);
        let tagged = Native::new(DIRECTORY | REPARSE, CLOUD_DIRECTORY);
        let native = Native::query_with(|| Ok(tagged), || panic!("tag class is supported"));
        assert_eq!(native.unwrap(), tagged);
    }

    #[test]
    fn real_handles_agree_on_attributes_and_classification() {
        let temp = crate::test_fixtures::build_temp("reparse-attributes-");
        let file = temp.path().join("file");
        std::fs::write(&file, b"x").unwrap();
        for (path, kind) in [
            (temp.path(), EntryKind::Directory),
            (file.as_path(), EntryKind::File),
        ] {
            use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
            let handle = std::fs::OpenOptions::new()
                .access_mode(windows_sys::Win32::Storage::FileSystem::FILE_READ_ATTRIBUTES)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(path)
                .unwrap();
            let expected = handle.metadata().unwrap().file_attributes();
            assert_eq!(attributes(&handle).unwrap(), expected);
            let native = Native::query(&handle).unwrap();
            assert_eq!((native.attributes, native.kind()), (expected, kind));
        }
    }

    /// Only filtered files take the filter-mediated reopen (with its identification-level
    /// quality of service, see `mutation::native::reopen_mediated`); every other object,
    /// links and filtered directories included, is reopened without following.
    #[test]
    fn only_filtered_files_reopen_through_their_filter() {
        for native in [
            Native::new(REPARSE, DEDUP),
            Native::new(0x20 | REPARSE, CLOUD_FILE),
            Native::new(REPARSE | 0x800, WOF),
        ] {
            assert!(mediated(native), "{native:?}");
        }
        for (native, flags) in [
            (Native::new(0x20, 0), FILE_FLAG_OPEN_REPARSE_POINT),
            (Native::new(REPARSE, SYMLINK), FILE_FLAG_OPEN_REPARSE_POINT),
            (
                Native::new(REPARSE, LX_SYMLINK),
                FILE_FLAG_OPEN_REPARSE_POINT,
            ),
            (
                Native::new(DIRECTORY | REPARSE, CLOUD_DIRECTORY),
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            ),
            (
                Native::new(DIRECTORY | REPARSE, MOUNT_POINT),
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            ),
            // A reparse point whose tag could not be read fails closed as a link.
            (Native::untagged(REPARSE), FILE_FLAG_OPEN_REPARSE_POINT),
        ] {
            assert!(!mediated(native), "{native:?}");
            assert_eq!(no_follow(native), flags, "{native:?}");
        }
    }
}
