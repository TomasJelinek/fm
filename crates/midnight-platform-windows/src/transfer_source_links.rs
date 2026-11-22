//! Read-only proofs of Windows links: a directory link materialized as an ordinary
//! directory, a symbolic link to a file followed to that file, and the exact
//! representation of a link that is kept as a link.
use super::*;
use midnight_core::TransferLinkRepresentation;
use std::{
    ffi::{OsStr, OsString},
    os::windows::io::AsRawHandle,
    path::PathBuf,
    ptr,
};
use windows_sys::{
    Wdk::Storage::FileSystem::{FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN},
    Win32::System::{IO::DeviceIoControl, Ioctl::FSCTL_GET_REPARSE_POINT},
};

pub(super) const MOUNT_POINT: u32 = 0xA000_0003;
pub(super) const SYMLINK: u32 = 0xA000_000C;
const REPARSE_BUFFER_BYTES: usize = 16 * 1024;

#[path = "transfer_source_link_target.rs"]
mod target;
pub(super) use target::MoveParent;

/// Which link targets a proof accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Targets {
    /// A path on a local drive, absolute or relative: what a followed link, and the
    /// original of a materialized directory link, may point to.
    Local,
    /// A link kept as a link, which is never opened through: any symbolic link to a local
    /// drive, a relative path or a network share, and any junction to a local drive.
    Any,
}

pub(super) struct Binding {
    parent: Arc<File>,
    name: OsString,
    path: PathBuf,
    original: Arc<File>,
    proof: Arc<OriginalLink>,
    target: Option<(Arc<File>, TransferIdentity)>,
    /// A symbolic link to a file, followed to that file.
    file_target: bool,
    _namespace_owner: super::namespace::NativeOwner,
}

/// Data observed through the original no-follow handle, independent of any target.
pub(super) struct OriginalLink {
    pub(super) entry: TransferEntry,
    revision: (u64, u64),
    representation: TransferLinkRepresentation,
    targets: Targets,
}

impl OriginalLink {
    pub(super) fn capture(
        file: &File,
        path: &Path,
        expected: Native,
        expected_identity: Option<TransferIdentity>,
        targets: Targets,
    ) -> Result<Self, FsError> {
        let (entry, native, metadata) = classified_with_metadata(file, path)?;
        if expected_identity.is_some_and(|id| id != entry.identity)
            || native.tag != expected.tag
            || native.kind() != expected.kind()
            || !matches!(native.kind(), EntryKind::DirectoryLink | EntryKind::Symlink)
        {
            return Err(invalid(path, "directory link changed while opening"));
        }
        Ok(Self {
            entry,
            revision: (metadata.last_write_time(), metadata.creation_time()),
            representation: read_representation(file, native, path, targets)?,
            targets,
        })
    }

    /// The link's exact representation, as it was captured.
    pub(super) fn representation(&self) -> &TransferLinkRepresentation {
        &self.representation
    }

    /// A link to a folder (a junction or a directory symbolic link).
    pub(super) fn directory(&self) -> bool {
        self.entry.item.kind == EntryKind::DirectoryLink
    }

    pub(super) fn validate_binding(&self, file: &File, path: &Path) -> Result<(), FsError> {
        let native = Native::query(file).map_err(|e| error(path, e))?;
        if identity(file, path)? != self.entry.identity
            || native.kind() != self.entry.item.kind
            || read_representation(file, native, path, self.targets)? != self.representation
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
        let original = Arc::new(open_link(&parent, name, true).map_err(|e| error(path, e))?);
        let proof = Arc::new(OriginalLink::capture(
            &original,
            path,
            expected,
            expected_identity,
            Targets::Local,
        )?);
        let target = if follow {
            checked(cancel, path)?;
            let resolved = target::resolve(&parent, &proof.representation, path, cancel)?;
            let target_identity = identity(&resolved, path)?;
            // Held like any folder of the source: its name is kept and other jobs, which
            // may write there (it can lie outside every claim), still publish into it.
            let pinned = native::reopen(
                &resolved,
                FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
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
        Self::bind(parent, name, path, original, proof, target, false, cancel)
    }

    /// A symbolic link to a file, followed to that file (FollowLinks): the file is held
    /// for reading, as a copied file is, while the link and its target are bound.
    pub(super) fn capture_file(
        parent: Arc<File>,
        name: &OsStr,
        path: &Path,
        expected: Native,
        expected_identity: Option<TransferIdentity>,
        cancel: &Cancellation,
    ) -> Result<(Arc<File>, Arc<Self>), FsError> {
        let original = Arc::new(open_link(&parent, name, false).map_err(|e| error(path, e))?);
        let proof = Arc::new(OriginalLink::capture(
            &original,
            path,
            expected,
            expected_identity,
            Targets::Local,
        )?);
        checked(cancel, path)?;
        let resolved = target::resolve_file(&parent, &proof.representation, path, cancel)?;
        let target_identity = identity(&resolved, path)?;
        // As a copied file is held: its content and name cannot change while it is read.
        let held = native::reopen(
            &resolved,
            FILE_READ_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ,
        )
        .map_err(|e| error(path, e))?;
        if identity(&held, path)? != target_identity {
            return Err(invalid(path, "file link target changed while opening"));
        }
        let target = Some((Arc::new(held), target_identity));
        Self::bind(parent, name, path, original, proof, target, true, cancel)
    }

    #[allow(clippy::too_many_arguments)]
    fn bind(
        parent: Arc<File>,
        name: &OsStr,
        path: &Path,
        original: Arc<File>,
        proof: Arc<OriginalLink>,
        target: Option<(Arc<File>, TransferIdentity)>,
        file_target: bool,
        cancel: &Cancellation,
    ) -> Result<(Arc<File>, Arc<Self>), FsError> {
        let original_identity = proof.entry.identity;
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
            file_target,
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
            let current = if self.file_target {
                target::resolve_file(&self.parent, &self.proof.representation, &self.path, cancel)?
            } else {
                target::resolve(&self.parent, &self.proof.representation, &self.path, cancel)?
            };
            if identity(&current, &self.path)? != *target_identity {
                return Err(invalid(&self.path, "directory link target changed"));
            }
            // Keep the resolved target bracketed by original namespace and payload checks.
            self.validate_original()?;
        }
        Ok(())
    }

    fn validate_original(&self) -> Result<(), FsError> {
        let current = open_link(&self.parent, &self.name, self.proof.directory())
            .map_err(|e| error(&self.path, e))?;
        // Retain the no-follow object so a deleted ID cannot be reused under this proof.
        let _original = &self.original;
        self.proof.validate_binding(&current, &self.path)
    }
}

/// The link `name` of `parent`, not followed, with attribute access only; `directory`
/// for a link to a folder.
pub(super) fn open_link(parent: &File, name: &OsStr, directory: bool) -> io::Result<File> {
    native::open_relative(
        parent,
        name,
        FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        FILE_OPEN,
        if directory {
            FILE_DIRECTORY_FILE
        } else {
            FILE_NON_DIRECTORY_FILE
        },
    )
}

/// The exact representation of the open no-follow link `file` (a symbolic link or a
/// local junction), for a link kept as a link.
pub(super) fn read_link(file: &File, path: &Path) -> Result<TransferLinkRepresentation, FsError> {
    let native = Native::query(file).map_err(|e| error(path, e))?;
    read_representation(file, native, path, Targets::Any)
}

fn read_representation(
    file: &File,
    native: Native,
    path: &Path,
    targets: Targets,
) -> Result<TransferLinkRepresentation, FsError> {
    let tag = native.tag;
    if !native.link() || !matches!(tag, SYMLINK | MOUNT_POINT) {
        return Err(unsupported(
            path,
            "this kind of link is not copied: only symbolic links and junctions are",
        ));
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
    if body.len() < names_start {
        return Err(invalid_payload());
    }
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
    let local = known_local_name(&substitute, flags);
    if tag == MOUNT_POINT && !native.directory() {
        return Err(invalid_payload());
    }
    if tag == MOUNT_POINT && !local {
        // A volume mounted in a folder (`\??\Volume{...}\`) is not entered or recreated.
        return Err(unsupported(
            path,
            "it is a volume mounted in a folder, which is not copied",
        ));
    }
    if !local && (targets == Targets::Local || !network_name(&substitute, flags)) {
        return Err(unsupported(
            path,
            "directory link has a remote, device, or volume mount target",
        ));
    }
    let representation = if tag == SYMLINK {
        TransferLinkRepresentation::windows_symbolic_link(
            native.directory(),
            flags,
            &substitute,
            &print,
        )
    } else {
        TransferLinkRepresentation::windows_junction(&substitute, &print)
    };
    representation.map_err(|_| invalid_payload())
}

const SEPARATOR: u16 = b'\\' as u16;
const COLON: u16 = b':' as u16;

/// A local drive's absolute path (`\??\X:\...`, flags 0) or a relative path (flags 1).
pub(super) fn known_local_name(name: &[u16], flags: u32) -> bool {
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

/// A network share's absolute path, `\??\UNC\server\share\...` (flags 0).
fn network_name(name: &[u16], flags: u32) -> bool {
    const PREFIX: [u16; 8] = [
        SEPARATOR,
        63,
        63,
        SEPARATOR,
        b'U' as u16,
        b'N' as u16,
        b'C' as u16,
        SEPARATOR,
    ];
    flags == 0
        && name.len() > PREFIX.len()
        && name[..PREFIX.len()] == PREFIX
        && !name[8..].contains(&COLON)
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
