//! Read-only, handle-relative discovery. Only directory enumeration data is read.
use super::*;
use std::{
    collections::VecDeque,
    mem::{offset_of, size_of},
    os::windows::{ffi::OsStringExt, io::AsRawHandle},
};
use windows_sys::Win32::{
    Foundation::ERROR_NO_MORE_FILES,
    Storage::FileSystem::{
        FILE_ID_BOTH_DIR_INFO, FileIdBothDirectoryInfo, GetFileInformationByHandleEx,
    },
};

struct Frame {
    directory: File,
    path: PathBuf,
    names: VecDeque<OsString>,
    ended: bool,
}
pub(crate) fn walk(
    parent: &File,
    source: &File,
    name: &OsStr,
    path: &Path,
    cancel: &Cancellation,
    visit: &mut dyn FnMut(&MutationItem),
) -> Result<(), FsError> {
    checked(cancel, path)?;
    let mut stack = Vec::new();
    if let Some(frame) = inspect(parent, source, name, path, cancel, visit)? {
        stack.push(frame);
    }
    while let Some(frame) = stack.last_mut() {
        checked(cancel, &frame.path)?;
        if frame.names.is_empty() && !frame.ended {
            fill(frame)?;
        }
        let Some(name) = frame.names.pop_front() else {
            if frame.ended {
                stack.pop();
            }
            continue;
        };
        let path = frame.path.join(&name);
        let source = native::open_child(&frame.directory, &name, false)
            .map_err(|error| failure("discover child", &path, error))?;
        if let Some(child) = inspect(&frame.directory, &source, &name, &path, cancel, visit)? {
            if stack.len() >= 128 {
                return Err(failure(
                    "discover",
                    &path,
                    io::Error::new(
                        io::ErrorKind::Unsupported,
                        "Discovery exceeds the 128-directory depth budget",
                    ),
                ));
            }
            stack.push(child);
        }
    }
    Ok(())
}
fn inspect(
    parent: &File,
    source: &File,
    name: &OsStr,
    path: &Path,
    cancel: &Cancellation,
    visit: &mut dyn FnMut(&MutationItem),
) -> Result<Option<Frame>, FsError> {
    checked(cancel, path)?;
    let metadata = source
        .metadata()
        .map_err(|error| failure("discover metadata", path, error))?;
    let attrs = metadata.file_attributes();
    let kind = if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        EntryKind::Symlink
    } else if metadata.is_dir() {
        EntryKind::Directory
    } else if metadata.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    visit(&MutationItem {
        path: path.to_owned(),
        kind,
        metadata: EntryMetadata {
            size: (kind == EntryKind::File).then_some(metadata.file_size()),
            modified: metadata.modified().ok(),
            native_attributes: Some(attrs),
            ..EntryMetadata::default()
        },
    });
    if kind != EntryKind::Directory {
        return Ok(None);
    }
    let directory = native::open_enumeration(parent, source, name)
        .map_err(|error| failure("discover directory", path, error))?;
    Ok(Some(Frame {
        directory,
        path: path.to_owned(),
        names: VecDeque::new(),
        ended: false,
    }))
}
fn fill(frame: &mut Frame) -> Result<(), FsError> {
    let mut buffer = vec![0u64; 8192];
    // SAFETY: one live directory handle and an aligned writable 64 KiB buffer.
    if unsafe {
        GetFileInformationByHandleEx(
            frame.directory.as_raw_handle(),
            FileIdBothDirectoryInfo,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * 8) as u32,
        )
    } == 0
    {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
            frame.ended = true;
            return Ok(());
        }
        return Err(failure("discover directory", &frame.path, error));
    }
    let invalid = || {
        failure(
            "discover directory",
            &frame.path,
            io::ErrorKind::InvalidData.into(),
        )
    };
    let mut offset = 0usize;
    loop {
        let header = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
        if offset + size_of::<FILE_ID_BOTH_DIR_INFO>() > buffer.len() * 8 {
            return Err(invalid());
        }
        // SAFETY: the range is checked above and native records have eight-byte alignment.
        let record = unsafe {
            &*buffer
                .as_ptr()
                .cast::<u8>()
                .add(offset)
                .cast::<FILE_ID_BOTH_DIR_INFO>()
        };
        let bytes = record.FileNameLength as usize;
        if !bytes.is_multiple_of(2) || offset + header + bytes > buffer.len() * 8 {
            return Err(invalid());
        }
        // SAFETY: the filename range fits entirely inside the validated native record.
        let units = unsafe { std::slice::from_raw_parts(record.FileName.as_ptr(), bytes / 2) };
        let name = OsString::from_wide(units);
        if name != "." && name != ".." {
            frame.names.push_back(name);
        }
        if record.NextEntryOffset == 0 {
            break;
        }
        let next = record.NextEntryOffset as usize;
        if !next.is_multiple_of(8) || next < header + bytes {
            return Err(invalid());
        }
        offset += next;
    }
    Ok(())
}
