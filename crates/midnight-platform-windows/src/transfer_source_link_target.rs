//! Confined read-only lookup of a captured directory link's substitute name.
use super::*;
use std::{
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
    },
    path::{Component, Prefix},
};
use windows_sys::Win32::Storage::FileSystem::QueryDosDeviceW;

const SEPARATOR: u16 = b'\\' as u16;

pub(super) fn resolve(
    parent: &File,
    representation: &TransferLinkRepresentation,
    path: &Path,
    cancel: &Cancellation,
) -> Result<File, FsError> {
    let (mut walk, remaining) = start(parent, representation, path, cancel)?;
    for component in remaining.split(|unit| *unit == SEPARATOR) {
        checked(cancel, path)?;
        walk.step(component, path)?;
    }
    Ok(walk.current)
}

/// `resolve` for a symbolic link to a file: its folders are walked as `resolve` walks
/// them, and its last name is opened, without following it, as an ordinary regular file
/// on the same volume (no reparse point, no cloud or deduplicated file). The handle has
/// attribute access only.
pub(super) fn resolve_file(
    parent: &File,
    representation: &TransferLinkRepresentation,
    path: &Path,
    cancel: &Cancellation,
) -> Result<File, FsError> {
    let (mut walk, remaining) = start(parent, representation, path, cancel)?;
    let names: Vec<&[u16]> = remaining
        .split(|unit| *unit == SEPARATOR)
        .filter(|name| !matches!(name, [] | [46]))
        .collect();
    let Some((last, folders)) = names.split_last() else {
        return Err(unsupported(path, "file link target names no file"));
    };
    for component in folders {
        checked(cancel, path)?;
        walk.step(component, path)?;
    }
    if *last == [46, 46] {
        return Err(unsupported(path, "file link target names a folder"));
    }
    let name = OsString::from_wide(last);
    validate_name(&name).map_err(|e| error(path, e))?;
    let file = native::open_relative(
        &walk.current,
        &name,
        FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        FILE_OPEN,
        FILE_NON_DIRECTORY_FILE,
    )
    .map_err(|e| error(path, e))?;
    let native = Native::query(&file).map_err(|e| error(path, e))?;
    if native.kind() != EntryKind::File || native.filtered() {
        return Err(unsupported(
            path,
            "its target is not an ordinary file (a link, or a cloud or deduplicated file)",
        ));
    }
    if identity(&file, path)?.device != walk.volume {
        return Err(unsupported(
            path,
            "file link target crosses a volume boundary",
        ));
    }
    Ok(file)
}

/// Where a link's walk starts (its target's drive root, or its own folder for a relative
/// target) and the names left to walk.
fn start<'a>(
    parent: &File,
    representation: &'a TransferLinkRepresentation,
    path: &Path,
    cancel: &Cancellation,
) -> Result<(Walk, &'a [u16]), FsError> {
    checked(cancel, path)?;
    let (substitute, _) = representation
        .windows_names()
        .ok_or_else(|| unsupported(path, "directory link has no Windows target representation"))?;
    Ok(match (representation.meaningful_flags(), substitute) {
        (
            0,
            [
                SEPARATOR,
                63,
                63,
                SEPARATOR,
                drive,
                58,
                SEPARATOR,
                rest @ ..,
            ],
        ) if u8::try_from(*drive).is_ok_and(|letter| letter.is_ascii_alphabetic()) => {
            let drive_name = OsString::from_wide(&[*drive, 58]);
            (Walk::root(&drive_name, path)?, rest)
        }
        (1, relative) => (Walk::at_parent(parent, path, cancel)?, relative),
        _ => {
            return Err(unsupported(
                path,
                "directory link target has no qualified local root",
            ));
        }
    })
}

struct Walk {
    current: File,
    ancestors: Vec<File>,
    volume: u64,
}

pub(in crate::transfer) struct MoveParent {
    selector: OsString,
    device: String,
    root: File,
    components: Vec<OsString>,
    identity: TransferIdentity,
}

impl MoveParent {
    pub(in crate::transfer) fn capture(
        parent: &File,
        logical: &Path,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Self, FsError> {
        checked(cancel, path)?;
        let (selector, components) = move_parent_root(logical, path)?;
        let device = local_device_name(&selector, path)?;
        let root = open_local_root(&device, path)?;
        let components = components
            .map(|component| match component {
                Component::Normal(name) => Ok(name.to_owned()),
                _ => Err(invalid(
                    path,
                    "directory link Move parent has invalid ancestry",
                )),
            })
            .collect::<Result<_, _>>()?;
        let proof = Self {
            selector,
            device,
            root,
            components,
            identity: identity(parent, path)?,
        };
        proof.revalidate(path, cancel)?;
        Ok(proof)
    }

    pub(in crate::transfer) fn revalidate(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        checked(cancel, path)?;
        if local_device_name(&self.selector, path)? != self.device {
            return Err(invalid(
                path,
                "original Move directory link volume mapping changed",
            ));
        }
        let mut current = self
            .root
            .try_clone()
            .map_err(|failure| error(path, failure))?;
        for name in &self.components {
            checked(cancel, path)?;
            current = open_directory_child(&current, name, self.identity.device, path)?;
        }
        if identity(&current, path)? != self.identity {
            return Err(invalid(path, "original Move directory link parent changed"));
        }
        checked(cancel, path)
    }
}

fn move_parent_root<'a>(
    logical: &'a Path,
    path: &Path,
) -> Result<(OsString, std::path::Components<'a>), FsError> {
    let mut components = logical.components();
    if let Some(Component::Prefix(prefix)) = components.next() {
        match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive)
                if components.next() == Some(Component::RootDir) =>
            {
                return Ok((OsString::from_wide(&[u16::from(drive), 58]), components));
            }
            Prefix::Verbatim(_) => {
                return volume_root(logical, path)
                    .map(|(root, components)| (root.to_owned(), components));
            }
            _ => {}
        }
    }
    Err(unsupported(
        path,
        "directory link Move requires a qualified local parent path",
    ))
}

fn open_local_root(mapped: &str, path: &Path) -> Result<File, FsError> {
    // The mapping identifies a kernel disk volume. Enter that specific root
    // through GLOBALROOT before using any logical path components.
    let device = PathBuf::from(format!(r"\\?\GLOBALROOT{mapped}\"));
    let current = File::options()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_TRAVERSE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(device)
        .map_err(|e| error(path, e))?;
    ordinary_directory(&current, path)?;
    let physical = destination::final_path(&current, VOLUME_NAME_GUID)
        .map_err(|_| unsupported(path, "directory link requires a local volume"))?;
    let (_, mut components) = volume_root(&physical, path)?;
    if components.next().is_some() {
        return Err(unsupported(
            path,
            "directory link device is not a volume root",
        ));
    }
    Ok(current)
}

fn open_directory_child(
    parent: &File,
    name: &OsStr,
    volume: u64,
    path: &Path,
) -> Result<File, FsError> {
    validate_name(name).map_err(|e| error(path, e))?;
    // The held directory and single no-follow basename confine this lookup.
    let child = native::open_relative(
        parent,
        name,
        FILE_READ_ATTRIBUTES | FILE_TRAVERSE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        FILE_OPEN,
        FILE_DIRECTORY_FILE,
    )
    .map_err(|e| error(path, e))?;
    ordinary_directory(&child, path)?;
    if identity(&child, path)?.device != volume {
        return Err(unsupported(
            path,
            "directory link ancestry crosses a volume boundary",
        ));
    }
    Ok(child)
}

impl Walk {
    fn root(dos_device: &OsStr, path: &Path) -> Result<Self, FsError> {
        let mapped = local_device_name(dos_device, path)?;
        let current = open_local_root(&mapped, path)?;
        let volume = identity(&current, path)?.device;
        Ok(Self {
            current,
            ancestors: Vec::new(),
            volume,
        })
    }

    fn at_parent(parent: &File, path: &Path, cancel: &Cancellation) -> Result<Self, FsError> {
        let expected = identity(parent, path)?;
        let physical = destination::final_path(parent, VOLUME_NAME_GUID)
            .map_err(|_| unsupported(path, "relative directory link requires a local parent"))?;
        let (volume, components) = volume_root(&physical, path)?;
        let mut walk = Self::root(volume, path)?;
        for component in components {
            checked(cancel, path)?;
            let Component::Normal(name) = component else {
                return Err(invalid(
                    path,
                    "relative directory link parent has invalid ancestry",
                ));
            };
            walk.descend(name, path)?;
        }
        if identity(&walk.current, path)? != expected {
            return Err(invalid(
                path,
                "relative directory link parent identity changed",
            ));
        }
        Ok(walk)
    }

    /// One name of a link's text: `.` stays, `..` goes up (never above the volume's root),
    /// any other name goes down into an ordinary folder.
    fn step(&mut self, component: &[u16], path: &Path) -> Result<(), FsError> {
        match component {
            [] | [46] => Ok(()),
            [46, 46] => {
                self.current = self.ancestors.pop().ok_or_else(|| {
                    unsupported(path, "directory link target goes above its local volume")
                })?;
                Ok(())
            }
            name => self.descend(&OsString::from_wide(name), path),
        }
    }

    fn descend(&mut self, name: &OsStr, path: &Path) -> Result<(), FsError> {
        let child = open_directory_child(&self.current, name, self.volume, path)?;
        self.ancestors
            .push(std::mem::replace(&mut self.current, child));
        Ok(())
    }
}

pub(super) fn ordinary_directory(file: &File, path: &Path) -> Result<(), FsError> {
    let native = Native::query(file).map_err(|e| error(path, e))?;
    if !is_plain_directory(native) {
        return Err(unsupported(
            path,
            "directory link target ancestry requires ordinary directories without reparse points",
        ));
    }
    Ok(())
}

fn is_plain_directory(native: Native) -> bool {
    native.kind() == EntryKind::Directory && !native.filtered()
}

fn volume_root<'a>(
    physical: &'a Path,
    path: &Path,
) -> Result<(&'a OsStr, std::path::Components<'a>), FsError> {
    let mut components = physical.components();
    if let Some(Component::Prefix(prefix)) = components.next()
        && let Prefix::Verbatim(volume) = prefix.kind()
        && volume.as_encoded_bytes().starts_with(b"Volume{")
        && volume.as_encoded_bytes().ends_with(b"}")
        && components.next() == Some(Component::RootDir)
    {
        return Ok((volume, components));
    }
    Err(unsupported(
        path,
        "directory link has no local volume GUID root",
    ))
}

fn local_device_name(dos_device: &OsStr, path: &Path) -> Result<String, FsError> {
    let name: Vec<_> = dos_device.encode_wide().chain(Some(0)).collect();
    let mut output = [0u16; 1024];
    // SAFETY: name is terminated and the UTF-16 output buffer remains writable for
    // this query of the object-manager namespace. The first result is the active mapping.
    let count = unsafe { QueryDosDeviceW(name.as_ptr(), output.as_mut_ptr(), output.len() as u32) };
    let refused = || {
        unsupported(
            path,
            "directory link root is not a direct local disk mapping",
        )
    };
    if count == 0 {
        return Err(refused());
    }
    let output = output.get(..count as usize).ok_or_else(refused)?;
    let end = output
        .iter()
        .position(|unit| *unit == 0)
        .ok_or_else(refused)?;
    let mapped = String::from_utf16(&output[..end]).map_err(|_| refused())?;
    if !mapped
        .strip_prefix(r"\Device\HarddiskVolume")
        .is_some_and(|suffix| {
            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return Err(refused());
    }
    Ok(mapped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_link_target_proof_rejects_filter_managed_directories() {
        const CLOUD_TAG: u32 = 0x9000_001A;
        assert!(is_plain_directory(Native::new(FILE_ATTRIBUTE_DIRECTORY, 0)));
        let cloud = Native::new(
            FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT,
            CLOUD_TAG,
        );
        assert_eq!(cloud.kind(), EntryKind::Directory);
        assert!(!is_plain_directory(cloud));
    }
}
