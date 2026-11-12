//! Opt-in OLE source-loop smoke: tiny non-activating window, synthetic IDropSource input.
//! No physical input, clipboard ownership, or filesystem operation is performed.
use super::*;
use std::{
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
    thread,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetForegroundWindow,
    RegisterClassW, UnregisterClassW, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP, WS_VISIBLE, WindowFromPoint,
};

struct Window(usize);
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0 as _);
            UnregisterClassW(
                w!("MidnightDndModalSmoke").as_ptr(),
                GetModuleHandleW(ptr::null()),
            );
        }
    }
}

#[derive(Default)]
struct Sink {
    entered: AtomicBool,
    dropped: AtomicU32,
}
impl FileDropSink for Sink {
    fn try_accept(&self, _: FileDropRequest) -> Result<(), FsError> {
        self.dropped.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn hover(&self, pane: Option<PaneId>) {
        if pane.is_some() {
            self.entered.store(true, Ordering::Relaxed);
        }
    }
}

/// Only the button-state fixture differs: policy delegates to the production source.
#[implement(IDropSource, Agile = false)]
struct TestSource {
    source: IDropSource,
    sink: Arc<Sink>,
    cancel: Cancellation,
    wake: Cancellation,
    drop_after_hover: bool,
    started: Instant,
}
impl IDropSource_Impl for TestSource_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, _: MODIFIERKEYS_FLAGS) -> HRESULT {
        if self.started.elapsed() > Duration::from_millis(600) {
            self.cancel.cancel();
        }
        let release = self.drop_after_hover
            && self.sink.entered.load(Ordering::Relaxed)
            && self.wake.is_cancelled();
        // Synthetic left-button state is contained in this test COM object, not sent to Windows.
        unsafe {
            self.source.QueryContinueDrag(
                escape.as_bool() && !release,
                MODIFIERKEYS_FLAGS(u32::from(!release)),
            )
        }
    }
    fn GiveFeedback(&self, effect: DROPEFFECT) -> HRESULT {
        unsafe { self.source.GiveFeedback(effect) }
    }
}

#[test]
#[ignore = "brief non-activating native window; run explicitly on an interactive test desktop"]
fn native_modal_copy_and_idle_cancellation_without_physical_input() {
    let _ole = Ole::new().unwrap();
    let foreground = unsafe { GetForegroundWindow() };
    let mut pointer = POINT { x: 0, y: 0 };
    assert_ne!(unsafe { GetCursorPos(&mut pointer) }, 0);
    let instance = unsafe { GetModuleHandleW(ptr::null()) };
    let class = WNDCLASSW {
        lpfnWndProc: Some(DefWindowProcW),
        hInstance: instance,
        lpszClassName: w!("MidnightDndModalSmoke").as_ptr(),
        ..Default::default()
    };
    assert_ne!(unsafe { RegisterClassW(&class) }, 0);
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            w!("MidnightDndModalSmoke").as_ptr(),
            w!("DND test").as_ptr(),
            WS_POPUP | WS_VISIBLE,
            pointer.x.saturating_sub(8),
            pointer.y.saturating_sub(8),
            24,
            24,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null_mut(),
        )
    };
    assert!(!hwnd.is_null());
    let window = Window(hwnd as usize);
    assert_eq!(
        unsafe { WindowFromPoint(pointer) },
        hwnd,
        "only the owned test window may receive the offer"
    );
    assert_eq!(
        unsafe { GetForegroundWindow() },
        foreground,
        "test window must not activate"
    );
    let sink = Arc::new(Sink::default());
    let lifetime = Cancellation::default();
    let host = DragHost {
        generation: 1,
        native: NativeDragWindow::Windows { hwnd: window.0 },
        lifetime,
    };
    let targets = Arc::new(std::sync::Mutex::new(DropTargets {
        host_generation: 1,
        revision: 1,
        targets: vec![FileDropTarget {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            directory: Arc::new("C:\\synthetic-destination".into()),
            rect: DropRect {
                x: 0,
                y: 0,
                width: 24,
                height: 24,
            },
        }],
    }));
    let mut session = Session::open(host.clone(), targets, sink.clone()).unwrap();
    session.service_pending().unwrap();
    let files = FileClipboard {
        paths: vec!["C:\\synthetic-source.txt".into()],
        intent: FileClipboardIntent::Copy,
    };
    let data = make_data(&files).unwrap();
    for drop_after_hover in [false, true] {
        sink.entered.store(false, Ordering::Relaxed);
        let cancel = Cancellation::default();
        let wake = if drop_after_hover {
            Cancellation::default()
        } else {
            cancel.clone()
        };
        let source: IDropSource = Source {
            cancel: cancel.clone(),
            lifetime: host.lifetime.clone(),
        }
        .into();
        let source: IDropSource = TestSource {
            source,
            sink: sink.clone(),
            cancel: cancel.clone(),
            wake: wake.clone(),
            drop_after_hover,
            started: Instant::now(),
        }
        .into();
        let done = Arc::new(AtomicBool::new(false));
        let finished = done.clone();
        // A native modal failure must not leave an unattended test window behind. This is
        // an explicitly isolated smoke process; process exit also destroys its owned HWNDs.
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(3));
            if !finished.load(Ordering::SeqCst) {
                eprintln!("OLE modal loop exceeded the native smoke deadline");
                std::process::exit(3);
            }
        });
        let wake_request = wake.clone();
        let cancellation = thread::spawn(move || {
            thread::sleep(Duration::from_millis(80));
            wake_request.cancel();
        });
        let timer = Timer::new(wake, host.lifetime.clone()).unwrap();
        let started = Instant::now();
        let mut effect = DROPEFFECT_NONE;
        let result = unsafe { DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect) };
        done.store(true, Ordering::SeqCst);
        drop(timer);
        cancellation.join().unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "idle OLE loop did not observe bounded control changes"
        );
        if drop_after_hover {
            assert_eq!(result, DRAGDROP_S_DROP);
            assert_eq!(effect, DROPEFFECT_COPY);
            assert_eq!(sink.dropped.load(Ordering::Relaxed), 1);
        } else {
            assert_eq!(result, DRAGDROP_S_CANCEL);
            assert_eq!(sink.dropped.load(Ordering::Relaxed), 0);
        }
    }
    drop(data);
    drop(session);
    drop(window);
    assert_eq!(unsafe { GetForegroundWindow() }, foreground);
}
