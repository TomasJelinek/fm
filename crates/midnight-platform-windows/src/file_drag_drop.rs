//! OLE file source and target owned by the engine's bounded STA actor.
use crate::WindowsFileSystem;
use midnight_core::*;
use std::{
    cell::{Cell, RefCell},
    io,
    path::Path,
    ptr,
    sync::Arc,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{
            DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, E_POINTER, HWND,
            POINTL,
        },
        System::{
            Com::IDataObject,
            Ole::{
                DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_NONE, DoDragDrop,
                IDropSource, IDropSource_Impl, IDropTarget, IDropTarget_Impl, OleInitialize,
                OleUninitialize, RegisterDragDrop, RevokeDragDrop,
            },
            SystemServices::MODIFIERKEYS_FLAGS,
        },
    },
    core::{BOOL, HRESULT, Ref, implement, w},
};
use windows_sys::Win32::{
    Foundation::POINT,
    Graphics::Gdi::ScreenToClient,
    System::Threading::GetCurrentProcessId,
    UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, GetPropW, GetWindowThreadProcessId,
        HWND_MESSAGE, IsWindow, KillTimer, MSG, PM_REMOVE, PeekMessageW, PostMessageW, RemovePropW,
        SetPropW, SetTimer, TranslateMessage, WM_KEYDOWN,
    },
};

#[path = "file_drag_drop_data.rs"]
mod data;
#[path = "file_drag_drop_input.rs"]
mod input;
#[cfg(test)]
use data::make_data;
use data::read_files;

// This input query has no side effects and does not depend on the actor's message history.
#[link(name = "user32")]
unsafe extern "system" {
    fn GetAsyncKeyState(key: i32) -> i16;
}

impl FileDragDropService for WindowsFileSystem {
    fn open_session(
        &self,
        host: DragHost,
        targets: SharedDropTargets,
        sink: Arc<dyn FileDropSink>,
    ) -> Result<Box<dyn FileDragDropSession>, FsError> {
        Ok(Box::new(Session::open(host, targets, sink)?))
    }
}

impl Session {
    fn open(
        host: DragHost,
        targets: SharedDropTargets,
        sink: Arc<dyn FileDropSink>,
    ) -> Result<Self, FsError> {
        let hwnd = validate_host(&host)?;
        let ole = Ole::new()?;
        let target = make_target(host.clone(), targets, sink.clone())?;
        // SAFETY: target is an apartment-bound COM object, retained by this actor. Windows
        // marshals calls to this STA even though the UI created the HWND on another thread.
        unsafe {
            RegisterDragDrop(HWND(hwnd), &target).map_err(error)?;
        }
        let token = Box::new(0u8);
        let identity = (&*token as *const u8).cast_mut().cast();
        // A window property disappears on destruction; it guards cleanup against HWND reuse.
        if unsafe {
            SetPropW(
                hwnd,
                w!("MidnightExplorer.FileDropTarget").as_ptr(),
                identity,
            )
        } == 0
        {
            unsafe {
                let _ = RevokeDragDrop(HWND(hwnd));
            }
            return Err(error(io::Error::last_os_error()));
        }
        Ok(Self {
            host,
            sink,
            target,
            token,
            _ole: ole,
        })
    }
}

fn error(message: impl std::fmt::Display) -> FsError {
    FsError::from_io(
        "Windows file drag/drop",
        Path::new(""),
        io::Error::other(message.to_string()),
    )
}

struct Ole;
impl Ole {
    fn new() -> Result<Self, FsError> {
        // SAFETY: paired with OleUninitialize on this same native actor thread.
        unsafe {
            OleInitialize(None).map_err(error)?;
        }
        Ok(Self)
    }
}
impl Drop for Ole {
    fn drop(&mut self) {
        unsafe {
            OleUninitialize();
        }
    }
}

struct Session {
    host: DragHost,
    sink: Arc<dyn FileDropSink>,
    target: IDropTarget,
    token: Box<u8>,
    _ole: Ole,
}
impl FileDragDropSession for Session {
    fn service_pending(&mut self) -> Result<(), FsError> {
        let deadline = Instant::now() + Duration::from_millis(5);
        // SAFETY: this STA owns its OLE helper windows; NULL pumps the complete thread queue.
        unsafe {
            let mut message: MSG = std::mem::zeroed();
            for _ in 0..128 {
                if Instant::now() >= deadline
                    || PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) == 0
                {
                    break;
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        Ok(())
    }
    fn start_drag(
        &mut self,
        files: &FileClipboard,
        cancel: &Cancellation,
    ) -> Result<DragFinish, FsError> {
        // A delayed preparation must never turn a completed click into a fresh native drag.
        if cancel.is_cancelled()
            || self.host.lifetime.is_cancelled()
            || !input::primary_pressed(
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                        windows_sys::Win32::UI::WindowsAndMessaging::SM_SWAPBUTTON,
                    )
                } != 0,
                |key| unsafe { GetAsyncKeyState(key) } < 0,
            )
        {
            return Ok(DragFinish::Cancelled);
        }
        let (data, feedback) = data::make_offer(files)?;
        let source: IDropSource = Source {
            cancel: cancel.clone(),
            lifetime: self.host.lifetime.clone(),
        }
        .into();
        let timer = Timer::new(cancel.clone(), self.host.lifetime.clone())?;
        let input_queue = input::InputQueue::attach(&self.host)?;
        let mut effect = DROPEFFECT_NONE;
        // SAFETY: both interfaces were created on this STA and live throughout the modal call.
        // Shell targets choose Copy/Move from volumes and modifiers. Only the target may
        // move files: completion feedback never authorizes native source cleanup here.
        let result = unsafe { DoDragDrop(&data, &source, outgoing_effects(), &mut effect) };
        drop(input_queue);
        drop(timer);
        self.sink.hover(None);
        if result == DRAGDROP_S_CANCEL {
            return Ok(DragFinish::Cancelled);
        }
        result.ok().map_err(error)?;
        Ok(if result == DRAGDROP_S_DROP {
            feedback.get().finish(effect)
        } else {
            DragFinish::Unconfirmed
        })
    }
}

fn outgoing_effects() -> DROPEFFECT {
    DROPEFFECT_COPY | DROPEFFECT_MOVE
}
impl Drop for Session {
    fn drop(&mut self) {
        self.sink.hover(None);
        if let NativeDragWindow::Windows { hwnd } = self.host.native {
            let hwnd = hwnd as windows_sys::Win32::Foundation::HWND;
            let identity = (&*self.token as *const u8).cast_mut().cast();
            // SAFETY: revoke only the exact registration's still-owned window property.
            unsafe {
                if GetPropW(hwnd, w!("MidnightExplorer.FileDropTarget").as_ptr()) == identity {
                    let _ = RevokeDragDrop(HWND(hwnd));
                    RemovePropW(hwnd, w!("MidnightExplorer.FileDropTarget").as_ptr());
                }
            }
        }
        // Retain the apartment-bound object until revocation is complete.
        let _ = &self.target;
    }
}

struct Timer {
    window: windows_sys::Win32::Foundation::HWND,
    id: usize,
}
struct TimerState {
    window: windows_sys::Win32::Foundation::HWND,
    id: usize,
    cancel: Cancellation,
    lifetime: Cancellation,
    posted: bool,
}
const CANCEL_WAKE: isize = 0x4d45_0001;
thread_local! { static ACTIVE_TIMER: RefCell<Option<TimerState>> = const { RefCell::new(None) }; }
impl Timer {
    fn new(cancel: Cancellation, lifetime: Cancellation) -> Result<Self, FsError> {
        if ACTIVE_TIMER.with(|active| active.borrow().is_some()) {
            return Err(error("A native drag loop is already active on this actor."));
        }
        // OLE does not call QueryContinueDrag for an ordinary WM_TIMER. The callback posts an
        // owned-helper Escape on cancellation; it never changes physical key state or sends
        // input to the UI thread, target application, foreground window, or system input queue.
        let window = unsafe {
            CreateWindowExW(
                0,
                w!("STATIC").as_ptr(),
                w!("Midnight DND wake").as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if window.is_null() {
            return Err(error(io::Error::last_os_error()));
        }
        let timer = unsafe { SetTimer(window, 1, 25, Some(cancel_timer)) };
        if timer == 0 {
            let failure = error(io::Error::last_os_error());
            unsafe {
                DestroyWindow(window);
            }
            Err(failure)
        } else {
            ACTIVE_TIMER.with(|active| {
                *active.borrow_mut() = Some(TimerState {
                    window,
                    id: timer,
                    cancel,
                    lifetime,
                    posted: false,
                })
            });
            Ok(Self { window, id: timer })
        }
    }
}
unsafe extern "system" fn cancel_timer(
    window: windows_sys::Win32::Foundation::HWND,
    _: u32,
    id: usize,
    _: u32,
) {
    ACTIVE_TIMER.with(|active| {
        let mut active = active.borrow_mut();
        if let Some(state) = active.as_mut()
            && state.window == window
            && state.id == id
            && !state.posted
            && (state.cancel.is_cancelled() || state.lifetime.is_cancelled())
        {
            // SAFETY: the timer and message-only helper belong to this actor. The OLE loop
            // consumes this private wake; the helper cannot receive physical keyboard input.
            state.posted = unsafe { PostMessageW(window, WM_KEYDOWN, 0x1b, CANCEL_WAKE) } != 0;
        }
    });
}
impl Drop for Timer {
    fn drop(&mut self) {
        ACTIVE_TIMER.with(|active| *active.borrow_mut() = None);
        unsafe {
            KillTimer(self.window, self.id);
            // No other component posts keyboard messages to this never-focused helper. Remove
            // its outstanding wake before another DoDragDrop can consume an old Escape.
            let mut message: MSG = std::mem::zeroed();
            while PeekMessageW(&mut message, self.window, WM_KEYDOWN, WM_KEYDOWN, PM_REMOVE) != 0 {}
            DestroyWindow(self.window);
        }
    }
}

fn validate_host(host: &DragHost) -> Result<windows_sys::Win32::Foundation::HWND, FsError> {
    let NativeDragWindow::Windows { hwnd } = host.native else {
        return Err(error("An OLE session requires a Windows window."));
    };
    let hwnd = hwnd as windows_sys::Win32::Foundation::HWND;
    let mut process = 0;
    // SAFETY: HWND queries validate opaque handles without dereferencing caller memory.
    if host.generation == 0
        || host.lifetime.is_cancelled()
        || unsafe {
            IsWindow(hwnd) == 0
                || GetWindowThreadProcessId(hwnd, &mut process) == 0
                || process != GetCurrentProcessId()
        }
    {
        return Err(error(
            "The native drag window is no longer owned by this application.",
        ));
    }
    Ok(hwnd)
}

fn make_target(
    host: DragHost,
    targets: SharedDropTargets,
    sink: Arc<dyn FileDropSink>,
) -> Result<IDropTarget, FsError> {
    validate_host(&host)?;
    Ok(Target {
        host,
        targets,
        sink,
        has_files: Cell::new(false),
    }
    .into())
}

// Agile=false is essential: the default windows-rs macro would allow callbacks on the UI STA.
#[implement(IDropTarget, Agile = false)]
struct Target {
    host: DragHost,
    targets: SharedDropTargets,
    sink: Arc<dyn FileDropSink>,
    has_files: Cell<bool>,
}
impl Target {
    fn at(&self, point: &POINTL) -> Option<(FileDropTarget, u64)> {
        let hwnd = validate_host(&self.host).ok()?;
        let mut point = POINT {
            x: point.x,
            y: point.y,
        };
        // SAFETY: conversion uses an owned live window and stack-owned POINT.
        if unsafe { ScreenToClient(hwnd, &mut point) } == 0 {
            return None;
        }
        let targets = self.targets.lock().unwrap_or_else(|e| e.into_inner());
        Some((
            targets.target_at(self.host.generation, point.x, point.y)?,
            targets.revision,
        ))
    }
    fn hover(
        &self,
        point: &POINTL,
        keys: u32,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        if effect.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: COM supplies a valid in/out effect pointer for the duration of this call.
        let offered = unsafe { *effect };
        let target = (self.has_files.get() && copy_requested(offered, keys))
            .then(|| self.at(point))
            .flatten();
        unsafe {
            *effect = if target.is_some() {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
        self.sink.hover(target.map(|(target, _)| target.pane));
        Ok(())
    }
}
impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        self.has_files
            .set(data.as_ref().is_some_and(data::supports_files));
        self.hover(point, keys.0, effect)
    }
    fn DragOver(
        &self,
        keys: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        self.hover(point, keys.0, effect)
    }
    fn DragLeave(&self) -> windows::core::Result<()> {
        self.has_files.set(false);
        self.sink.hover(None);
        Ok(())
    }
    fn Drop(
        &self,
        data: Ref<IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        if effect.is_null() {
            return Err(E_POINTER.into());
        }
        let offered = unsafe { *effect };
        unsafe {
            *effect = DROPEFFECT_NONE;
        }
        self.sink.hover(None);
        if !copy_requested(offered, keys.0) {
            return Ok(());
        }
        if let Some((target, targets_revision)) = self.at(point)
            && let Some(data) = data.as_ref()
            && let Ok(files) = read_files(data)
            && self
                .sink
                .try_accept(FileDropRequest {
                    files,
                    target,
                    targets_revision,
                })
                .is_ok()
        {
            unsafe {
                *effect = DROPEFFECT_COPY;
            }
        }
        Ok(())
    }
}

fn copy_requested(effect: DROPEFFECT, keys: u32) -> bool {
    effect.0 & DROPEFFECT_COPY.0 != 0 && keys & (4 | 32) == 0 // MK_SHIFT / MK_ALT request unsupported move/link.
}

#[implement(IDropSource, Agile = false)]
struct Source {
    cancel: Cancellation,
    lifetime: Cancellation,
}
impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        #[cfg(test)]
        handoff_tests::observe(keys.0);
        continue_drag(escape.as_bool(), keys.0, &self.cancel, &self.lifetime)
    }
    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}
fn continue_drag(
    escape: bool,
    keys: u32,
    cancel: &Cancellation,
    lifetime: &Cancellation,
) -> HRESULT {
    if escape || cancel.is_cancelled() || lifetime.is_cancelled() {
        DRAGDROP_S_CANCEL
    } else if keys & 1 == 0 {
        DRAGDROP_S_DROP
    }
    // MK_LBUTTON
    else {
        HRESULT(0)
    }
}

#[cfg(test)]
#[path = "file_drag_drop_handoff_tests.rs"]
mod handoff_tests;
#[cfg(test)]
#[path = "file_drag_drop_modal_tests.rs"]
mod modal_tests;
#[cfg(test)]
#[path = "file_drag_drop_native_tests.rs"]
mod native_tests;
#[cfg(test)]
#[path = "file_drag_drop_shell_tests.rs"]
mod shell_tests;
#[cfg(test)]
#[path = "file_drag_drop_tests.rs"]
mod tests;
