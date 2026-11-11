//! Handle-only metadata changes; retain the same source through rename and writes.
use super::*;
use midnight_core::*;
use midnight_platform_common::metadata::validate_patch;
use std::{
    mem::size_of,
    os::windows::{
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle},
    },
    time::UNIX_EPOCH,
};
use windows_sys::{
    Wdk::Storage::FileSystem::FILE_OPEN,
    Win32::{
        Foundation::{FILETIME, INVALID_HANDLE_VALUE},
        Globalization::{CSTR_EQUAL, CompareStringOrdinal},
        Storage::FileSystem::*,
        System::{IO::DeviceIoControl, Ioctl::FSCTL_SET_COMPRESSION},
    },
};

pub(super) fn name_available(
    item: &WindowsMutation,
    name: &OsStr,
    cancel: &Cancellation,
) -> Result<(), FsError> {
    let path = &item.item.path;
    checked(cancel, path)?;
    validate_name(name).map_err(|e| failure("validate case name", path, e))?;
    if name == item.name {
        return Ok(());
    }
    let collision = || {
        failure(
            "check name collision",
            &path.with_file_name(name),
            io::ErrorKind::AlreadyExists.into(),
        )
    };
    let existing = match native::open_child(&item.parent, name, false) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(failure("check name collision", path, error)),
    };
    if native::identity(&existing).map_err(|e| failure("identify name collision", path, e))?
        != item.identity
        || !same_name_ignoring_case(name, &item.name)
            .map_err(|e| failure("compare case alias", path, e))?
    {
        return Err(collision());
    }
    let mut info = FILE_STANDARD_INFO::default();
    // SAFETY: fixed-size writable output and live owned handle.
    if unsafe {
        GetFileInformationByHandleEx(
            existing.as_raw_handle(),
            FileStandardInfo,
            (&mut info as *mut FILE_STANDARD_INFO).cast(),
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    } == 0
    {
        return Err(failure(
            "inspect hard links",
            path,
            io::Error::last_os_error(),
        ));
    }
    if info.NumberOfLinks <= 1 {
        return Ok(());
    }
    // Same FileId does not identify a directory entry when hard links exist. Inspect exact
    // native spelling without materializing a directory-sized list. Final rename remains no-replace.
    let directory = std::fs::OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(
            path.parent()
                .ok_or_else(|| invalid(path, "missing parent"))?,
        )
        .map_err(|e| failure("inspect case alias", path, e))?;
    if native::identity(&directory).map_err(|e| failure("identify case-alias parent", path, e))?
        != native::identity(&item.parent).map_err(|e| failure("identify pinned parent", path, e))?
    {
        return Err(changed(path));
    }
    if native::has_exact_child_name(&directory, name, cancel)
        .map_err(|e| failure("inspect case alias", path, e))?
    {
        Err(collision())
    } else {
        Ok(())
    }
}

// The same file ID may be reached through another hard-link name (including a
// case variant of that name). Only a case variant of the selected basename can
// be a case-only rename. The exact-name scan above still rejects a second entry
// in a case-sensitive directory. Raw UTF-16 and ordinal folding preserve native
// Unicode names without depending on the application's display locale.
fn same_name_ignoring_case(left: &OsStr, right: &OsStr) -> io::Result<bool> {
    let left: Vec<_> = left.encode_wide().collect();
    let right: Vec<_> = right.encode_wide().collect();
    let length = |units: &[u16]| {
        i32::try_from(units.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "filename is too long"))
    };
    let left_len = length(&left)?;
    let right_len = length(&right)?;
    // SAFETY: live UTF-16 buffers and checked explicit lengths; no terminators are required.
    let compared =
        unsafe { CompareStringOrdinal(left.as_ptr(), left_len, right.as_ptr(), right_len, 1) };
    if compared == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(compared == CSTR_EQUAL)
    }
}

pub(super) fn snapshot(
    item: &WindowsMutation,
    cancel: &Cancellation,
) -> Result<MetadataSnapshot, FsError> {
    checked(cancel, &item.item.path)?;
    let source = item.revalidate(false)?;
    snapshot_file(&source, &item.item.path)
}

fn snapshot_file(source: &File, path: &Path) -> Result<MetadataSnapshot, FsError> {
    let meta = source
        .metadata()
        .map_err(|e| failure("inspect metadata", path, e))?;
    let attrs = meta.file_attributes();
    let ordinary = attrs & FILE_ATTRIBUTE_REPARSE_POINT == 0;
    let mut standard = FILE_STANDARD_INFO::default();
    // SAFETY: live owned handle, fixed-size writable output.
    let links = unsafe {
        GetFileInformationByHandleEx(
            source.as_raw_handle(),
            FileStandardInfo,
            (&mut standard as *mut FILE_STANDARD_INFO).cast(),
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    };
    let mut flags = 0;
    // SAFETY: only the flags output is requested; all optional buffers are null/zero.
    let volume = unsafe {
        GetVolumeInformationByHandleW(
            source.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut flags,
            std::ptr::null_mut(),
            0,
        )
    };
    Ok(MetadataSnapshot {
        capabilities: MetadataCapabilities {
            windows_attributes: if ordinary {
                EDIT_READ_ONLY | EDIT_HIDDEN | EDIT_SYSTEM | EDIT_ARCHIVE
            } else {
                0
            },
            compressed: ordinary
                && attrs & FILE_ATTRIBUTE_ENCRYPTED == 0
                && volume != 0
                && flags & 0x10 != 0, // FILE_FILE_COMPRESSION
            created: ordinary,
            modified: ordinary,
            accessed: ordinary,
            rename: true,
            ..Default::default()
        },
        windows_attributes: Some(attrs),
        unix_mode: None,
        compressed: Some(attrs & FILE_ATTRIBUTE_COMPRESSED != 0),
        encrypted: Some(attrs & FILE_ATTRIBUTE_ENCRYPTED != 0),
        created: meta.created().ok(),
        modified: meta.modified().ok(),
        accessed: meta.accessed().ok(),
        hard_links: if links != 0 {
            standard.NumberOfLinks as u64
        } else {
            1
        },
    })
}

pub(super) fn children(
    item: &WindowsMutation,
    cancel: &Cancellation,
    max_targets: usize,
) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
    checked(cancel, &item.item.path)?;
    if item.item.kind != EntryKind::Directory {
        return Err(invalid(
            &item.item.path,
            "only ordinary directories can be traversed",
        ));
    }
    let source = item.revalidate(false)?;
    let enumeration = native::open_enumeration(&item.parent, &source, &item.name)
        .map_err(|e| failure("enumerate metadata targets", &item.item.path, e))?;
    let names = native::bounded_child_names(
        &enumeration,
        cancel,
        max_targets.min(MAX_METADATA_TARGETS),
        MAX_METADATA_NAMES_BYTES,
    )
    .map_err(|e| failure("snapshot metadata targets", &item.item.path, e))?;
    let mut children: Vec<Arc<dyn PreparedMutation>> = Vec::with_capacity(names.len());
    for name in names {
        checked(cancel, &item.item.path)?;
        let path = item.item.path.join(&name);
        let child = native::open_child(&source, &name, false)
            .map_err(|e| failure("pin child", &path, e))?;
        let identity = native::identity(&child).map_err(|e| failure("identify child", &path, e))?;
        if identity.volume != item.identity.volume {
            return Err(invalid(&path, "mount crossings are not allowed"));
        }
        let meta = child
            .metadata()
            .map_err(|e| failure("inspect child", &path, e))?;
        let attrs = meta.file_attributes();
        let kind = match (
            attrs & FILE_ATTRIBUTE_DIRECTORY != 0,
            attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0,
        ) {
            (true, false) => EntryKind::Directory,
            (true, true) => EntryKind::DirectoryLink,
            (false, true) => EntryKind::Symlink,
            (false, false) => EntryKind::File,
        };
        children.push(Arc::new(WindowsMutation {
            item: MutationItem {
                path,
                kind,
                metadata: EntryMetadata {
                    native_attributes: Some(attrs),
                    modified: meta.modified().ok(),
                    ..Default::default()
                },
            },
            parent: source
                .try_clone()
                .map_err(|e| failure("pin parent", &item.item.path, e))?,
            source: child,
            name,
            identity,
        }));
    }
    Ok(children)
}

pub(super) fn edit(
    item: &WindowsMutation,
    patch: &MetadataPatch,
    name: Option<&OsStr>,
    cancel: &Cancellation,
) -> MetadataOutcome {
    let mut outcome = MetadataOutcome::unchanged(item.item.path.clone());
    if let Err(error) = apply(item, patch, name, cancel, &mut outcome) {
        outcome.error = Some(error);
    }
    outcome
}

fn apply(
    item: &WindowsMutation,
    patch: &MetadataPatch,
    name: Option<&OsStr>,
    cancel: &Cancellation,
    result: &mut MetadataOutcome,
) -> Result<(), FsError> {
    let path = &item.item.path;
    checked(cancel, path)?;
    let original = item.revalidate(false)?;
    let snapshot = snapshot_file(&original, path)?;
    validate_patch(patch, snapshot.capabilities, path)?;
    let times = [
        file_time(patch.created, path)?,
        file_time(patch.accessed, path)?,
        file_time(patch.modified, path)?,
    ];
    if let Some(name) = name {
        validate_name(name).map_err(|e| failure("validate case name", path, e))?;
    }
    let rename = name.filter(|name| *name != item.name);
    let access = FILE_READ_ATTRIBUTES
        | SYNCHRONIZE
        | if patch.has_metadata_changes() {
            FILE_WRITE_ATTRIBUTES
        } else {
            0
        }
        | if rename.is_some() { DELETE } else { 0 };
    let source = native::open_relative(
        &item.parent,
        &item.name,
        access,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        FILE_OPEN,
        0,
    )
    .map_err(|e| failure("open metadata target", path, e))?;
    if native::identity(&source).map_err(|e| failure("revalidate metadata identity", path, e))?
        != item.identity
    {
        return Err(changed(path));
    }
    if let Some(name) = rename {
        name_available(item, name, cancel)?;
    }
    checked(cancel, path)?;
    if let Some(name) = rename {
        native::rename(&source, name).map_err(|e| failure("change filename case", path, e))?;
        result.current_path = path.with_file_name(name);
        result.changes.renamed = true;
    }
    // Opening for data writes fails while Read-only is set. Apply the explicitly requested
    // attribute patch first only when it clears that flag; name checks/rename have completed.
    // Keep the metadata handle (which denies delete sharing) alive throughout both operations.
    let attributes_first = patch.compressed.is_some()
        && patch.windows_mask & EDIT_READ_ONLY != 0
        && patch.windows_value & EDIT_READ_ONLY == 0;
    if attributes_first {
        apply_attributes(&source, patch, cancel, result)?;
    }
    if let Some(compressed) = patch.compressed {
        checked(cancel, &result.current_path)?;
        // Reopen the pinned object, never its possibly renamed pathname. The new sharing mode
        // permits the guard's DELETE access; the retained guard still prevents external rename.
        // SAFETY: the source is a live file handle; the returned handle is independently owned.
        let handle = unsafe {
            ReOpenFile(
                source.as_raw_handle(),
                FILE_READ_DATA | FILE_WRITE_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(failure(
                "open compression target",
                &result.current_path,
                io::Error::last_os_error(),
            ));
        }
        // SAFETY: ReOpenFile succeeded; transfer its unique ownership to File for all exit paths.
        let compression_source = unsafe { File::from_raw_handle(handle) };
        if native::identity(&compression_source)
            .map_err(|e| failure("revalidate compression identity", &result.current_path, e))?
            != item.identity
        {
            return Err(changed(&result.current_path));
        }
        let format: u16 = if compressed { 1 } else { 0 }; // COMPRESSION_FORMAT_DEFAULT / NONE
        let mut returned = 0;
        // SAFETY: live handle and fixed-size input remain valid for this synchronous call.
        if unsafe {
            DeviceIoControl(
                compression_source.as_raw_handle(),
                FSCTL_SET_COMPRESSION,
                (&format as *const u16).cast(),
                2,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(failure(
                "change compression",
                &result.current_path,
                io::Error::last_os_error(),
            ));
        }
        result.changes.compressed = true;
    }
    if !attributes_first {
        apply_attributes(&source, patch, cancel, result)?;
    }
    if times.iter().any(Option::is_some) {
        checked(cancel, &result.current_path)?;
        let pointer = |i: usize| {
            times[i]
                .as_ref()
                .map_or(std::ptr::null(), |value| value as *const FILETIME)
        };
        // SAFETY: live handle and optional timestamps remain alive throughout the call.
        if unsafe { SetFileTime(source.as_raw_handle(), pointer(0), pointer(1), pointer(2)) } == 0 {
            return Err(failure(
                "change timestamps",
                &result.current_path,
                io::Error::last_os_error(),
            ));
        }
        result.changes.created = times[0].is_some();
        result.changes.accessed = times[1].is_some();
        result.changes.modified = times[2].is_some();
    }
    Ok(())
}

fn apply_attributes(
    source: &File,
    patch: &MetadataPatch,
    cancel: &Cancellation,
    result: &mut MetadataOutcome,
) -> Result<(), FsError> {
    if patch.windows_mask != 0 {
        checked(cancel, &result.current_path)?;
        let attrs = source
            .metadata()
            .map_err(|e| failure("read current attributes", &result.current_path, e))?
            .file_attributes();
        let mut next = (attrs & !patch.windows_mask) | (patch.windows_value & patch.windows_mask);
        next &= !FILE_ATTRIBUTE_NORMAL;
        if next == 0 {
            next = FILE_ATTRIBUTE_NORMAL;
        }
        let basic = FILE_BASIC_INFO {
            FileAttributes: next,
            ..Default::default()
        };
        // SAFETY: fixed-size valid structure, live handle has FILE_WRITE_ATTRIBUTES.
        if unsafe {
            SetFileInformationByHandle(
                source.as_raw_handle(),
                FileBasicInfo,
                (&basic as *const FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        } == 0
        {
            return Err(failure(
                "change attributes",
                &result.current_path,
                io::Error::last_os_error(),
            ));
        }
        let actual = source
            .metadata()
            .map_err(|e| failure("verify attributes", &result.current_path, e))?
            .file_attributes();
        result.changes.attributes |= attrs != actual;
        if actual & patch.windows_mask != patch.windows_value & patch.windows_mask {
            return Err(failure(
                "verify attributes",
                &result.current_path,
                io::Error::other("the filesystem did not retain the requested attributes"),
            ));
        }
    }
    Ok(())
}

fn file_time(edit: TimeEdit, path: &Path) -> Result<Option<FILETIME>, FsError> {
    let TimeEdit::Set(time) = edit else {
        return Ok(None);
    };
    let base: i128 = 116_444_736_000_000_000;
    let ticks = match time.duration_since(UNIX_EPOCH) {
        Ok(delta) => base + (delta.as_nanos() / 100) as i128,
        Err(error) => base - (error.duration().as_nanos().div_ceil(100)) as i128,
    };
    if ticks <= 0 || ticks > i64::MAX as i128 {
        return Err(invalid(
            path,
            "timestamp is outside the Windows file-time range",
        ));
    }
    let ticks = ticks as u64;
    Ok(Some(FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    }))
}
