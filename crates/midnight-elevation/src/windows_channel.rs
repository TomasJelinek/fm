//! Overlapped named-pipe I/O. Data wakes a waiter immediately; cancellation and the deadline
//! are observed within one bounded wait slice, and every abandoned operation is cancelled and
//! awaited before its buffer is released.
use midnight_core::Cancellation;
use std::{
    io::{self, Read, Write},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr, thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    },
    Storage::FileSystem::{ReadFile, WriteFile},
    System::{
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        Pipes::ConnectNamedPipe,
        Threading::{CreateEventW, ResetEvent, SetEvent, WaitForMultipleObjects},
    },
};

/// Bound on how long a cancelled or expired operation can remain unnoticed.
const CANCEL_POLL: Duration = Duration::from_millis(50);
/// Largest single transfer; frames are written in several operations.
const MAX_TRANSFER: usize = 1024 * 1024;

pub(super) fn millis(duration: Duration) -> u32 {
    // INFINITE is u32::MAX; every wait here is bounded.
    u32::try_from(duration.as_millis()).map_or(u32::MAX - 1, |value| value.min(u32::MAX - 1))
}

/// A manual-reset event for one class of overlapped operations.
pub(super) fn event() -> io::Result<OwnedHandle> {
    // SAFETY: an unnamed, non-inheritable event; ownership is taken only on success.
    let raw = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateEventW returned a new owned handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
}

fn aborted() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "Authorized listing cancelled",
    )
}
fn timed_out() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "Authorization helper timeout")
}

pub(super) struct Channel {
    pipe: OwnedHandle,
    event: OwnedHandle,
    cancel: Cancellation,
    deadline: Instant,
}

impl Channel {
    /// `pipe` must have been opened or created with `FILE_FLAG_OVERLAPPED`.
    pub(super) fn new(
        pipe: OwnedHandle,
        cancel: Cancellation,
        deadline: Instant,
    ) -> io::Result<Self> {
        Ok(Self {
            pipe,
            event: event()?,
            cancel,
            deadline,
        })
    }

    pub(super) fn pipe(&self) -> &OwnedHandle {
        &self.pipe
    }

    fn check(&self) -> io::Result<()> {
        if self.cancel.is_cancelled() {
            return Err(aborted());
        }
        if Instant::now() >= self.deadline {
            return Err(timed_out());
        }
        Ok(())
    }

    fn overlapped(&self) -> io::Result<OVERLAPPED> {
        // SAFETY: the owned event is live; a reset only clears a previous completion.
        if unsafe { ResetEvent(self.event.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(OVERLAPPED {
            hEvent: self.event.as_raw_handle(),
            ..Default::default()
        })
    }

    /// Wait for the server end to be connected. `peer` is the launched helper process; its exit
    /// ends the wait, as do cancellation and the deadline.
    pub(super) fn connect(&mut self, peer: HANDLE) -> io::Result<()> {
        self.check()?;
        let mut overlapped = self.overlapped()?;
        // SAFETY: the pipe and event are live; `overlapped` outlives the operation because
        // `finish` waits for completion or cancels and awaits it.
        let started = unsafe { ConnectNamedPipe(self.pipe.as_raw_handle(), &mut overlapped) };
        if started == 0
            && io::Error::last_os_error().raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32)
        {
            return Ok(());
        }
        self.finish(&mut overlapped, started, Some(peer)).map(drop)
    }

    /// Complete an operation that was just started, waiting in bounded slices.
    fn finish(
        &self,
        overlapped: &mut OVERLAPPED,
        started: i32,
        peer: Option<HANDLE>,
    ) -> io::Result<u32> {
        if started == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
                return Err(error);
            }
            let handles = [self.event.as_raw_handle(), peer.unwrap_or(ptr::null_mut())];
            let count = if peer.is_some() { 2 } else { 1 };
            loop {
                if let Err(error) = self.check() {
                    return Err(self.abandon(overlapped, error));
                }
                let slice = self
                    .deadline
                    .saturating_duration_since(Instant::now())
                    .min(CANCEL_POLL);
                // SAFETY: every handle in the first `count` entries is live for this call.
                match unsafe { WaitForMultipleObjects(count, handles.as_ptr(), 0, millis(slice)) } {
                    WAIT_OBJECT_0 => break,
                    WAIT_TIMEOUT => {}
                    signalled if peer.is_some() && signalled == WAIT_OBJECT_0 + 1 => {
                        let error = io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "Authorization helper ended before connecting",
                        );
                        return Err(self.abandon(overlapped, error));
                    }
                    _ => {
                        let error = io::Error::last_os_error();
                        return Err(self.abandon(overlapped, error));
                    }
                }
            }
        }
        let mut count = 0;
        // SAFETY: the operation completed; the OVERLAPPED structure is still live.
        if unsafe { GetOverlappedResult(self.pipe.as_raw_handle(), overlapped, &mut count, 0) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(count)
    }

    /// Cancel a pending operation and wait until the kernel no longer uses its buffers.
    fn abandon(&self, overlapped: &OVERLAPPED, error: io::Error) -> io::Error {
        let mut count = 0;
        // SAFETY: cancelling an operation on our live pipe, then waiting for its completion so
        // that the caller's buffer and OVERLAPPED may be released.
        unsafe {
            CancelIoEx(self.pipe.as_raw_handle(), overlapped);
            GetOverlappedResult(self.pipe.as_raw_handle(), overlapped, &mut count, 1);
        }
        error
    }
}

impl Read for Channel {
    fn read(&mut self, data: &mut [u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        self.check()?;
        let mut overlapped = self.overlapped()?;
        let length = data.len().min(MAX_TRANSFER) as u32;
        // SAFETY: `data` and `overlapped` stay live until `finish` completes or awaits the read.
        let started = unsafe {
            ReadFile(
                self.pipe.as_raw_handle(),
                data.as_mut_ptr(),
                length,
                ptr::null_mut(),
                &mut overlapped,
            )
        };
        match self.finish(&mut overlapped, started, None) {
            Ok(count) => Ok(count as usize),
            // The peer closed after its last byte; buffered data was returned first.
            Err(error) if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) => Ok(0),
            Err(error) => Err(error),
        }
    }
}

impl Write for Channel {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        self.check()?;
        let mut overlapped = self.overlapped()?;
        let length = data.len().min(MAX_TRANSFER) as u32;
        // SAFETY: `data` and `overlapped` stay live until `finish` completes or awaits the write.
        let started = unsafe {
            WriteFile(
                self.pipe.as_raw_handle(),
                data.as_ptr(),
                length,
                ptr::null_mut(),
                &mut overlapped,
            )
        };
        self.finish(&mut overlapped, started, None)
            .map(|count| count as usize)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Ends the dedicated helper process when its requester exits, closes the pipe, sends any
/// byte after its single request, or the deadline passes, even while a native directory read
/// is blocked. A pending one-byte read observes the pipe; no polling is involved.
pub(super) struct Monitor {
    stop: OwnedHandle,
    thread: Option<thread::JoinHandle<()>>,
}

impl Monitor {
    pub(super) fn start(
        pipe: &OwnedHandle,
        requester: OwnedHandle,
        deadline: Instant,
        cancel: Cancellation,
        terminate: impl FnOnce() + Send + 'static,
    ) -> io::Result<Self> {
        let stop = event()?;
        let stopped = stop.try_clone()?;
        let pipe = pipe.try_clone()?;
        let read = event()?;
        let thread = thread::Builder::new()
            .name("authorized-listing-peer".into())
            .spawn(move || {
                if watch(&pipe, &read, &requester, &stopped, deadline) {
                    cancel.cancel();
                    // This process owns only directory reads and the private channel. End it
                    // even if a native filesystem call is stuck and cannot poll cancellation.
                    terminate();
                }
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    /// Stop watching before the reply is written: afterwards the requester may close at once.
    pub(super) fn stop(mut self) {
        self.finish();
    }

    fn finish(&mut self) {
        // SAFETY: the owned event is live; signalling it only wakes the watcher.
        unsafe {
            SetEvent(self.stop.as_raw_handle());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Returns whether the requester violated the channel or disappeared.
fn watch(
    pipe: &OwnedHandle,
    read: &OwnedHandle,
    requester: &OwnedHandle,
    stop: &OwnedHandle,
    deadline: Instant,
) -> bool {
    let mut overlapped = OVERLAPPED {
        hEvent: read.as_raw_handle(),
        ..Default::default()
    };
    let mut byte = 0u8;
    // SAFETY: `byte` and `overlapped` stay live until the read completes or is cancelled and
    // awaited below.
    let started = unsafe {
        ReadFile(
            pipe.as_raw_handle(),
            &mut byte,
            1,
            ptr::null_mut(),
            &mut overlapped,
        )
    };
    if started != 0 || io::Error::last_os_error().raw_os_error() != Some(ERROR_IO_PENDING as i32) {
        // An unexpected byte, or the pipe is already broken.
        return true;
    }
    let handles = [
        read.as_raw_handle(),
        requester.as_raw_handle(),
        stop.as_raw_handle(),
    ];
    let remaining = deadline.saturating_duration_since(Instant::now());
    // SAFETY: all three handles are live for this bounded wait.
    let woke = unsafe { WaitForMultipleObjects(3, handles.as_ptr(), 0, millis(remaining)) };
    if woke != WAIT_OBJECT_0 {
        let mut count = 0;
        // SAFETY: cancel our own pending read and wait until it no longer uses `byte`.
        unsafe {
            CancelIoEx(pipe.as_raw_handle(), &overlapped);
            GetOverlappedResult(pipe.as_raw_handle(), &overlapped, &mut count, 1);
        }
    }
    woke != WAIT_OBJECT_0 + 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use windows_sys::Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
            PIPE_ACCESS_DUPLEX,
        },
        System::{
            Pipes::{CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT},
            Threading::{GetCurrentProcessId, OpenProcess, PROCESS_SYNCHRONIZE},
        },
    };

    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn owned(raw: HANDLE) -> OwnedHandle {
        assert!(
            !raw.is_null() && raw != INVALID_HANDLE_VALUE,
            "{}",
            io::Error::last_os_error()
        );
        // SAFETY: a successful creation call returned this owned handle.
        unsafe { OwnedHandle::from_raw_handle(raw) }
    }

    /// Server and overlapped client ends of a fresh private pipe.
    fn pipe_pair() -> (OwnedHandle, OwnedHandle) {
        let name: Vec<u16> = format!(
            r"\\.\pipe\MidnightExplorer.MonitorTest.{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )
        .encode_utf16()
        .chain([0])
        .collect();
        // SAFETY: a terminated unique name and default security; ownership checked above.
        let server = owned(unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                4096,
                4096,
                0,
                ptr::null(),
            )
        });
        // SAFETY: the pipe exists; a client may connect before ConnectNamedPipe.
        let client = owned(unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                ptr::null_mut(),
            )
        });
        (server, client)
    }

    fn requester() -> OwnedHandle {
        // SAFETY: a wait handle to this test process, which outlives every monitor.
        owned(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, GetCurrentProcessId()) })
    }

    #[test]
    fn closed_requester_pipe_ends_the_helper_and_stop_does_not() {
        let (server, client) = pipe_pair();
        let cancel = Cancellation::default();
        let (ended, endings) = mpsc::channel();
        let monitor = Monitor::start(
            &client,
            requester(),
            Instant::now() + Duration::from_secs(60),
            cancel.clone(),
            move || ended.send(()).unwrap(),
        )
        .unwrap();
        // The requester closing its end must wake the watcher long before the deadline.
        drop(server);
        endings.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(cancel.is_cancelled());
        monitor.stop();

        let (server, client) = pipe_pair();
        let cancel = Cancellation::default();
        let (ended, endings) = mpsc::channel::<()>();
        let monitor = Monitor::start(
            &client,
            requester(),
            Instant::now() + Duration::from_secs(60),
            cancel.clone(),
            move || ended.send(()).unwrap(),
        )
        .unwrap();
        monitor.stop();
        // Stopping joins the watcher; it neither terminated nor cancelled the listing.
        assert!(endings.try_recv().is_err());
        assert!(!cancel.is_cancelled());
        drop(server);
    }

    #[test]
    fn unexpected_requester_bytes_end_the_helper() {
        let (server, client) = pipe_pair();
        let cancel = Cancellation::default();
        let (ended, endings) = mpsc::channel();
        let _monitor = Monitor::start(
            &client,
            requester(),
            Instant::now() + Duration::from_secs(60),
            cancel.clone(),
            move || ended.send(()).unwrap(),
        )
        .unwrap();
        let mut requester_end = Channel::new(
            server,
            Cancellation::default(),
            Instant::now() + Duration::from_secs(10),
        )
        .unwrap();
        let own = requester();
        requester_end.connect(own.as_raw_handle()).unwrap();
        requester_end.write_all(b"second request").unwrap();
        endings.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(cancel.is_cancelled());
    }
}
