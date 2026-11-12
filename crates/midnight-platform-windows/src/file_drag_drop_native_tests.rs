//! Actual OLE apartment marshalling using a hidden window, without pointer/clipboard changes.
use super::*;
use std::{
    ffi::c_void,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc,
    },
    thread,
};
use windows::core::{GUID, Interface};
use windows_sys::Win32::{
    Graphics::Gdi::ClientToScreen,
    System::Threading::GetCurrentThreadId,
    UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow},
};

// Exact OLE declarations avoid expanding the production SDK feature set for this test harness.
#[link(name = "ole32")]
unsafe extern "system" {
    fn CoMarshalInterThreadInterfaceInStream(
        iid: *const GUID,
        object: *mut c_void,
        stream: *mut *mut c_void,
    ) -> HRESULT;
    fn CoGetInterfaceAndReleaseStream(
        stream: *mut c_void,
        iid: *const GUID,
        object: *mut *mut c_void,
    ) -> HRESULT;
}

struct Window(usize);
impl Window {
    fn new() -> Self {
        let class = w!("STATIC");
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                160,
                100,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        assert!(!hwnd.is_null());
        Self(hwnd as usize)
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
struct Receiver {
    requests: Mutex<Vec<FileDropRequest>>,
    thread: AtomicU32,
}
impl FileDropSink for Receiver {
    fn try_accept(&self, request: FileDropRequest) -> Result<(), FsError> {
        self.thread
            .store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
        self.requests.lock().unwrap().push(request);
        Ok(())
    }
}

#[test]
fn production_target_registers_on_worker_and_marshals_drop_away_from_ui_window_owner() {
    let _ole = Ole::new().unwrap();
    let window = Window::new();
    let ui_thread = unsafe { GetCurrentThreadId() };
    let lifetime = Cancellation::default();
    let host = DragHost {
        generation: 11,
        native: NativeDragWindow::Windows { hwnd: window.0 },
        lifetime: lifetime.clone(),
    };
    let targets = Arc::new(std::sync::Mutex::new(DropTargets {
        host_generation: 11,
        revision: 8,
        targets: vec![FileDropTarget {
            pane: PaneId::Left,
            navigation: NavigationId(4),
            directory: Arc::new("C:\\destination".into()),
            rect: DropRect {
                x: 0,
                y: 0,
                width: 160,
                height: 100,
            },
        }],
    }));
    let sink = Arc::new(Receiver::default());
    let worker_sink = sink.clone();
    let worker_host = host.clone();
    let worker_targets = targets.clone();
    let quit = Arc::new(AtomicBool::new(false));
    let stop = quit.clone();
    let (ready, wait) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let mut session = Session::open(worker_host, worker_targets, worker_sink).unwrap();
        let mut stream = ptr::null_mut();
        unsafe {
            CoMarshalInterThreadInterfaceInStream(
                &IDropTarget::IID,
                session.target.as_raw(),
                &mut stream,
            )
            .ok()
            .unwrap();
        }
        ready
            .send((stream as usize, unsafe { GetCurrentThreadId() }))
            .unwrap();
        while !stop.load(Ordering::SeqCst) {
            session.service_pending().unwrap();
            thread::sleep(Duration::from_millis(1));
        }
    });
    let (stream, native_thread) = wait.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(
        Session::open(host.clone(), targets.clone(), sink.clone()).is_err(),
        "a second session cannot steal registration"
    );
    let mut raw = ptr::null_mut();
    unsafe {
        CoGetInterfaceAndReleaseStream(stream as _, &IDropTarget::IID, &mut raw)
            .ok()
            .unwrap();
    }
    let target = unsafe { IDropTarget::from_raw(raw) };
    let files = FileClipboard {
        paths: vec!["C:\\source\\文档.txt".into()],
        intent: FileClipboardIntent::Copy,
    };
    let data = make_data(&files).unwrap();
    let mut point = POINT { x: 50, y: 40 };
    unsafe {
        assert_ne!(ClientToScreen(window.0 as _, &mut point), 0);
    }
    let point = POINTL {
        x: point.x,
        y: point.y,
    };
    let mut effect = DROPEFFECT_COPY;
    unsafe {
        target
            .DragEnter(&data, MODIFIERKEYS_FLAGS(1), point, &mut effect)
            .unwrap();
    }
    assert_eq!(effect, DROPEFFECT_COPY);
    unsafe {
        target
            .Drop(&data, MODIFIERKEYS_FLAGS(0), point, &mut effect)
            .unwrap();
    }
    assert_eq!(effect, DROPEFFECT_COPY);
    assert_eq!(sink.thread.load(Ordering::SeqCst), native_thread);
    assert_ne!(native_thread, ui_thread);
    assert_eq!(sink.requests.lock().unwrap()[0].files, files);
    lifetime.cancel();
    effect = DROPEFFECT_COPY;
    unsafe {
        target
            .DragOver(MODIFIERKEYS_FLAGS(1), point, &mut effect)
            .unwrap();
    }
    assert_eq!(effect, DROPEFFECT_NONE);
    drop(target);
    drop(data);
    quit.store(true, Ordering::SeqCst);
    worker.join().unwrap();
    assert!(
        unsafe {
            GetPropW(
                window.0 as _,
                w!("MidnightExplorer.FileDropTarget").as_ptr(),
            )
        }
        .is_null()
    );
    let replacement = DragHost {
        lifetime: Cancellation::default(),
        ..host
    };
    drop(
        Session::open(replacement, targets, sink).expect("worker cleanup must revoke registration"),
    );
}
