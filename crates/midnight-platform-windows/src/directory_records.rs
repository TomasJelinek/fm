//! Bounded native directory records read through a directory handle.
//!
//! Deletion, discovery, Find and transfer enumeration share this reader. Each record
//! already carries the attributes, reparse tag, size and modification time, so callers
//! classify children without opening them. One 64 KiB record buffer is reused per
//! thread instead of allocating and zeroing a new one for every batch.
use crate::reparse::Native;
use std::{
    cell::Cell,
    fs::File,
    io,
    mem::{offset_of, size_of, size_of_val},
    os::windows::io::AsRawHandle,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES},
    Storage::FileSystem::{
        FILE_ID_BOTH_DIR_INFO, FileIdBothDirectoryInfo, GetFileInformationByHandleEx,
    },
};

const BUFFER_WORDS: usize = 64 * 1024 / size_of::<u64>();

thread_local! {
    static BUFFER: Cell<Option<Box<[u64]>>> = const { Cell::new(None) };
}

/// One directory record other than `.` and `..`.
pub(crate) struct Record<'a> {
    pub(crate) name: &'a [u16],
    pub(crate) native: Native,
    /// End of file in bytes; meaningful for files.
    pub(crate) size: u64,
    last_write: i64,
}

impl Record<'_> {
    pub(crate) fn modified(&self) -> Option<SystemTime> {
        system_time(self.last_write)
    }
}

/// Enumeration state of one directory handle.
#[derive(Default)]
pub(crate) struct Cursor {
    started: bool,
    ended: bool,
}

impl Cursor {
    pub(crate) fn ended(&self) -> bool {
        self.ended
    }

    /// Reads the next batch of records and calls `visit` for each one in native order.
    /// At the end of the directory the cursor becomes ended and no record is visited.
    pub(crate) fn next_batch(
        &mut self,
        directory: &File,
        visit: &mut dyn FnMut(Record<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        if self.ended {
            return Ok(());
        }
        let mut buffer = BUFFER
            .take()
            .unwrap_or_else(|| vec![0u64; BUFFER_WORDS].into_boxed_slice());
        let result = self.read(directory, &mut buffer, visit);
        BUFFER.set(Some(buffer));
        result
    }

    fn read(
        &mut self,
        directory: &File,
        buffer: &mut [u64],
        visit: &mut dyn FnMut(Record<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        // SAFETY: an aligned writable buffer of the stated byte length and a live,
        // synchronous directory handle remain valid for this native query.
        if unsafe {
            GetFileInformationByHandleEx(
                directory.as_raw_handle(),
                FileIdBothDirectoryInfo,
                buffer.as_mut_ptr().cast(),
                size_of_val(buffer) as u32,
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            if end_of_directory(error.raw_os_error(), self.started) {
                self.ended = true;
                return Ok(());
            }
            return Err(error);
        }
        self.started = true;
        parse(buffer, visit)
    }
}

/// Filesystems that do not synthesize `.` and `..` (FAT/exFAT volume roots, several
/// user-mode filesystems) report an empty directory on the first query of a handle as
/// "no such file" instead of "no more files". Later, that error is a real failure.
fn end_of_directory(code: Option<i32>, started: bool) -> bool {
    code == Some(ERROR_NO_MORE_FILES as i32)
        || (!started && code == Some(ERROR_FILE_NOT_FOUND as i32))
}

fn parse(buffer: &[u64], visit: &mut dyn FnMut(Record<'_>) -> io::Result<()>) -> io::Result<()> {
    let length = size_of_val(buffer);
    // Every record and filename pointer derives from the whole buffer, never from the
    // one-element FileName field, so reads past that field keep their provenance.
    let base = buffer.as_ptr().cast::<u8>();
    let header = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
    let mut offset = 0usize;
    loop {
        if offset + size_of::<FILE_ID_BOTH_DIR_INFO>() > length {
            return Err(invalid());
        }
        // SAFETY: the complete record header lies inside the aligned buffer and native
        // record offsets are checked to stay eight-byte aligned below.
        let record = unsafe { &*base.add(offset).cast::<FILE_ID_BOTH_DIR_INFO>() };
        let bytes = record.FileNameLength as usize;
        if !bytes.is_multiple_of(2) || offset + header + bytes > length {
            return Err(invalid());
        }
        // SAFETY: the counted UTF-16 name lies inside the buffer (checked above) and
        // starts at an even offset from the eight-byte aligned buffer base.
        let name = unsafe {
            std::slice::from_raw_parts(base.add(offset + header).cast::<u16>(), bytes / 2)
        };
        if name != [46] && name != [46, 46] {
            visit(Record {
                name,
                native: Native::new(record.FileAttributes, record.EaSize),
                size: u64::try_from(record.EndOfFile).unwrap_or(0),
                last_write: record.LastWriteTime,
            })?;
        }
        if record.NextEntryOffset == 0 {
            return Ok(());
        }
        let next = record.NextEntryOffset as usize;
        if !next.is_multiple_of(8) || next < header + bytes {
            return Err(invalid());
        }
        offset += next;
    }
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid native directory record",
    )
}

/// Converts a FILETIME tick count (100 ns since 1601) exactly as std's metadata does.
pub(crate) fn system_time(ticks: i64) -> Option<SystemTime> {
    const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
    let delta = ticks.checked_sub(UNIX_EPOCH_TICKS)?;
    let ticks = delta.unsigned_abs();
    let duration = Duration::new(ticks / 10_000_000, (ticks % 10_000_000) as u32 * 100);
    if delta >= 0 {
        UNIX_EPOCH.checked_add(duration)
    } else {
        UNIX_EPOCH.checked_sub(duration)
    }
}

#[cfg(test)]
#[path = "directory_records_tests.rs"]
mod tests;
