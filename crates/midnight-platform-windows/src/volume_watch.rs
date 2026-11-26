//! Drive-letter and media notifications for the drive list.
//!
//! One worker thread owns a hidden top-level window. Message-only windows receive no
//! broadcasts, so it is an ordinary top-level window that is never shown. Three
//! sources each turn into the same short hint:
//!
//! - `WM_DEVICECHANGE` with `DBT_DEVICEARRIVAL` or `DBT_DEVICEREMOVECOMPLETE` for a
//!   `DBT_DEVTYP_VOLUME`: the system broadcasts it to every top-level window when a
//!   volume gets or loses a drive letter (USB sticks, card media, mapped network
//!   drives, `subst`), after the letter is assigned or released.
//! - The shell's `SHCNE_DRIVEADD`, `SHCNE_DRIVEREMOVED`, `SHCNE_MEDIAINSERTED` and
//!   `SHCNE_MEDIAREMOVED`, which shell programs post for changes that send no
//!   broadcast.
//! - A safety net: every [`POLL`] the window compares `GetLogicalDrives`, one call
//!   that reads the session's drive bitmask without touching any drive, with the
//!   mask it last saw. The broadcast is sent to every top-level window in turn, so a
//!   slow window delays it: `subst` drives were announced 45 to 620 ms after the
//!   command, while the mask changes when the command returns. A letter defined
//!   without any broadcast (`DefineDosDevice` by another program) is noticed too.
//!
//! The hint never names a drive; the consumer lists the drives again.

use midnight_core::{FsError, WatchSubscription};
use std::{
    cell::RefCell,
    io, ptr,
    sync::{Arc, OnceLock, mpsc},
    thread::JoinHandle,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM},
    Storage::FileSystem::GetLogicalDrives,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Shell::{
            Common::ITEMIDLIST, ILFree, SHCNE_DRIVEADD, SHCNE_DRIVEREMOVED, SHCNE_MEDIAINSERTED,
            SHCNE_MEDIAREMOVED, SHCNRF_NewDelivery, SHCNRF_ShellLevel, SHChangeNotification_Lock,
            SHChangeNotification_Unlock, SHChangeNotifyDeregister, SHChangeNotifyEntry,
            SHChangeNotifyRegister, SHGetSpecialFolderLocation,
        },
        WindowsAndMessaging::{
            CreateWindowExW, DBT_DEVICEARRIVAL, DBT_DEVICEREMOVECOMPLETE, DBT_DEVTYP_VOLUME,
            DEV_BROADCAST_HDR, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
            KillTimer, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SetTimer,
            TranslateMessage, WM_APP, WM_CLOSE, WM_DESTROY, WM_DEVICECHANGE, WM_TIMER, WNDCLASSW,
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
        },
    },
};

/// The interval of the drive-mask comparison. Each tick is one system call that reads
/// no drive (about a microsecond) and calls back only when the mask changed.
pub(crate) const POLL: Duration = Duration::from_millis(250);
const SHELL_MESSAGE: u32 = WM_APP + 1;
const POLL_TIMER: usize = 1;
const CSIDL_DESKTOP: i32 = 0;

type Callback = Arc<dyn Fn() + Send + Sync>;

/// Where a hint came from; only diagnostics and tests tell them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Broadcast,
    Shell,
    Poll,
}

struct State {
    changed: Arc<dyn Fn(Source) + Send + Sync>,
    mask: u32,
    shell: u32,
}

thread_local! {
    /// The window procedure runs only on the worker thread that owns this state.
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// The live notification window. Dropping it closes the window and joins its thread,
/// so the callback is never called afterwards.
pub(crate) struct VolumeWatch {
    window: usize,
    thread: Option<JoinHandle<()>>,
}

impl WatchSubscription for VolumeWatch {}

impl Drop for VolumeWatch {
    fn drop(&mut self) {
        // SAFETY: posting to a window handle is safe even if it was destroyed meanwhile;
        // the thread exits after its window is destroyed.
        unsafe { PostMessageW(self.window as HWND, WM_CLOSE, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Start watching for drive changes. Each notification calls `changed` once.
pub(crate) fn watch(changed: Callback) -> Result<VolumeWatch, FsError> {
    watch_sources(Arc::new(move |_| changed()))
}

pub(crate) fn watch_sources(
    changed: Arc<dyn Fn(Source) + Send + Sync>,
) -> Result<VolumeWatch, FsError> {
    let (ready, started) = mpsc::sync_channel(1);
    let thread = std::thread::Builder::new()
        .name("midnight-volume-watch".into())
        .spawn(move || run(changed, ready))
        .map_err(failure)?;
    match started.recv() {
        Ok(Ok(window)) => Ok(VolumeWatch {
            window,
            thread: Some(thread),
        }),
        Ok(Err(error)) => {
            let _ = thread.join();
            Err(failure(error))
        }
        Err(_) => {
            let _ = thread.join();
            Err(failure(io::Error::other("The drive watch thread stopped.")))
        }
    }
}

fn failure(error: io::Error) -> FsError {
    FsError::from_io("watch drives", std::path::Path::new(""), error)
}

fn run(changed: Arc<dyn Fn(Source) + Send + Sync>, ready: mpsc::SyncSender<io::Result<usize>>) {
    let window = match create_window() {
        Ok(window) => window,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let shell = register_shell(window);
    STATE.with_borrow_mut(|state| {
        *state = Some(State {
            changed,
            mask: logical_drives(),
            shell,
        });
    });
    // SAFETY: the timer belongs to this thread's live window.
    if unsafe { SetTimer(window, POLL_TIMER, POLL.as_millis() as u32, None) } == 0 {
        let error = io::Error::last_os_error();
        // SAFETY: the window was created on this thread.
        unsafe { DestroyWindow(window) };
        let _ = ready.send(Err(error));
        return;
    }
    if ready.send(Ok(window as usize)).is_err() {
        // SAFETY: the window was created on this thread.
        unsafe { DestroyWindow(window) };
    }
    let mut message = MSG::default();
    // SAFETY: a standard message loop over this thread's queue.
    while unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    STATE.with_borrow_mut(|state| drop(state.take()));
}

fn create_window() -> io::Result<HWND> {
    static CLASS: OnceLock<Result<u16, i32>> = OnceLock::new();
    let class = windows_sys::w!("MidnightExplorerVolumeWatch");
    // SAFETY: the executable and its window procedure stay loaded.
    let instance = unsafe { GetModuleHandleW(ptr::null()) };
    CLASS
        .get_or_init(|| {
            let value = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance,
                lpszClassName: class,
                ..Default::default()
            };
            // SAFETY: the class structure and its static name outlive the call.
            let atom = unsafe { RegisterClassW(&value) };
            if atom == 0 {
                Err(io::Error::last_os_error().raw_os_error().unwrap_or(0))
            } else {
                Ok(atom)
            }
        })
        .map_err(io::Error::from_raw_os_error)?;
    // SAFETY: a hidden, never shown top-level window owned by this thread. It must be
    // top-level: message-only windows receive no WM_DEVICECHANGE broadcasts.
    let window = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            class,
            WS_POPUP,
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        )
    };
    if window.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(window)
}

/// Register the shell's drive and media notifications. The broadcast and the poll
/// still work without them, so a failure only leaves this source out.
fn register_shell(window: HWND) -> u32 {
    let mut desktop: *mut ITEMIDLIST = ptr::null_mut();
    // SAFETY: the out pointer is valid; the returned list is freed below.
    if unsafe { SHGetSpecialFolderLocation(ptr::null_mut(), CSIDL_DESKTOP, &mut desktop) } < 0
        || desktop.is_null()
    {
        return 0;
    }
    let entry = SHChangeNotifyEntry {
        pidl: desktop,
        fRecursive: 1,
    };
    // SAFETY: the entry array has one valid element; the shell copies it.
    let id = unsafe {
        SHChangeNotifyRegister(
            window,
            SHCNRF_ShellLevel | SHCNRF_NewDelivery,
            (SHCNE_DRIVEADD | SHCNE_DRIVEREMOVED | SHCNE_MEDIAINSERTED | SHCNE_MEDIAREMOVED) as i32,
            SHELL_MESSAGE,
            1,
            &entry,
        )
    };
    // SAFETY: the list came from the shell allocator above.
    unsafe { ILFree(desktop) };
    id
}

fn logical_drives() -> u32 {
    // SAFETY: GetLogicalDrives takes no pointers and returns the session's drive bitmask.
    unsafe { GetLogicalDrives() }
}

/// Whether a `WM_DEVICECHANGE` names a volume that arrived or was removed.
fn is_volume_change(event: WPARAM, header: *const DEV_BROADCAST_HDR) -> bool {
    let event = event as u32;
    if (event != DBT_DEVICEARRIVAL && event != DBT_DEVICEREMOVECOMPLETE) || header.is_null() {
        return false;
    }
    // SAFETY: for these two events the system passes a valid broadcast header.
    unsafe { (*header).dbch_devicetype == DBT_DEVTYP_VOLUME }
}

fn signal(source: Source) {
    let callback = STATE.with_borrow_mut(|state| {
        state.as_mut().and_then(|state| {
            let mask = logical_drives();
            let changed = mask != state.mask;
            state.mask = mask;
            (source != Source::Poll || changed).then(|| state.changed.clone())
        })
    });
    // Called outside the state borrow, so a callback cannot re-enter it.
    if let Some(callback) = callback {
        callback(source);
    }
}

unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_DEVICECHANGE => {
            if is_volume_change(wparam, lparam as *const DEV_BROADCAST_HDR) {
                signal(Source::Broadcast);
            }
            // TRUE grants any query; this window never vetoes a removal.
            1
        }
        SHELL_MESSAGE => {
            let mut items: *mut *mut ITEMIDLIST = ptr::null_mut();
            let mut event = 0;
            // SAFETY: new delivery passes a shared-memory handle and the sender's
            // process id; the lock must be released to free that memory.
            let lock = unsafe {
                SHChangeNotification_Lock(wparam as HANDLE, lparam as u32, &mut items, &mut event)
            };
            if !lock.is_null() {
                // SAFETY: the handle was just returned by SHChangeNotification_Lock.
                unsafe { SHChangeNotification_Unlock(lock) };
            }
            signal(Source::Shell);
            0
        }
        WM_TIMER if wparam == POLL_TIMER => {
            signal(Source::Poll);
            0
        }
        WM_CLOSE => {
            // SAFETY: the window belongs to this thread.
            unsafe { DestroyWindow(window) };
            0
        }
        WM_DESTROY => {
            let shell = STATE.with_borrow(|state| state.as_ref().map_or(0, |state| state.shell));
            if shell != 0 {
                // SAFETY: the id came from SHChangeNotifyRegister for this window.
                unsafe { SHChangeNotifyDeregister(shell) };
            }
            // SAFETY: the timer and window belong to this thread.
            unsafe {
                KillTimer(window, POLL_TIMER);
                PostQuitMessage(0);
            }
            0
        }
        // SAFETY: default processing for this thread's own window.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

#[cfg(test)]
#[path = "volume_watch_tests.rs"]
mod tests;
