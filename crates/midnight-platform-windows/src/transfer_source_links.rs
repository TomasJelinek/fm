//! Read-only proof for materializing a Windows directory link as an ordinary directory.
use super::*;
use midnight_core::TransferLinkRepresentation;
use std::{
    ffi::{OsStr, OsString},
    os::windows::io::AsRawHandle,
    path::PathBuf,
    ptr,
};
use windows_sys::{
    Wdk::Storage::FileSystem::{FILE_DIRECTORY_FILE, FILE_OPEN},
    Win32::System::{IO::DeviceIoControl, Ioctl::FSCTL_GET_REPARSE_POINT},
};

const MOUNT_POINT: u32 = 0xA000_0003;
const SYMLINK: u32 = 0xA000_000C;
const REPARSE_BUFFER_BYTES: usize = 16 * 1024;

#[path = "transfer_source_link_target.rs"]
mod target;
pub(super) use target::MoveParent;

pub(super) struct Binding {
    parent: Arc<File>,
    name: OsString,
    path: PathBuf,
    original: Arc<File>,
    proof: Arc<OriginalLink>,
    target: Option<(Arc<File>, TransferIdentity)>,
    _namespace_owner: super::namespace::NativeOwner,
}

/// Data observed through the original no-follow handle, independent of any target.
pub(super) struct OriginalLink {
    pub(super) entry: TransferEntry,
    revision: (u64, u64),
    representation: TransferLinkRepresentation,
}

impl OriginalLink {
    pub(super) fn capture(
        file: &File,
        path: &Path,
        expected: Native,
        expected_identity: Option<TransferIdentity>,
    ) -> Result<Self, FsError> {
        let (entry, native, metadata) = classified_with_metadata(file, path)?;
        if expected_identity.is_some_and(|id| id != entry.identity)
            || native.tag != expected.tag
            || native.kind() != EntryKind::DirectoryLink
        {
            return Err(invalid(path, "directory link changed while opening"));
        }
        Ok(Self {
            entry,
            revision: (metadata.last_write_time(), metadata.creation_time()),
            representation: read_representation(file, native.tag, path)?,
        })
    }

    pub(super) fn validate_binding(&self, file: &File, path: &Path) -> Result<(), FsError> {
        let native = Native::query(file).map_err(|e| error(path, e))?;
        if identity(file, path)? != self.entry.identity
            || native.kind() != EntryKind::DirectoryLink
            || read_representation(file, native.tag, path)? != self.representation
        {
            return Err(invalid(path, "directory link changed"));
        }
        Ok(())
    }

    pub(super) fn validate_for_move(&self, file: &File) -> Result<(), FsError> {
        let path = &self.entry.item.path;
        self.validate_binding(file, path)?;
        let metadata = file.metadata().map_err(|e| error(path, e))?;
        if (metadata.last_write_time(), metadata.creation_time()) != self.revision
            || Some(metadata.file_attributes()) != self.entry.item.metadata.native_attributes
        {
            return Err(invalid(path, "original directory link revision changed"));
        }
        Ok(())
    }
}

impl Binding {
    pub(super) fn capture(
        parent: Arc<File>,
        name: &OsStr,
        path: &Path,
        expected: Native,
        expected_identity: Option<TransferIdentity>,
        follow: bool,
        cancel: &Cancellation,
    ) -> Result<(Arc<File>, Arc<Self>), FsError> {
        let original = Arc::new(open_link(&parent, name).map_err(|e| error(path, e))?);
        let proof = Arc::new(OriginalLink::capture(
            &original,
            path,
            expected,
            expected_identity,
        )?);
        let original_identity = proof.entry.identity;
        let target = if follow {
            checked(cancel, path)?;
            let resolved = target::resolve(&parent, &proof.representation, path, cancel)?;
            let target_identity = identity(&resolved, path)?;
            let pinned = native::reopen(
                &resolved,
                FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ,
            )
            .map_err(|e| error(path, e))?;
            target::ordinary_directory(&pinned, path)?;
            if identity(&pinned, path)? != target_identity {
                return Err(invalid(path, "directory link target changed while opening"));
            }
            Some((Arc::new(pinned), target_identity))
        } else {
            None
        };
        let file = match &target {
            Some((file, _)) => file.clone(),
            None => original.clone(),
        };
        let binding = Arc::new(Self {
            parent,
            name: name.to_owned(),
            path: path.to_owned(),
            original,
            proof,
            target,
            _namespace_owner: super::namespace::owner(&[original_identity]),
        });
        binding.revalidate(cancel)?;
        Ok((file, binding))
    }

    pub(super) fn is_empty(&self) -> bool {
        self.target.is_none()
    }
    pub(super) fn original_identity(&self) -> TransferIdentity {
        self.proof.entry.identity
    }
    pub(super) fn original_link(&self) -> Arc<OriginalLink> {
        self.proof.clone()
    }

    pub(super) fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.path)?;
        self.validate_original()?;
        if let Some((_, target_identity)) = &self.target {
            let current =
                target::resolve(&self.parent, &self.proof.representation, &self.path, cancel)?;
            if identity(&current, &self.path)? != *target_identity {
                return Err(invalid(&self.path, "directory link target changed"));
            }
            // Keep the resolved target bracketed by original namespace and payload checks.
            self.validate_original()?;
        }
        Ok(())
    }

    fn validate_original(&self) -> Result<(), FsError> {
        let current = open_link(&self.parent, &self.name).map_err(|e| error(&self.path, e))?;
        // Retain the no-follow object so a deleted ID cannot be reused under this proof.
        let _original = &self.original;
        self.proof.validate_binding(&current, &self.path)
    }
}

fn open_link(parent: &File, name: &OsStr) -> io::Result<File> {
    native::open_relative(
        parent,
        name,
        FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        FILE_OPEN,
        FILE_DIRECTORY_FILE,
    )
}

fn read_representation(
    file: &File,
    tag: u32,
    path: &Path,
) -> Result<TransferLinkRepresentation, FsError> {
    if !matches!(tag, SYMLINK | MOUNT_POINT) {
        return Err(unsupported(path, "unsupported directory reparse tag"));
    }
    let mut buffer = vec![0u8; REPARSE_BUFFER_BYTES];
    let mut used = 0;
    // SAFETY: file is a live synchronous no-follow handle and the complete writable
    // buffer and length output remain valid throughout this no-input query.
    if unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_GET_REPARSE_POINT,
            ptr::null(),
            0,
            buffer.as_mut_ptr().cast(),
            REPARSE_BUFFER_BYTES as u32,
            &mut used,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(error(path, io::Error::last_os_error()));
    }
    let invalid_payload = || invalid(path, "invalid native directory link payload");
    let buffer = buffer.get(..used as usize).ok_or_else(invalid_payload)?;
    if buffer.len() < 8 || small_u32(buffer, 0) != tag {
        return Err(invalid_payload());
    }
    let body = buffer
        .get(8..8 + small_u16(buffer, 4))
        .ok_or_else(invalid_payload)?;
    let names_start = if tag == SYMLINK { 12 } else { 8 };
    let names = body.get(names_start..).ok_or_else(invalid_payload)?;
    let substitute =
        read_name(names, small_u16(body, 0), small_u16(body, 2)).ok_or_else(invalid_payload)?;
    let print =
        read_name(names, small_u16(body, 4), small_u16(body, 6)).ok_or_else(invalid_payload)?;
    let flags = if tag == SYMLINK {
        small_u32(body, 8)
    } else {
        0
    };
    if !known_local_name(&substitute, flags) {
        return Err(unsupported(
            path,
            "directory link has a remote, device, or volume mount target",
        ));
    }
    let representation = if tag == SYMLINK {
        TransferLinkRepresentation::windows_symbolic_link(true, flags, &substitute, &print)
    } else {
        TransferLinkRepresentation::windows_junction(&substitute, &print)
    };
    representation.map_err(|_| invalid_payload())
}

fn known_local_name(name: &[u16], flags: u32) -> bool {
    const SEPARATOR: u16 = b'\\' as u16;
    const COLON: u16 = b':' as u16;
    match flags {
        0 => matches!(
            name,
            [SEPARATOR, 63, 63, SEPARATOR, drive, COLON, SEPARATOR, rest @ ..]
                if u8::try_from(*drive).is_ok_and(|letter| letter.is_ascii_alphabetic())
                    && !rest.contains(&COLON)
        ),
        1 => !name.is_empty() && !matches!(name[0], SEPARATOR | 47) && !name.contains(&COLON),
        _ => false,
    }
}

fn read_name(buffer: &[u8], offset: usize, count: usize) -> Option<Vec<u16>> {
    if !offset.is_multiple_of(2) || !count.is_multiple_of(2) {
        return None;
    }
    let bytes = buffer.get(offset..offset.checked_add(count)?)?;
    Some(
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|unit| u16::from_le_bytes(*unit))
            .collect(),
    )
}

fn small_u16(bytes: &[u8], offset: usize) -> usize {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize
}

fn small_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}
