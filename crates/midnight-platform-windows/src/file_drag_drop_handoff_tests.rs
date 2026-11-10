//! Explicit desktop diagnostic: real input on owned windows, no fabricated OLE key state.
use super::*;
use std::sync::{Mutex, mpsc};
use std::thread;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetCursorPos, GetForegroundWindow, RegisterClassW, SetCursorPos,
    SetForegroundWindow, UnregisterClassW, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    WS_VISIBLE, WindowFromPoint,
};

#[repr(C)]
struct MouseInput {
    dx: i32,
    dy: i32,
    data: u32,
    flags: u32,
    time: u32,
    extra: usize,
}
#[repr(C)]
struct Input {
    kind: u32,
    mouse: MouseInput,
}
// MOUSEINPUT is the largest member of INPUT's union on supported Windows ABIs.
#[link(name = "user32")]
unsafe extern "system" {
    fn SendInput(count: u32, input: *const Input, size: i32) -> u32;
    fn SetCapture(
        window: windows_sys::Win32::Foundation::HWND,
    ) -> windows_sys::Win32::Foundation::HWND;
    fn ReleaseCapture() -> i32;
    fn SetThreadDpiAwarenessContext(context: isize) -> isize;
}

struct NativeContext(isize);
impl NativeContext {
    fn new() -> Self {
        let previous = unsafe { SetThreadDpiAwarenessContext(-4) };
        assert_ne!(previous, 0);
        let class = WNDCLASSW {
            lpfnWndProc: Some(DefWindowProcW),
            hInstance: unsafe { GetModuleHandleW(ptr::null()) },
            lpszClassName: w!("MidnightDragHandoffProbe").as_ptr(),
            ..Default::default()
        };
        assert_ne!(unsafe { RegisterClassW(&class) }, 0);
        Self(previous)
    }
}
impl Drop for NativeContext {
    fn drop(&mut self) {
        unsafe {
            UnregisterClassW(
                w!("MidnightDragHandoffProbe").as_ptr(),
                GetModuleHandleW(ptr::null()),
            );
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}

fn button(flags: u32) -> bool {
    let input = Input {
        kind: 0,
        mouse: MouseInput {
            dx: 0,
            dy: 0,
            data: 0,
            flags,
            time: 0,
            extra: 0,
        },
    };
    // SAFETY: initialized INPUT-compatible layout, borrowed for one synchronous call.
    unsafe { SendInput(1, &input, std::mem::size_of::<Input>() as i32) == 1 }
}

struct Desktop {
    cursor: POINT,
    foreground: usize,
    pressed: bool,
}
impl Desktop {
    fn capture() -> Self {
        // Do not start the diagnostic during somebody else's pointer/key gesture.
        for key in [1, 2, 4, 0x10, 0x11, 0x12] {
            assert!(
                unsafe { GetAsyncKeyState(key) } >= 0,
                "desktop input is busy"
            );
        }
        let mut cursor = POINT { x: 0, y: 0 };
        assert_ne!(unsafe { GetCursorPos(&mut cursor) }, 0);
        Self {
            cursor,
            foreground: unsafe { GetForegroundWindow() } as usize,
            pressed: false,
        }
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        if self.pressed {
            let _ = button(4);
        }
        // SAFETY: restores only the cursor/focus observed before this opt-in test.
        unsafe {
            ReleaseCapture();
            SetCursorPos(self.cursor.x, self.cursor.y);
            if self.foreground != 0 {
                SetForegroundWindow(self.foreground as _);
            }
        }
    }
}

struct Window(usize);
impl Window {
    fn new(x: i32) -> Self {
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                w!("MidnightDragHandoffProbe").as_ptr(),
                w!("Midnight owned drag diagnostic").as_ptr(),
                WS_POPUP | WS_VISIBLE,
                x,
                120,
                180,
                120,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        assert!(!hwnd.is_null());
        Self(hwnd as usize)
    }
    fn move_pointer(&self, x: i32) {
        assert_ne!(unsafe { SetCursorPos(x, 160) }, 0);
        assert_eq!(
            unsafe { WindowFromPoint(POINT { x, y: 160 }) } as usize,
            self.0,
            "only an owned test window may receive injected input"
        );
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0 as _);
        }
    }
}

#[derive(Default)]
struct Sink {
    requests: Mutex<Vec<FileDropRequest>>,
}
impl FileDropSink for Sink {
    fn try_accept(&self, request: FileDropRequest) -> Result<(), FsError> {
        self.requests.lock().unwrap().push(request);
        Ok(())
    }
}

type Samples = Arc<Mutex<Vec<(u32, bool)>>>;
thread_local! { static TRACE: RefCell<Option<Samples>> = const { RefCell::new(None) }; }

pub(super) fn observe(keys: u32) {
    TRACE.with(|trace| {
        if let Some(samples) = trace.borrow().as_ref() {
            let mut samples = samples.lock().unwrap();
            if samples.len() < 32 {
                samples.push((keys, unsafe { GetAsyncKeyState(1) } < 0));
            }
        }
    });
}

fn pump() {
    let mut message: MSG = unsafe { std::mem::zeroed() };
    unsafe {
        while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

#[test]
#[ignore = "interactive owned-window SendInput diagnostic; run alone on an idle test desktop"]
fn real_mouse_handoff_to_worker_sta_delivers_ole_files() {
    // Successful, cancelled, and fresh subsequent gestures exercise guard teardown.
    for cancel_during_drag in [false, true, false] {
        handoff(cancel_during_drag);
    }
}

fn handoff(cancel_during_drag: bool) {
    let _context = NativeContext::new();
    let mut desktop = Desktop::capture();
    let _ole = Ole::new().unwrap();
    let source_window = Window::new(120);
    let target_window = Window::new(340);
    let fixture = tempfile::tempdir().unwrap();
    let source_path = fixture.path().join("drag-文档.txt");
    std::fs::write(&source_path, b"owned drag fixture").unwrap();
    let destination = fixture.path().join("destination");
    std::fs::create_dir(&destination).unwrap();
    let expected = FileClipboard {
        paths: vec![source_path],
        intent: FileClipboardIntent::Copy,
    };
    let host = DragHost {
        generation: 7,
        native: NativeDragWindow::Windows {
            hwnd: target_window.0,
        },
        lifetime: Cancellation::default(),
    };
    let targets = Arc::new(Mutex::new(DropTargets {
        host_generation: 7,
        revision: 1,
        targets: vec![FileDropTarget {
            pane: PaneId::Right,
            navigation: NavigationId(1),
            directory: Arc::new(destination),
            rect: DropRect {
                x: 0,
                y: 0,
                width: 180,
                height: 120,
            },
        }],
    }));
    let sink = Arc::new(Sink::default());
    let samples = Arc::new(Mutex::new(Vec::new()));
    let cancel = Cancellation::default();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (start_tx, start_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let worker_sink = sink.clone();
    let worker_samples = samples.clone();
    let worker_cancel = cancel.clone();
    let files = expected.clone();
    let worker = thread::spawn(move || {
        let mut session = Session::open(host.clone(), targets, worker_sink).unwrap();
        ready_tx.send(()).unwrap();
        while start_rx.try_recv().is_err() {
            session.service_pending().unwrap();
            thread::sleep(Duration::from_millis(1));
        }
        let physical_start = unsafe { GetAsyncKeyState(1) } < 0;
        eprintln!("worker entered native source: physical_start={physical_start}");
        TRACE.with(|trace| *trace.borrow_mut() = Some(worker_samples));
        let result = session.start_drag(&files, &worker_cancel);
        TRACE.with(|trace| *trace.borrow_mut() = None);
        result_tx.send((physical_start, result)).unwrap();
    });
    let start = Instant::now();
    while ready_rx.try_recv().is_err() {
        pump();
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "worker registration stalled"
        );
        thread::sleep(Duration::from_millis(1));
    }
    unsafe {
        SetForegroundWindow(source_window.0 as _);
        SetCapture(source_window.0 as _);
    }
    source_window.move_pointer(160);
    assert!(button(2));
    desktop.pressed = true;
    let press = Instant::now();
    while press.elapsed() < Duration::from_millis(50) {
        pump();
        thread::sleep(Duration::from_millis(1));
    }
    assert!(unsafe { GetAsyncKeyState(1) } < 0);
    unsafe {
        ReleaseCapture();
    }
    start_tx.send(()).unwrap();
    let handoff = Instant::now();
    let mut moved = false;
    let mut released = false;
    let outcome = loop {
        pump();
        if !moved && handoff.elapsed() >= Duration::from_millis(150) {
            target_window.move_pointer(380);
            moved = true;
        }
        if !released && handoff.elapsed() >= Duration::from_millis(350) {
            assert!(button(4));
            desktop.pressed = false;
            released = true;
        }
        if cancel_during_drag && handoff.elapsed() >= Duration::from_millis(250) {
            cancel.cancel();
        }
        if let Ok(result) = result_rx.try_recv() {
            break result;
        }
        if handoff.elapsed() >= Duration::from_secs(2) {
            cancel.cancel();
        }
        assert!(
            handoff.elapsed() < Duration::from_secs(4),
            "native OLE loop did not cancel: samples={:?}, physical={}, admitted={}",
            samples.lock().unwrap(),
            unsafe { GetAsyncKeyState(1) } < 0,
            sink.requests.lock().unwrap().len()
        );
        thread::sleep(Duration::from_millis(1));
    };
    while !worker.is_finished() {
        pump();
        assert!(
            handoff.elapsed() < Duration::from_secs(4),
            "worker teardown stalled"
        );
        thread::sleep(Duration::from_millis(1));
    }
    worker.join().unwrap();
    eprintln!(
        "handoff: cancel_requested={cancel_during_drag} physical_start={} result={:?} samples={:?}",
        outcome.0,
        outcome.1,
        samples.lock().unwrap()
    );
    // Release before reporting a failed assertion, including an immediate false drop.
    drop(desktop);
    assert!(
        outcome.0,
        "real button must remain down through worker handoff"
    );
    assert_eq!(
        outcome.1.unwrap(),
        if cancel_during_drag {
            DragFinish::Cancelled
        } else {
            DragFinish::Copied
        }
    );
    let requests = sink.requests.lock().unwrap();
    assert_eq!(requests.len(), usize::from(!cancel_during_drag));
    if !cancel_during_drag {
        assert_eq!(requests[0].files, expected);
    }
    assert_eq!(
        std::fs::read(&expected.paths[0]).unwrap(),
        b"owned drag fixture"
    );
}
