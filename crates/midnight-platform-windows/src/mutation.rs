//! Identity-pinned Windows mutations. Only worker threads call this module.
use crate::WindowsFileSystem;
use midnight_core::{
    Cancellation, CompressionStatus, DeleteMode, EntryKind, EntryMetadata, FsError, MutationItem,
    MutationService, PreparedDirectory, PreparedMutation,
};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io,
    os::windows::{ffi::OsStrExt, fs::MetadataExt},
    path::{Component, Path, PathBuf, Prefix},
    sync::Arc,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
};

#[path = "mutation_create.rs"]
mod create;
#[path = "mutation_native.rs"]
mod native;
#[path = "mutation_trash.rs"]
mod trash;

struct WindowsMutation {
    item: MutationItem,
    parent: File,
    source: File,
    name: OsString,
    identity: native::Identity,
}

impl MutationService for WindowsFileSystem {
    fn prepare_create(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn PreparedDirectory>, FsError> {
        create::prepare(path, cancel)
    }
    fn prepare(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn PreparedMutation>, FsError> {
        checked(cancel, path)?;
        validate_path(path, false)?;
        let parent_path = path
            .parent()
            .ok_or_else(|| invalid(path, "a parent directory is required"))?;
        let name = path
            .file_name()
            .ok_or_else(|| invalid(path, "a basename is required"))?
            .to_owned();
        let parent =
            native::open_parent(parent_path).map_err(|e| failure("open parent", path, e))?;
        let source = native::open_child(&parent, &name, false)
            .map_err(|e| failure("prepare mutation", path, e))?;
        let identity = native::identity(&source).map_err(|e| failure("read identity", path, e))?;
        let metadata = source
            .metadata()
            .map_err(|e| failure("read metadata", path, e))?;
        let attributes = metadata.file_attributes();
        let directory = attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
        let reparse = attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
        let kind = match (directory, reparse) {
            (true, true) => EntryKind::DirectoryLink,
            (false, true) => EntryKind::Symlink,
            (true, false) => EntryKind::Directory,
            (false, false) => EntryKind::File,
        };
        checked(cancel, path)?;
        Ok(Arc::new(WindowsMutation {
            item: MutationItem {
                path: path.to_owned(),
                kind,
                metadata: EntryMetadata {
                    size: (!directory).then_some(metadata.len()),
                    modified: metadata.modified().ok(),
                    localized_modified: metadata.modified().ok().and_then(crate::locale::modified),
                    compression: if attributes & FILE_ATTRIBUTE_COMPRESSED != 0 {
                        CompressionStatus::Compressed
                    } else if reparse {
                        CompressionStatus::Unknown
                    } else {
                        CompressionStatus::Uncompressed
                    },
                    native_attributes: Some(attributes),
                    unix_mode: None,
                },
            },
            parent,
            source,
            name,
            identity,
        }))
    }
}

impl WindowsMutation {
    fn revalidate(&self, exclusive: bool) -> Result<File, FsError> {
        let path = &self.item.path;
        // Reopening the visible parent catches replacement before acquiring the relative source.
        // Subsequent operations use retained handles even if ancestors are concurrently renamed.
        let parent_path = path
            .parent()
            .ok_or_else(|| invalid(path, "missing parent"))?;
        let visible_parent =
            native::open_parent(parent_path).map_err(|e| failure("revalidate parent", path, e))?;
        if native::identity(&visible_parent)
            .map_err(|e| failure("read parent identity", path, e))?
            != native::identity(&self.parent)
                .map_err(|e| failure("read parent identity", path, e))?
        {
            return Err(changed(path));
        }
        let source = native::open_child(&self.parent, &self.name, exclusive)
            .map_err(|e| failure("revalidate source", path, e))?;
        if native::identity(&source).map_err(|e| failure("read identity", path, e))?
            != self.identity
        {
            return Err(changed(path));
        }
        // Keep the original handle alive across confirmation so file IDs cannot be recycled.
        let _retained = &self.source;
        Ok(source)
    }
}

impl PreparedMutation for WindowsMutation {
    fn item(&self) -> &MutationItem {
        &self.item
    }

    fn rename(&self, new_name: &OsStr, cancel: &Cancellation) -> Result<PathBuf, FsError> {
        let path = &self.item.path;
        checked(cancel, path)?;
        validate_name(new_name).map_err(|e| failure("rename", path, e))?;
        let source = self.revalidate(true)?;
        checked(cancel, path)?;
        native::rename(&source, new_name).map_err(|e| failure("rename", path, e))?;
        Ok(path.with_file_name(new_name))
    }

    fn delete(
        &self,
        mode: DeleteMode,
        cancel: &Cancellation,
        progress: &mut dyn FnMut(&Path),
    ) -> Result<(), FsError> {
        checked(cancel, &self.item.path)?;
        if mode == DeleteMode::Trash {
            return trash::recycle(self, cancel, progress);
        }
        let source = self.revalidate(true)?;
        native::delete_tree(
            source,
            &self.parent,
            self.item.path.clone(),
            cancel,
            progress,
        )
    }
}

fn validate_path(path: &Path, allow_root: bool) -> Result<(), FsError> {
    reject_dot_components(path)?;
    if !path.is_absolute() || (!allow_root && path.file_name().is_none()) {
        return Err(invalid(
            path,
            "an absolute non-root filesystem path is required",
        ));
    }
    for component in path.components() {
        match component {
            Component::Prefix(prefix)
                if !matches!(
                    prefix.kind(),
                    Prefix::Disk(_)
                        | Prefix::VerbatimDisk(_)
                        | Prefix::UNC(_, _)
                        | Prefix::VerbatimUNC(_, _)
                ) =>
            {
                return Err(invalid(
                    path,
                    "device and special namespaces are not accepted",
                ));
            }
            Component::ParentDir | Component::CurDir => {
                return Err(invalid(path, "dot path components are not accepted"));
            }
            Component::Normal(name) => {
                validate_name(name).map_err(|e| failure("prepare mutation", path, e))?
            }
            _ => {}
        }
    }
    Ok(())
}

fn reject_dot_components(path: &Path) -> Result<(), FsError> {
    let units: Vec<_> = path.as_os_str().encode_wide().collect();
    if units
        .split(|u| *u == b'/' as u16 || *u == b'\\' as u16)
        .any(|part| part == [b'.' as u16] || part == [b'.' as u16, b'.' as u16])
    {
        return Err(invalid(path, "dot path components are not accepted"));
    }
    Ok(())
}

fn validate_name(name: &OsStr) -> io::Result<()> {
    let units: Vec<_> = name.encode_wide().collect();
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "a valid Windows basename is required; paths, streams and reserved device names are not accepted",
        )
    };
    if units.is_empty()
        || units
            .last()
            .is_some_and(|u| *u == b'.' as u16 || *u == b' ' as u16)
        || units
            .iter()
            .any(|u| *u < 32 || b"<>:\"/\\|?*".iter().any(|c| *u == *c as u16))
    {
        return Err(invalid());
    }
    let stem: Vec<_> = units
        .iter()
        .copied()
        .take_while(|u| *u != b'.' as u16)
        .map(|u| {
            if (b'a' as u16..=b'z' as u16).contains(&u) {
                u - 32
            } else {
                u
            }
        })
        .collect();
    let is = |text: &str| stem.iter().copied().eq(text.encode_utf16());
    if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]
        .iter()
        .any(|s| is(s))
        || (stem.len() == 4
            && (stem[..3] == [67, 79, 77] || stem[..3] == [76, 80, 84])
            && ((49..=57).contains(&stem[3]) || [0x00b9, 0x00b2, 0x00b3].contains(&stem[3])))
    {
        return Err(invalid());
    }
    Ok(())
}

fn checked(cancel: &Cancellation, path: &Path) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(failure(
            "mutate",
            path,
            io::Error::new(
                io::ErrorKind::Interrupted,
                "operation cancelled; already processed entries are not restored",
            ),
        ))
    } else {
        Ok(())
    }
}
fn failure(operation: &'static str, path: &Path, error: io::Error) -> FsError {
    FsError::from_io(operation, path, error)
}
fn invalid(path: &Path, message: &str) -> FsError {
    failure(
        "validate mutation",
        path,
        io::Error::new(io::ErrorKind::InvalidInput, message),
    )
}
fn changed(path: &Path) -> FsError {
    failure(
        "revalidate mutation",
        path,
        io::Error::new(
            io::ErrorKind::NotFound,
            "the selected entry or its parent has been replaced",
        ),
    )
}
