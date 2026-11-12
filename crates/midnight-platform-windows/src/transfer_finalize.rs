//! Exact-handle regular-file disposition after independent per-file proof checks.
use super::super::destination::recovery::Revision;
use super::*;
use midnight_core::{TransferMoveFinalizeOutcome, TransferMoveFinalizer, TransferSourceRemoval};
use std::{
    mem::{offset_of, size_of},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{ERROR_HANDLE_EOF, ERROR_IO_PENDING, INVALID_HANDLE_VALUE},
    System::{
        IO::{CancelIoEx, DeviceIoControl, GetOverlappedResult, OVERLAPPED},
        Ioctl::*,
        Threading::CreateEventW,
    },
};

pub(super) fn qualify(file: &File, path: &Path) -> Result<(), FsError> {
    let mut standard = FILE_STANDARD_INFO::default();
    // SAFETY: live handle and a writable structure with its exact native size.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileStandardInfo,
            (&mut standard as *mut FILE_STANDARD_INFO).cast(),
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    } == 0
    {
        return Err(error(path, io::Error::last_os_error()));
    }
    if standard.Directory
        || standard.NumberOfLinks != 1
        || file
            .metadata()
            .map_err(|e| error(path, e))?
            .file_attributes()
            & FILE_ATTRIBUTE_READONLY
            != 0
    {
        return Err(unsupported(
            path,
            "Verified Move removal requires a single-link regular file without the readonly attribute. Choose Copy to retain the source.",
        ));
    }
    check_streams(file, path, false)
}

pub(super) fn qualify_directory(file: &File, path: &Path) -> Result<(), FsError> {
    let metadata = file.metadata().map_err(|e| error(path, e))?;
    if !metadata.is_dir()
        || metadata.file_attributes() & (FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_REPARSE_POINT)
            != 0
    {
        return Err(unsupported(
            path,
            "Directory Move requires an ordinary directory without the readonly attribute.",
        ));
    }
    check_streams(file, path, true)
}

fn check_streams(file: &File, path: &Path, directory: bool) -> Result<(), FsError> {
    // Fixed bound: inability to enumerate every stream is refusal, never omission.
    let mut buffer = vec![0u8; 64 * 1024];
    // SAFETY: live handle; byte buffer remains writable for its stated size.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileStreamInfo,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
        )
    } == 0
    {
        let failure = io::Error::last_os_error();
        // Ordinary NTFS directories have no unnamed data stream. EOF explicitly
        // reports an empty stream list; other errors never authorize removal.
        if directory && failure.raw_os_error() == Some(ERROR_HANDLE_EOF as i32) {
            return Ok(());
        }
        return Err(unsupported(
            path,
            &format!(
                "Cannot establish that all source streams are verified: {}",
                failure
            ),
        ));
    }
    let mut offset = 0usize;
    loop {
        let header = buffer
            .get(offset..offset.saturating_add(size_of::<FILE_STREAM_INFO>()))
            .ok_or_else(|| invalid(path, "invalid native stream information"))?;
        // SAFETY: checked size, read_unaligned accepts byte-buffer alignment.
        let info = unsafe { ptr::read_unaligned(header.as_ptr().cast::<FILE_STREAM_INFO>()) };
        let start = offset + offset_of!(FILE_STREAM_INFO, StreamName);
        let name = buffer
            .get(start..start.saturating_add(info.StreamNameLength as usize))
            .ok_or_else(|| invalid(path, "native stream name exceeds bound"))?;
        let expected: Vec<u8> = "::$DATA"
            .encode_utf16()
            .flat_map(u16::to_ne_bytes)
            .collect();
        if directory || name != expected {
            return Err(unsupported(
                path,
                "The source has alternate data streams that were not copied and verified. Choose Copy to retain every original stream.",
            ));
        }
        if info.NextEntryOffset == 0 {
            return Ok(());
        }
        let next = info.NextEntryOffset as usize;
        if next < size_of::<FILE_STREAM_INFO>() || next > buffer.len() - offset {
            return Err(invalid(path, "invalid native stream chain"));
        }
        offset += next;
    }
}

fn probe_mapped_writers(file: &File, path: &Path) -> Result<(), FsError> {
    // SAFETY: reopens the exact retained native file object, never its pathname.
    let handle = unsafe {
        ReOpenFile(
            file.as_raw_handle(),
            FILE_READ_DATA | FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_FLAG_OVERLAPPED | FILE_FLAG_OPEN_REPARSE_POINT,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(error(path, io::Error::last_os_error()));
    }
    // SAFETY: ownership starts only after successful native handle creation.
    let held = unsafe { File::from_raw_handle(handle) };
    // SAFETY: unnamed manual-reset event, no inherited handle/security override.
    let event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
    if event.is_null() {
        return Err(error(path, io::Error::last_os_error()));
    }
    // SAFETY: ownership starts only after successful native handle creation.
    let event = unsafe { OwnedHandle::from_raw_handle(event) };
    let input = REQUEST_OPLOCK_INPUT_BUFFER {
        StructureVersion: REQUEST_OPLOCK_CURRENT_VERSION as u16,
        StructureLength: size_of::<REQUEST_OPLOCK_INPUT_BUFFER>() as u16,
        RequestedOplockLevel: OPLOCK_LEVEL_CACHE_READ,
        Flags: REQUEST_OPLOCK_INPUT_FLAG_REQUEST,
    };
    let mut output = REQUEST_OPLOCK_OUTPUT_BUFFER::default();
    let mut overlapped = OVERLAPPED {
        hEvent: event.as_raw_handle(),
        ..Default::default()
    };
    // SAFETY: pinned stack buffers/OVERLAPPED and event outlive the pending IO;
    // completion is drained below before any buffer/handle is dropped.
    let result = unsafe {
        DeviceIoControl(
            held.as_raw_handle(),
            FSCTL_REQUEST_OPLOCK,
            (&input as *const REQUEST_OPLOCK_INPUT_BUFFER).cast(),
            size_of::<REQUEST_OPLOCK_INPUT_BUFFER>() as u32,
            (&mut output as *mut REQUEST_OPLOCK_OUTPUT_BUFFER).cast(),
            size_of::<REQUEST_OPLOCK_OUTPUT_BUFFER>() as u32,
            ptr::null_mut(),
            &mut overlapped,
        )
    };
    let failure = io::Error::last_os_error();
    if result != 0 || failure.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
        return Err(unsupported(
            path,
            &format!(
                "Source mapped-writer quiescence could not be established: {failure}. Source removal refused."
            ),
        ));
    }
    // A Read grant rejects existing writable sections. The ORIGINAL retained source
    // handle denies write sharing, so new writable handles cannot appear after this
    // probe. The advisory oplock itself is not treated as an immutable snapshot.
    let mut bytes = 0;
    // SAFETY: cancel only this owned pending request, then synchronously drain it.
    // Even ERROR_NOT_FOUND means completion raced; GetOverlappedResult still drains.
    unsafe {
        CancelIoEx(held.as_raw_handle(), &overlapped);
        GetOverlappedResult(held.as_raw_handle(), &overlapped, &mut bytes, 1);
    }
    Ok(())
}

pub(super) fn prepare(
    capture: &Capture,
    cancel: &Cancellation,
) -> Result<Box<dyn TransferMoveFinalizer>, FsError> {
    checked(cancel, &capture.entry.item.path)?;
    let state = capture.state.lock().unwrap_or_else(|e| e.into_inner());
    let Some(TransferMoveCaptureOutcome::Captured { source, recovery }) = &state.outcome else {
        return Err(invalid(
            &capture.entry.item.path,
            "source has no successful matching temporary staging",
        ));
    };
    let directory = state
        .retained
        .as_ref()
        .ok_or_else(|| invalid(source, "temporary source authority missing"))?
        .clone();
    qualify(&capture.file, source)?;
    probe_mapped_writers(&capture.file, source)?;
    let finalizer = Finalizer {
        source: source.clone(),
        receipt: recovery.clone(),
        directory,
        name: capture.name.clone(),
        file: capture.file.clone(),
        revision: Revision::of(&capture.file, source)?,
        outcome: None,
    };
    finalizer.revalidate(cancel)?;
    Ok(Box::new(finalizer))
}

struct Finalizer {
    source: std::path::PathBuf,
    receipt: TransferRecovery,
    directory: destination::Directory,
    name: OsString,
    file: Arc<File>,
    revision: Revision,
    outcome: Option<TransferMoveFinalizeOutcome>,
}
impl TransferMoveFinalizer for Finalizer {
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        checked(cancel, &self.source)?;
        self.directory.revalidate(cancel)?;
        qualify(&self.file, &self.source)?;
        if identity(&self.file, &self.source)?
            != self
                .receipt
                .identity
                .ok_or_else(|| invalid(&self.source, "temporary source identity unknown"))?
            || Revision::of(&self.file, &self.source)? != self.revision
            || self
                .directory
                .inspect(&self.name, cancel)?
                .map(|entry| entry.identity)
                != self.receipt.identity
        {
            return Err(invalid(
                &self.source,
                "source revision or temporary binding changed; original preserved",
            ));
        }
        Ok(())
    }
    fn finalize(&mut self, cancel: &Cancellation) -> TransferMoveFinalizeOutcome {
        if let Some(outcome) = &self.outcome {
            return outcome.clone();
        }
        let removal = self.revalidate(cancel).and_then(|()| {
            checked(cancel, &self.source)?;
            let info = FILE_DISPOSITION_INFO_EX {
                Flags: FILE_DISPOSITION_FLAG_DELETE
                    | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS
                    | FILE_DISPOSITION_FLAG_FORCE_IMAGE_SECTION_CHECK,
            };
            // SAFETY: live DELETE-capable source handle, fixed native structure. No
            // readonly override. POSIX removes this staged link when our DELETE
            // owners close; other read handles may still retain physical storage.
            if unsafe {
                SetFileInformationByHandle(
                    self.file.as_raw_handle(),
                    FileDispositionInfoEx,
                    (&info as *const FILE_DISPOSITION_INFO_EX).cast(),
                    size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
                )
            } == 0
            {
                return Err(error(&self.source, io::Error::last_os_error()));
            }
            Ok(())
        });
        let outcome = match removal {
            Err(error) => TransferMoveFinalizeOutcome::Retained {
                source: self.source.clone(),
                recovery: self.receipt.clone(),
                error,
            },
            Ok(()) => TransferMoveFinalizeOutcome::Removed {
                source: self.source.clone(),
                recovery: self.receipt.clone(),
                state: TransferSourceRemoval::DeletePending,
                durability_confirmed: false,
            },
        };
        self.outcome = Some(outcome.clone());
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Memory::*;

    #[test]
    fn read_oplock_probe_refuses_surviving_writable_mapping_and_accepts_after_unmap() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
        let temp = tempfile::tempdir_in(root).unwrap();
        let path = temp.path().join("mapped-source");
        std::fs::write(&path, vec![42; 4096]).unwrap();
        let writer = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(&path)
            .unwrap();
        // SAFETY: owned writable file; unnamed mapping, no security override.
        let mapping = unsafe {
            CreateFileMappingW(
                writer.as_raw_handle(),
                ptr::null(),
                PAGE_READWRITE,
                0,
                0,
                ptr::null(),
            )
        };
        assert!(!mapping.is_null());
        // SAFETY: starts ownership only for successful native mapping handle.
        let mapping = unsafe { OwnedHandle::from_raw_handle(mapping) };
        // SAFETY: mapping covers the owned file; release exact view below.
        let view = unsafe { MapViewOfFile(mapping.as_raw_handle(), FILE_MAP_WRITE, 0, 0, 4096) };
        assert!(!view.Value.is_null());
        drop(writer);
        let reader = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(&path)
            .unwrap();
        assert!(probe_mapped_writers(&reader, &path).is_err());
        // SAFETY: writable owned mapping; edit stays inside its length.
        unsafe {
            view.Value.cast::<u8>().write(99);
        }
        // SAFETY: releases exactly the successful view, no further view accesses.
        assert_ne!(unsafe { UnmapViewOfFile(view) }, 0);
        drop(mapping);
        probe_mapped_writers(&reader, &path).unwrap();
        assert_eq!(std::fs::read(path).unwrap()[0], 99);
    }
}

#[cfg(test)]
mod section_tests {
    use super::*;
    use windows_sys::Win32::System::Memory::*;
    #[test]
    fn read_oplock_refuses_a_writable_section_handle_before_any_view_is_mapped() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
        let temp = tempfile::tempdir_in(root).unwrap();
        let path = temp.path().join("future-mapping");
        std::fs::write(&path, vec![42; 4096]).unwrap();
        let writer = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(&path)
            .unwrap();
        // SAFETY: owned writable regular file; unnamed mapping with default security.
        let mapping = unsafe {
            CreateFileMappingW(
                writer.as_raw_handle(),
                ptr::null(),
                PAGE_READWRITE,
                0,
                0,
                ptr::null(),
            )
        };
        assert!(!mapping.is_null());
        // SAFETY: ownership begins only after successful native handle creation.
        let mapping = unsafe { OwnedHandle::from_raw_handle(mapping) };
        drop(writer);
        let reader = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(&path)
            .unwrap();
        assert!(
            probe_mapped_writers(&reader, &path).is_err(),
            "future writable views must not escape source quiescence"
        );
        drop(mapping);
        probe_mapped_writers(&reader, &path).unwrap();
    }
}
