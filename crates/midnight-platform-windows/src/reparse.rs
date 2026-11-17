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
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FileAttributeTagInfo,
        GetFileInformationByHandleEx, ReOpenFile,
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

    /// One `FileAttributeTagInfo` query on an already open (normally no-follow) handle.
    pub(crate) fn query(file: &File) -> io::Result<Self> {
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
        Ok(Self::new(
            information.FileAttributes,
            information.ReparseTag,
        ))
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

/// Reopens the exact object `pinned` refers to with new access, never by pathname.
///
/// `pinned` was opened without following its final reparse point and `native` is its
/// classification. A non-surrogate reparse *file* is reopened through the filter stack
/// (without `FILE_FLAG_OPEN_REPARSE_POINT`), so a cloud placeholder hydrates and
/// deduplicated or WOF-compressed data is returned instead of the raw stub. Every other
/// object, including every link, keeps no-follow semantics. Callers verify identity.
pub(crate) fn reopen_content(
    pinned: &File,
    native: Native,
    access: u32,
    share: u32,
) -> io::Result<File> {
    let mediated = native.filtered() && !native.directory();
    let flags = if native.directory() {
        FILE_FLAG_BACKUP_SEMANTICS
    } else {
        0
    } | if mediated {
        0
    } else {
        FILE_FLAG_OPEN_REPARSE_POINT
    };
    // SAFETY: pinned is a live handle; the OS performs new access and sharing checks.
    let handle = unsafe { ReOpenFile(pinned.as_raw_handle(), access, share, flags) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful ReOpenFile transfers one owned handle to the same object.
    Ok(unsafe { File::from_raw_handle(handle) })
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
}
