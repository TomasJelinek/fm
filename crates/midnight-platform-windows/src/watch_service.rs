//! Owned `ReadDirectoryChangesW` service: one thread blocked on one I/O completion port
//! serves every subscription, so idle watches cause no wakeups. The thread issues every
//! read and owns every buffer and OVERLAPPED; subscriptions only post commands.
//!
//! A completion maps to the registration's own bit, plus the bit of any registered
//! target that is the reported immediate child. A zero-byte completion (the 64 KiB
//! buffer overflowed), `ERROR_NOTIFY_ENUM_DIR` and every other error signal all bits,
//! as the core watch contract requires. Overflow re-arms; any other error stops that
//! directory's watch (it was removed or became inaccessible) after signalling.
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io,
    mem::{offset_of, size_of, size_of_val},
    os::windows::{
        ffi::OsStrExt,
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::PathBuf,
    ptr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};
use windows_sys::Win32::{
    Foundation::{ERROR_NOTIFY_ENUM_DIR, GetLastError, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY,
        FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_CREATION, FILE_NOTIFY_CHANGE_DIR_NAME,
        FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SECURITY,
        FILE_NOTIFY_CHANGE_SIZE, FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, ReadDirectoryChangesW,
    },
    System::IO::{
        CancelIoEx, CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED,
        PostQueuedCompletionStatus,
    },
};

/// Receives a changed-target mask; runs briefly on the service thread.
pub(crate) type Signal = Arc<dyn Fn(u32) + Send + Sync>;

const COMMAND_KEY: usize = 0;
const INFINITE: u32 = u32::MAX;
/// The largest buffer `ReadDirectoryChangesW` accepts for network directories.
const BUFFER_UNITS: usize = 64 * 1024 / size_of::<u32>();
const FILTER: u32 = FILE_NOTIFY_CHANGE_FILE_NAME
    | FILE_NOTIFY_CHANGE_DIR_NAME
    | FILE_NOTIFY_CHANGE_ATTRIBUTES
    | FILE_NOTIFY_CHANGE_SIZE
    | FILE_NOTIFY_CHANGE_LAST_WRITE
    | FILE_NOTIFY_CHANGE_CREATION
    | FILE_NOTIFY_CHANGE_SECURITY;

enum Command {
    Register {
        id: u64,
        directories: Vec<File>,
        targets: Arc<[PathBuf]>,
        signal: Signal,
        reply: mpsc::SyncSender<io::Result<()>>,
    },
    Unregister(u64),
}

pub(crate) struct Service {
    port: OwnedHandle,
    /// `None` once the service thread has stopped; registrations then fail at once.
    commands: Mutex<Option<Vec<Command>>>,
    next: AtomicU64,
}

/// Unregisters on drop without waiting: the service thread cancels and frees the
/// native reads. The owner disables its callback first (see `watch.rs`).
pub(crate) struct Registration {
    id: u64,
    service: &'static Service,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.service.post(Command::Unregister(self.id));
    }
}

impl Service {
    pub(crate) fn get() -> io::Result<&'static Self> {
        static SERVICE: OnceLock<Result<&'static Service, String>> = OnceLock::new();
        SERVICE
            .get_or_init(|| Self::start().map_err(|error| error.to_string()))
            .as_ref()
            .copied()
            .map_err(|message| io::Error::other(message.clone()))
    }

    fn start() -> io::Result<&'static Self> {
        // SAFETY: creates a new completion port; no file handle is associated yet.
        let port = unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, ptr::null_mut(), 0, 1) };
        if port.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a successful call returned one owned port handle.
        let port = unsafe { OwnedHandle::from_raw_handle(port) };
        // The process-wide service lives as long as the process.
        let service: &'static Self = Box::leak(Box::new(Self {
            port,
            commands: Mutex::new(Some(Vec::new())),
            next: AtomicU64::new(1),
        }));
        std::thread::Builder::new()
            .name("midnight-native-watch".into())
            .spawn(move || Worker::default().run(service))?;
        Ok(service)
    }

    /// Opens every directory on the calling backend worker (a slow share blocks only
    /// that worker), then hands the handles to the service thread, which arms them.
    pub(crate) fn register(
        &'static self,
        targets: Arc<[PathBuf]>,
        signal: Signal,
    ) -> io::Result<Registration> {
        let directories = targets
            .iter()
            .map(|path| {
                OpenOptions::new()
                    .access_mode(FILE_LIST_DIRECTORY)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED)
                    .open(path)
            })
            .collect::<io::Result<Vec<_>>>()?;
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (reply, result) = mpsc::sync_channel(1);
        self.post(Command::Register {
            id,
            directories,
            targets,
            signal,
            reply,
        });
        // Arming never blocks the service thread, so the answer is prompt.
        result
            .recv()
            .map_err(|_| io::Error::other("the native watch service stopped"))??;
        Ok(Registration { id, service: self })
    }

    fn post(&self, command: Command) {
        let mut commands = self.commands.lock().unwrap_or_else(|e| e.into_inner());
        let Some(queue) = commands.as_mut() else {
            if let Command::Register { reply, .. } = command {
                let _ = reply.send(Err(io::Error::other("the native watch service stopped")));
            }
            return;
        };
        queue.push(command);
        drop(commands);
        // SAFETY: a live port; a null OVERLAPPED with the command key only wakes the
        // service thread, which drains the whole queue on every wakeup.
        unsafe {
            PostQueuedCompletionStatus(self.port.as_raw_handle(), 0, COMMAND_KEY, ptr::null())
        };
    }
}

/// One watched directory, boxed so its OVERLAPPED and buffer keep stable addresses
/// while a read is pending. Only the service thread touches it.
struct Entry {
    overlapped: OVERLAPPED,
    buffer: Box<[u32]>,
    directory: File,
    registration: u64,
    bit: u32,
    all: u32,
    /// Registered targets that are immediate children of this directory, by name.
    children: Vec<(Vec<u16>, u32)>,
    signal: Signal,
    pending: bool,
    cancelled: bool,
}

#[derive(Default)]
struct Worker {
    entries: HashMap<usize, Box<Entry>>,
    next_key: usize,
}

impl Worker {
    fn run(mut self, service: &'static Service) {
        loop {
            let mut bytes = 0u32;
            let mut key = 0usize;
            let mut overlapped: *mut OVERLAPPED = ptr::null_mut();
            // SAFETY: a live port and writable outputs; INFINITE blocks without wakeups.
            let completed = unsafe {
                GetQueuedCompletionStatus(
                    service.port.as_raw_handle(),
                    &mut bytes,
                    &mut key,
                    &mut overlapped,
                    INFINITE,
                )
            } != 0;
            if overlapped.is_null() {
                if !completed {
                    // Only a broken port fails without dequeuing anything: stop serving
                    // instead of spinning, and fail pending and later registrations.
                    break;
                }
                let commands = service
                    .commands
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_mut()
                    .map(std::mem::take)
                    .unwrap_or_default();
                for command in commands {
                    self.command(service, command);
                }
                continue;
            }
            // SAFETY: reads the calling thread's error for the failed dequeued read.
            let error = (!completed).then(|| unsafe { GetLastError() });
            self.completed(key, bytes, error);
        }
        let pending = service
            .commands
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .unwrap_or_default();
        for command in pending {
            if let Command::Register { reply, .. } = command {
                let _ = reply.send(Err(io::Error::other("the native watch service stopped")));
            }
        }
    }

    fn command(&mut self, service: &Service, command: Command) {
        match command {
            Command::Register {
                id,
                directories,
                targets,
                signal,
                reply,
            } => {
                let result = self.register(service, id, directories, &targets, signal);
                if result.is_err() {
                    self.unregister(id);
                }
                let _ = reply.send(result);
            }
            Command::Unregister(id) => self.unregister(id),
        }
    }

    fn register(
        &mut self,
        service: &Service,
        id: u64,
        directories: Vec<File>,
        targets: &[PathBuf],
        signal: Signal,
    ) -> io::Result<()> {
        let all = u32::MAX >> (32 - targets.len());
        for (index, directory) in directories.into_iter().enumerate() {
            let children = targets
                .iter()
                .enumerate()
                .filter(|(_, child)| child.parent() == Some(targets[index].as_path()))
                .filter_map(|(child, path)| {
                    path.file_name()
                        .map(|name| (name.encode_wide().collect(), 1 << child))
                })
                .collect();
            self.next_key += 1;
            let key = self.next_key;
            // SAFETY: associates a live overlapped directory handle with the live port;
            // its completions carry `key`.
            if unsafe {
                CreateIoCompletionPort(
                    directory.as_raw_handle(),
                    service.port.as_raw_handle(),
                    key,
                    0,
                )
            }
            .is_null()
            {
                return Err(io::Error::last_os_error());
            }
            let mut entry = Box::new(Entry {
                overlapped: OVERLAPPED::default(),
                buffer: vec![0u32; BUFFER_UNITS].into_boxed_slice(),
                directory,
                registration: id,
                bit: 1 << index,
                all,
                children,
                signal: signal.clone(),
                pending: false,
                cancelled: false,
            });
            arm(&mut entry)?;
            self.entries.insert(key, entry);
        }
        Ok(())
    }

    fn unregister(&mut self, id: u64) {
        let keys: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.registration == id)
            .map(|(key, _)| *key)
            .collect();
        for key in keys {
            let Some(entry) = self.entries.get_mut(&key) else {
                continue;
            };
            entry.cancelled = true;
            if entry.pending {
                // SAFETY: cancels exactly this entry's pending read; its completion still
                // arrives (normally ERROR_OPERATION_ABORTED) and only then frees it.
                unsafe { CancelIoEx(entry.directory.as_raw_handle(), &entry.overlapped) };
            } else {
                self.entries.remove(&key);
            }
        }
    }

    fn completed(&mut self, key: usize, bytes: u32, error: Option<u32>) {
        let Some(entry) = self.entries.get_mut(&key) else {
            return;
        };
        entry.pending = false;
        if entry.cancelled {
            self.entries.remove(&key);
            return;
        }
        let rearm = match error {
            // A zero-byte completion: the buffer overflowed and the changes were lost.
            None if bytes == 0 => {
                (entry.signal)(entry.all);
                true
            }
            None => {
                (entry.signal)(entry.mask(bytes as usize));
                true
            }
            Some(ERROR_NOTIFY_ENUM_DIR) => {
                (entry.signal)(entry.all);
                true
            }
            // Removed, inaccessible or disconnected directory, or an abort that was not
            // requested: refresh everything once and stop this directory's watch.
            Some(_) => {
                (entry.signal)(entry.all);
                false
            }
        };
        if rearm && arm(entry).is_ok() {
            return;
        }
        if rearm {
            (entry.signal)(entry.all);
        }
        self.entries.remove(&key);
    }
}

fn arm(entry: &mut Entry) -> io::Result<()> {
    entry.overlapped = OVERLAPPED::default();
    // SAFETY: the boxed entry keeps the DWORD-aligned buffer and OVERLAPPED at stable
    // addresses until this read's completion is dequeued; only this thread uses them.
    if unsafe {
        ReadDirectoryChangesW(
            entry.directory.as_raw_handle(),
            entry.buffer.as_mut_ptr().cast(),
            size_of_val(&*entry.buffer) as u32,
            0,
            FILTER,
            ptr::null_mut(),
            &mut entry.overlapped,
            None,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    entry.pending = true;
    Ok(())
}

impl Entry {
    fn mask(&self, bytes: usize) -> u32 {
        records_mask(&self.buffer, bytes, self.bit, self.all, &self.children)
    }
}

/// Every record maps to the directory's own bit, plus the bit of a registered target
/// that is the reported immediate child. A malformed buffer refreshes everything.
fn records_mask(
    buffer: &[u32],
    bytes: usize,
    bit: u32,
    all: u32,
    children: &[(Vec<u16>, u32)],
) -> u32 {
    let capacity = size_of_val(buffer);
    let length = bytes.min(capacity);
    let base = buffer.as_ptr().cast::<u8>();
    let header = offset_of!(FILE_NOTIFY_INFORMATION, FileName);
    let mut mask = bit;
    let mut offset = 0usize;
    loop {
        if offset + header > length || offset + size_of::<FILE_NOTIFY_INFORMATION>() > capacity {
            return all;
        }
        // SAFETY: the whole record header lies inside the DWORD-aligned buffer and record
        // offsets are checked to stay DWORD aligned.
        let record = unsafe { &*base.add(offset).cast::<FILE_NOTIFY_INFORMATION>() };
        let name_bytes = record.FileNameLength as usize;
        if !name_bytes.is_multiple_of(2) || offset + header + name_bytes > length {
            return all;
        }
        // SAFETY: the counted name lies inside the buffer; its pointer derives from the
        // buffer base, not from the one-element FileName field.
        let name = unsafe {
            std::slice::from_raw_parts(base.add(offset + header).cast::<u16>(), name_bytes / 2)
        };
        for (child, child_bit) in children {
            if child.as_slice() == name {
                mask |= child_bit;
            }
        }
        let next = record.NextEntryOffset as usize;
        if next == 0 {
            return mask;
        }
        if !next.is_multiple_of(4) || next < header + name_bytes {
            return all;
        }
        offset += next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(records: &[(&str, u32)]) -> (Vec<u32>, usize) {
        let mut bytes = Vec::new();
        for (index, (name, next)) in records.iter().enumerate() {
            let units: Vec<u16> = name.encode_utf16().collect();
            let start = bytes.len();
            let next = if index + 1 == records.len() { 0 } else { *next };
            bytes.extend(next.to_le_bytes());
            bytes.extend(1u32.to_le_bytes());
            bytes.extend(((units.len() * 2) as u32).to_le_bytes());
            for unit in units {
                bytes.extend(unit.to_le_bytes());
            }
            if next != 0 {
                bytes.resize(start + next as usize, 0);
            }
        }
        let length = bytes.len();
        bytes.resize(BUFFER_UNITS * 4, 0);
        let words = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| u32::from_le_bytes(*chunk))
            .collect();
        (words, length)
    }

    #[test]
    fn records_map_to_the_directory_and_registered_immediate_children() {
        let children = vec![
            ("child".encode_utf16().collect(), 2),
            ("other".encode_utf16().collect(), 4),
        ];
        let (words, length) = buffer(&[("file.txt", 32), ("child", 24), ("CHILD", 0)]);
        assert_eq!(
            records_mask(&words, length, 1, 7, &children),
            3,
            "exact names only"
        );
        let (words, length) = buffer(&[("other", 0)]);
        assert_eq!(records_mask(&words, length, 1, 7, &children), 5);
    }

    #[test]
    fn malformed_records_refresh_every_target() {
        let (words, length) = buffer(&[("name", 0)]);
        for (bytes, label) in [(length - 1, "truncated name"), (8, "truncated header")] {
            assert_eq!(records_mask(&words, bytes, 1, 7, &[]), 7, "{label}");
        }
        for next in [6u32, 13] {
            let (mut words, length) = buffer(&[("name", 0)]);
            words[0] = next;
            assert_eq!(records_mask(&words, length, 1, 7, &[]), 7, "next {next}");
        }
        let (mut words, length) = buffer(&[("name", 0)]);
        words[2] = 3;
        assert_eq!(
            records_mask(&words, length, 1, 7, &[]),
            7,
            "odd name length"
        );
    }
}
