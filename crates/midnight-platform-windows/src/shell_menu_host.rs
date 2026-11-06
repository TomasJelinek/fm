//! A bounded process-lifetime STA keeps Shell windows and COM callbacks alive.

use super::*;
use std::{
    cell::{Cell, RefCell},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
};
use windows::{
    Win32::{
        Foundation::{LPARAM, LRESULT, WPARAM},
        UI::Shell::{IContextMenu2, IContextMenu3},
    },
    core::Interface,
};
use windows_sys::Win32::{
    Foundation::RECT,
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentProcessId},
    UI::WindowsAndMessaging::{
        CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
        GWLP_USERDATA, GetForegroundWindow, GetMessageW, GetWindowLongPtrW, GetWindowRect,
        GetWindowThreadProcessId, IsWindow, MSG, PostMessageW, RegisterClassW, SWP_NOACTIVATE,
        SWP_NOZORDER, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, TPM_RETURNCMD,
        TPM_RIGHTBUTTON, TrackPopupMenuEx, TranslateMessage, WM_APP, WM_DRAWITEM, WM_INITMENUPOPUP,
        WM_MEASUREITEM, WM_MENUCHAR, WM_NCCREATE, WM_NCDESTROY, WM_NULL, WNDCLASSW,
        WS_EX_TOOLWINDOW, WS_POPUP,
    },
};

const SHOW_MENU: u32 = WM_APP + 17;
static WORKER: OnceLock<Result<Worker, String>> = OnceLock::new();

struct Job {
    request: ShellMenuRequest,
    reply: SyncSender<Result<ShellMenuOutcome, FsError>>,
}

struct Worker {
    sender: SyncSender<Job>,
    window: usize,
    busy: AtomicBool,
}

struct BusyGuard<'a>(&'a AtomicBool);
impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(super) fn show(request: &ShellMenuRequest) -> Result<ShellMenuOutcome, FsError> {
    let worker = WORKER
        .get_or_init(Worker::start)
        .as_ref()
        .map_err(|error| failure(&request.paths[0], error.clone()))?;
    worker
        .busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| failure(&request.paths[0], "A system menu is already open."))?;
    let _busy = BusyGuard(&worker.busy);
    let (reply, receiver) = mpsc::sync_channel(1);
    worker
        .sender
        .try_send(Job {
            request: request.clone(),
            reply,
        })
        .map_err(|_| failure(&request.paths[0], "The native menu queue is unavailable."))?;
    // SAFETY: the native worker owns this permanent HWND. Only scalar wake messages cross threads.
    if unsafe { PostMessageW(worker.window as RawWindow, SHOW_MENU, 0, 0) } == 0 {
        return Err(failure(&request.paths[0], io::Error::last_os_error()));
    }
    receiver
        .recv()
        .map_err(|_| failure(&request.paths[0], "The native menu worker stopped."))?
}

impl Worker {
    fn start() -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (ready, started) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("midnight-shell-menu".into())
            .spawn(move || run(receiver, ready))
            .map_err(|error| error.to_string())?;
        let window = started
            .recv()
            .map_err(|_| "The Shell menu worker failed to start.".to_owned())??;
        Ok(Self {
            sender,
            window,
            busy: AtomicBool::new(false),
        })
    }
}

#[derive(Clone)]
struct MenuMessages {
    third: Option<IContextMenu3>,
    second: Option<IContextMenu2>,
}

struct Host {
    receiver: Receiver<Job>,
    window: Cell<RawWindow>,
    active: RefCell<Option<MenuMessages>>,
}

struct ActiveMenu<'a>(&'a RefCell<Option<MenuMessages>>);
impl Drop for ActiveMenu<'_> {
    fn drop(&mut self) {
        self.0.replace(None);
    }
}

struct ForegroundRestore {
    application: RawWindow,
    helper: RawWindow,
}

impl Drop for ForegroundRestore {
    fn drop(&mut self) {
        // SAFETY: restore only our own foreground helper, never steal focus from an
        // extension dialog or another application the user activated while tracking.
        unsafe {
            if !self.application.is_null()
                && IsWindow(self.application) != 0
                && GetForegroundWindow() == self.helper
            {
                SetForegroundWindow(self.application);
            }
        }
    }
}

fn run(receiver: Receiver<Job>, ready: SyncSender<Result<usize, String>>) {
    let Some(_com) = ComGuard::initialize(COINIT_APARTMENTTHREADED as u32) else {
        let _ = ready.send(Err("The system menu requires a Windows COM STA.".into()));
        return;
    };
    let host = Box::new(Host {
        receiver,
        window: Cell::new(ptr::null_mut()),
        active: RefCell::new(None),
    });
    let class: Vec<u16> = "MidnightExplorerShellMenuOwner\0".encode_utf16().collect();
    // SAFETY: null selects the current executable, which remains loaded for this process.
    let instance = unsafe { GetModuleHandleW(ptr::null()) };
    let window_class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        ..Default::default()
    };
    // SAFETY: all class fields point to live process-lifetime code or a copied class name.
    if unsafe { RegisterClassW(&window_class) } == 0 {
        let _ = ready.send(Err(io::Error::last_os_error().to_string()));
        return;
    }
    // SAFETY: the boxed Host address remains stable until after DestroyWindow. This is a
    // hidden top-level owner, not a message-only window; Shell dialogs require an owner.
    let window = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            0,
            0,
            640,
            480,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            (&*host as *const Host).cast(),
        )
    };
    if window.is_null() {
        let _ = ready.send(Err(io::Error::last_os_error().to_string()));
        return;
    }
    host.window.set(window);
    if ready.send(Ok(window as usize)).is_err() {
        // SAFETY: the owner is still live and belongs to this thread.
        unsafe { DestroyWindow(window) };
        return;
    }
    let mut message = MSG::default();
    // SAFETY: this STA owns every dispatched window and message storage remains valid.
    unsafe {
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        DestroyWindow(window);
    }
}

impl Host {
    fn display(&self, request: &ShellMenuRequest) -> Result<ShellMenuOutcome, FsError> {
        let window = self.window.get();
        let app_owner = request.owner as RawWindow;
        if !app_owner.is_null() {
            let mut process = 0;
            // SAFETY: Win32 validates this borrowed scalar window handle. No foreign
            // process may become our owner, and its identity is checked again per request.
            if unsafe { IsWindow(app_owner) } == 0
                || unsafe { GetWindowThreadProcessId(app_owner, &mut process) } == 0
                || process != unsafe { GetCurrentProcessId() }
            {
                return Err(failure(
                    &request.paths[0],
                    "The menu's application window is no longer valid.",
                ));
            }
        }
        // SAFETY: the popup belongs to this thread. Match the app's bounds for Shell
        // dialogs, but keep it independently owned: cross-thread ownership implicitly
        // attaches input queues and would let a blocked extension freeze the main UI.
        unsafe {
            if !app_owner.is_null() {
                let mut bounds = RECT::default();
                if GetWindowRect(app_owner, &mut bounds) != 0 {
                    SetWindowPos(
                        window,
                        ptr::null_mut(),
                        bounds.left,
                        bounds.top,
                        bounds.right.saturating_sub(bounds.left),
                        bounds.bottom.saturating_sub(bounds.top),
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    );
                }
            }
        }
        let menu = bind_context_menu(window, request)?;
        let popup = PopupMenu::new()?;
        self.active.replace(Some(MenuMessages {
            third: menu.cast().ok(),
            second: menu.cast().ok(),
        }));
        let active = ActiveMenu(&self.active);
        let count = query_menu(&menu, &popup, request.modifiers)?;
        let _restore = ForegroundRestore {
            application: app_owner,
            helper: window,
        };
        // SAFETY: the caller requested this menu in the foreground app. The independent
        // helper receives menu input without attaching its queue to the Slint UI thread.
        unsafe { SetForegroundWindow(window) };
        // SAFETY: tracking pumps this thread's window messages. Interfaces and HMENU stay
        // live through tracking and invocation; no COM object is marshaled to another thread.
        let command = unsafe {
            TrackPopupMenuEx(
                popup.0,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                request.position.x,
                request.position.y,
                window,
                ptr::null(),
            )
        } as u32;
        // SAFETY: the documented popup completion wake targets our live owner window.
        unsafe { PostMessageW(window, WM_NULL, 0, 0) };
        drop(active);
        let Some(offset) = command_offset(command, count) else {
            return Ok(ShellMenuOutcome::Dismissed);
        };
        invoke_command(&menu, window, request, offset)
    }
}

unsafe extern "system" fn window_proc(
    window: RawWindow,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    if message == WM_NCCREATE {
        // SAFETY: Windows provides CREATESTRUCTW during WM_NCCREATE, carrying our boxed Host.
        let create = unsafe { &*(lparam as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize) };
    }
    // SAFETY: the box lives until after window destruction; pointer is cleared at NCDESTROY.
    let state = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *const Host;
    if !state.is_null() {
        let host = unsafe { &*state };
        if message == SHOW_MENU {
            if let Ok(job) = host.receiver.try_recv() {
                // Rust panics must not unwind through the native window procedure.
                let result = catch_unwind(AssertUnwindSafe(|| host.display(&job.request)))
                    .unwrap_or_else(|_| {
                        Err(failure(
                            &job.request.paths[0],
                            "The native menu handler failed.",
                        ))
                    });
                let _ = job.reply.send(result);
            }
            return 0;
        }
        let is_menu_message = matches!(message, WM_INITMENUPOPUP | WM_MENUCHAR)
            || (wparam == 0 && matches!(message, WM_DRAWITEM | WM_MEASUREITEM));
        let active = if is_menu_message {
            host.active.borrow().clone()
        } else {
            None
        };
        if let Some(active) = active {
            if let Some(menu) = active.third {
                let mut result = LRESULT(0);
                // SAFETY: the message and its payload are forwarded synchronously, on the
                // interface's own STA. Return its LRESULT, particularly for WM_MENUCHAR.
                if unsafe {
                    menu.HandleMenuMsg2(message, WPARAM(wparam), LPARAM(lparam), Some(&mut result))
                }
                .is_ok()
                {
                    return result.0;
                }
            }
            if let Some(menu) = active.second {
                // SAFETY: same synchronous message lifetime and apartment as above.
                if unsafe { menu.HandleMenuMsg(message, WPARAM(wparam), LPARAM(lparam)) }.is_ok() {
                    return 0;
                }
            }
        }
    }
    if message == WM_NCDESTROY {
        // SAFETY: clear the borrowed pointer before the box can be dropped.
        unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, 0) };
    }
    // SAFETY: unhandled messages retain their original Win32 values.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}
