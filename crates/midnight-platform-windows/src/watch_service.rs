//! Owned `ReadDirectoryChangesW` service: one thread blocked on one I/O completion port
//! serves every subscription, so idle watches cause no wakeups. The thread issues every
//! read and owns every buffer and OVERLAPPED; subscriptions only post commands.
//!
//! A completion maps to the registration's own bit, plus the bit of any registered
//! target that is the reported immediate child. A zero-byte completion (the 64 KiB
//! buffer overflowed), `ERROR_NOTIFY_ENUM_DIR` and every other error signal all bits,
//! as the core watch contract requires. Overflow re-arms; any other error stops that
//! directory's watch (it was removed or became inaccessible) after signalling.
//!
//! Consumer callbacks run on the service thread. A panicking callback counts as
//! delivered and never unwinds that thread. Should the thread still end (a broken port
//! or a panic in the service itself), it leaks every entry whose read is pending instead
//! of freeing memory the kernel may still write, and fails queued and later
//! registrations instead of leaving them waiting.
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
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc, Mutex, MutexGuard, OnceLock, PoisonError,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
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
/// Arming never blocks the service thread, but consumer callbacks run on it. Should one
/// block (a consumer bug), registration fails after this bound instead of blocking the
/// registering backend worker forever.
const REGISTER_TIMEOUT: Duration = Duration::from_secs(10);

fn stopped() -> io::Error {
    io::Error::other("the native watch service stopped")
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Delivers one mask. A panicking consumer callback counts as delivered: it must not
/// unwind the service thread, which owns every pending read.
fn deliver(signal: &Signal, mask: u32) {
    let _ = catch_unwind(AssertUnwindSafe(|| signal(mask)));
}

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
        // A lost wakeup leaves the command queued for the next one; the owner already
        // disabled its callback, so a late unregistration only delays freeing the reads.
        let _ = self.service.post(Command::Unregister(self.id));
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
        let service = Self::detached()?;
        std::thread::Builder::new()
            .name("midnight-native-watch".into())
            .spawn(move || Worker::new(service).run())?;
        Ok(service)
    }

    /// A service whose worker the caller still has to run. Services live as long as
    /// the process; production has exactly one.
    fn detached() -> io::Result<&'static Self> {
        // SAFETY: creates a new completion port; no file handle is associated yet.
        let port = unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, ptr::null_mut(), 0, 1) };
        if port.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a successful call returned one owned port handle.
        let port = unsafe { OwnedHandle::from_raw_handle(port) };
        Ok(Box::leak(Box::new(Self {
            port,
            commands: Mutex::new(Some(Vec::new())),
            next: AtomicU64::new(1),
        })))
    }

    /// A separate service with its own thread, so a test that stalls its thread cannot
    /// delay the subscriptions of other tests.
    #[cfg(test)]
    pub(crate) fn private() -> io::Result<&'static Self> {
        Self::start()
    }

    /// Opens every directory on the calling backend worker (a slow share blocks only
    /// that worker), then hands the handles to the service thread, which arms them.
    pub(crate) fn register(
        &'static self,
        targets: Arc<[PathBuf]>,
        signal: Signal,
    ) -> io::Result<Registration> {
        self.register_within(targets, signal, REGISTER_TIMEOUT)
    }

    fn register_within(
        &'static self,
        targets: Arc<[PathBuf]>,
        signal: Signal,
        timeout: Duration,
    ) -> io::Result<Registration> {
        // Target bits are one u32 mask; the service thread must never compute an
        // out-of-range shift (`watch.rs` validates the same bound for its callers).
        if targets.is_empty() || targets.len() > 32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a native watch has 1..=32 target directories",
            ));
        }
        let directories = targets
            .iter()
            .map(|path| open_directory(path))
            .collect::<io::Result<Vec<_>>>()?;
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (reply, result) = mpsc::sync_channel(1);
        let posted = self.post(Command::Register {
            id,
            directories,
            targets,
            signal,
            reply,
        });
        if let Err(error) = posted {
            // No wakeup was queued. Withdraw the registration unless a wakeup for
            // another reason already handed it to the service thread.
            if self.withdraw(id) {
                return Err(error);
            }
        }
        match result.recv_timeout(timeout) {
            Ok(armed) => armed?,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(stopped()),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // No Registration owns it: unarm it whenever the thread gets to it.
                let _ = self.post(Command::Unregister(id));
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the native watch service did not respond",
                ));
            }
        }
        Ok(Registration { id, service: self })
    }

    /// Queues `command` and wakes the service thread. When the wakeup cannot be posted,
    /// the command stays queued and runs at the thread's next wakeup (it drains the
    /// queue on every wakeup, read completions included); the error is returned.
    fn post(&self, command: Command) -> io::Result<()> {
        let mut commands = lock(&self.commands);
        let Some(queue) = commands.as_mut() else {
            if let Command::Register { reply, .. } = command {
                let _ = reply.send(Err(stopped()));
            }
            return Ok(());
        };
        queue.push(command);
        drop(commands);
        // SAFETY: a live port; a null OVERLAPPED with the command key only wakes the
        // service thread.
        if unsafe {
            PostQueuedCompletionStatus(self.port.as_raw_handle(), 0, COMMAND_KEY, ptr::null())
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Removes a still queued registration; false once the service thread took it.
    fn withdraw(&self, id: u64) -> bool {
        let mut commands = lock(&self.commands);
        let Some(queue) = commands.as_mut() else {
            return false;
        };
        let queued = queue.iter().position(
            |command| matches!(command, Command::Register { id: queued, .. } if *queued == id),
        );
        queued.map(|index| queue.remove(index)).is_some()
    }
}

fn open_directory(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED)
        .open(path)
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

struct Worker {
    service: &'static Service,
    entries: HashMap<usize, Box<Entry>>,
    next_key: usize,
}

/// Runs when the service thread ends: a broken port, or a panic unwinding the thread.
impl Drop for Worker {
    fn drop(&mut self) {
        // Later registrations fail at once instead of waiting for a reply nobody sends;
        // queued ones fail now.
        let queued = lock(&self.service.commands).take().unwrap_or_default();
        for command in queued {
            if let Command::Register { reply, .. } = command {
                let _ = reply.send(Err(stopped()));
            }
        }
        // A pending read still owns its OVERLAPPED and buffer, and the kernel may complete
        // it at any time. Leak such entries, handles included, rather than free memory
        // under a pending read; a stopped service never restarts.
        for (_, entry) in self.entries.drain() {
            if entry.pending {
                std::mem::forget(entry);
            }
        }
    }
}

impl Worker {
    fn new(service: &'static Service) -> Self {
        Self {
            service,
            entries: HashMap::new(),
            next_key: 0,
        }
    }

    fn run(mut self) {
        loop {
            let mut bytes = 0u32;
            let mut key = 0usize;
            let mut overlapped: *mut OVERLAPPED = ptr::null_mut();
            // SAFETY: a live port and writable outputs; INFINITE blocks without wakeups.
            let completed = unsafe {
                GetQueuedCompletionStatus(
                    self.service.port.as_raw_handle(),
                    &mut bytes,
                    &mut key,
                    &mut overlapped,
                    INFINITE,
                )
            } != 0;
            if !overlapped.is_null() {
                // SAFETY: reads the calling thread's error for the failed dequeued read.
                let error = (!completed).then(|| unsafe { GetLastError() });
                self.completed(key, bytes, error);
            } else if !completed {
                // Only a broken port fails without dequeuing anything: stop serving
                // instead of spinning; dropping the worker fails every registration.
                return;
            }
            // Every wakeup drains the queue, so a command whose own wakeup could not be
            // posted still runs at the next completion.
            self.drain();
        }
    }

    fn drain(&mut self) {
        let commands = lock(&self.service.commands)
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        for command in commands {
            self.command(command);
        }
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Register {
                id,
                directories,
                targets,
                signal,
                reply,
            } => {
                let result = self.register(id, directories, &targets, signal);
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
                    self.service.port.as_raw_handle(),
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
                deliver(&entry.signal, entry.all);
                true
            }
            None => {
                deliver(&entry.signal, entry.mask(bytes as usize));
                true
            }
            Some(ERROR_NOTIFY_ENUM_DIR) => {
                deliver(&entry.signal, entry.all);
                true
            }
            // Removed, inaccessible or disconnected directory, or an abort that was not
            // requested: refresh everything once and stop this directory's watch.
            Some(_) => {
                deliver(&entry.signal, entry.all);
                false
            }
        };
        if rearm && arm(entry).is_ok() {
            return;
        }
        if rearm {
            deliver(&entry.signal, entry.all);
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
    use crate::test_fixtures::build_temp;
    use std::{fs, sync::atomic::AtomicBool};

    const DEADLINE: Duration = Duration::from_secs(10);

    fn targets(path: &Path) -> Arc<[PathBuf]> {
        Arc::from([path.to_path_buf()])
    }

    /// Sends once when dropped: proves that every owner released a signal closure.
    struct DropNotice(mpsc::SyncSender<()>);
    impl Drop for DropNotice {
        fn drop(&mut self) {
            let _ = self.0.try_send(());
        }
    }

    /// Before: a panicking callback unwound the service thread, which freed every
    /// pending read's buffer and OVERLAPPED and left later registrations waiting forever.
    #[test]
    fn a_panicking_callback_counts_as_delivered_and_the_service_keeps_watching() {
        let service = Service::private().unwrap();
        let directory = build_temp("watch-panic-");
        let (sender, receiver) = mpsc::sync_channel(16);
        let panicked = Arc::new(AtomicBool::new(false));
        let first = panicked.clone();
        let _registration = service
            .register(
                targets(directory.path()),
                Arc::new(move |mask| {
                    // Set the flag before the receiver can observe this call.
                    let first_call = !first.swap(true, Ordering::SeqCst);
                    let _ = sender.try_send(mask);
                    if first_call {
                        panic!("deliberate consumer callback panic");
                    }
                }),
            )
            .unwrap();
        fs::write(directory.path().join("first"), b"first").unwrap();
        receiver.recv_timeout(DEADLINE).unwrap();
        assert!(panicked.load(Ordering::SeqCst));
        receiver.try_iter().for_each(drop);
        fs::write(directory.path().join("second"), b"second").unwrap();
        assert_eq!(receiver.recv_timeout(DEADLINE).unwrap() & 1, 1);
        let other = build_temp("watch-panic-other-");
        drop(
            service
                .register(targets(other.path()), Arc::new(|_| {}))
                .expect("the service thread survived the panic"),
        );
    }

    #[test]
    fn an_unwinding_worker_leaks_pending_reads_and_fails_registrations() {
        let service = Service::detached().unwrap();
        let directory = build_temp("watch-unwind-");
        let (dropped, notice) = mpsc::sync_channel(1);
        let guard = DropNotice(dropped);
        let signal: Signal = Arc::new(move |_| {
            let _ = &guard;
        });
        let mut worker = Worker::new(service);
        worker
            .register(
                1,
                vec![open_directory(directory.path()).unwrap()],
                &[directory.path().to_path_buf()],
                signal,
            )
            .unwrap();
        let (reply, queued) = mpsc::sync_channel(1);
        service
            .post(Command::Register {
                id: 2,
                directories: Vec::new(),
                targets: Arc::from([]),
                signal: Arc::new(|_| {}),
                reply,
            })
            .unwrap();
        let unwound = catch_unwind(AssertUnwindSafe(move || {
            let _worker = worker;
            panic!("deliberate service thread failure");
        }));
        assert!(unwound.is_err());
        // The entry with a pending read (and its signal) was leaked, never freed.
        assert!(notice.try_recv().is_err());
        assert!(queued.recv_timeout(DEADLINE).unwrap().is_err());
        let later = service.register(targets(directory.path()), Arc::new(|_| {}));
        assert!(later.err().unwrap().to_string().contains("stopped"));
        // The kernel may still complete the leaked read into its intact buffer.
        fs::write(directory.path().join("after"), b"after").unwrap();
    }

    #[test]
    fn a_registration_times_out_while_a_callback_blocks_and_is_unarmed_later() {
        let service = Service::private().unwrap();
        let blocking = build_temp("watch-blocking-");
        let (entered, entered_receiver) = mpsc::sync_channel(1);
        let (release, release_receiver) = mpsc::sync_channel::<()>(1);
        let release_receiver = Mutex::new(release_receiver);
        let held = Arc::new(AtomicBool::new(true));
        let _blocking = service
            .register(
                targets(blocking.path()),
                Arc::new(move |_| {
                    if held.swap(false, Ordering::SeqCst) {
                        let _ = entered.send(());
                        let _ = lock(&release_receiver).recv_timeout(DEADLINE);
                    }
                }),
            )
            .unwrap();
        fs::write(blocking.path().join("hold"), b"hold").unwrap();
        entered_receiver.recv_timeout(DEADLINE).unwrap();
        let late = build_temp("watch-late-");
        let (dropped, notice) = mpsc::sync_channel(1);
        let guard = DropNotice(dropped);
        let error = service
            .register_within(
                targets(late.path()),
                Arc::new(move |_| {
                    let _ = &guard;
                }),
                Duration::from_millis(50),
            )
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        release.send(()).unwrap();
        // The thread arms the late registration, then unarms it: its signal is freed.
        notice.recv_timeout(DEADLINE).unwrap();
        let after = build_temp("watch-after-");
        drop(
            service
                .register(targets(after.path()), Arc::new(|_| {}))
                .unwrap(),
        );
    }

    #[test]
    fn a_command_whose_wakeup_was_lost_runs_at_the_next_completion() {
        let service = Service::private().unwrap();
        let watched = build_temp("watch-wakeup-");
        let (sender, receiver) = mpsc::sync_channel(16);
        let _watched = service
            .register(
                targets(watched.path()),
                Arc::new(move |mask| {
                    let _ = sender.try_send(mask);
                }),
            )
            .unwrap();
        // Queue a registration without its wakeup, as after a failed post.
        let other = build_temp("watch-wakeup-other-");
        let id = service.next.fetch_add(1, Ordering::Relaxed);
        let (reply, armed) = mpsc::sync_channel(1);
        lock(&service.commands)
            .as_mut()
            .unwrap()
            .push(Command::Register {
                id,
                directories: vec![open_directory(other.path()).unwrap()],
                targets: targets(other.path()),
                signal: Arc::new(|_| {}),
                reply,
            });
        fs::write(watched.path().join("change"), b"change").unwrap();
        receiver.recv_timeout(DEADLINE).unwrap();
        armed.recv_timeout(DEADLINE).unwrap().unwrap();
        drop(Registration { id, service });
    }

    #[test]
    fn only_a_still_queued_registration_can_be_withdrawn() {
        let service = Service::detached().unwrap();
        let (reply, result) = mpsc::sync_channel(1);
        lock(&service.commands)
            .as_mut()
            .unwrap()
            .push(Command::Register {
                id: 7,
                directories: Vec::new(),
                targets: Arc::from([]),
                signal: Arc::new(|_| {}),
                reply,
            });
        assert!(!service.withdraw(8));
        assert!(service.withdraw(7));
        assert!(!service.withdraw(7));
        assert!(
            result.try_recv().is_err(),
            "a withdrawn registration has no reply"
        );
        assert!(lock(&service.commands).as_ref().unwrap().is_empty());
    }

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
