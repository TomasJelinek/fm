use super::*;
use std::{
    ffi::OsString,
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    ptr,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use windows::Win32::{
    Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP},
    System::{
        Ole::{DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_NONE, OleInitialize, OleUninitialize},
        SystemServices::MODIFIERKEYS_FLAGS,
    },
};
use windows_sys::Win32::{
    Foundation::POINT,
    Graphics::Gdi::ClientToScreen,
    UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow},
};

fn files() -> FileClipboard {
    let mut raw: Vec<u16> = "C:\\raw\\文档".encode_utf16().collect();
    raw.push(0xd800);
    FileClipboard {
        paths: vec![PathBuf::from(OsString::from_wide(&raw))],
        intent: FileClipboardIntent::Copy,
    }
}

#[test]
fn ole_file_object_roundtrip_keeps_raw_utf16_and_rejects_move_offer() {
    // SAFETY: test balances OLE initialization on its test thread.
    unsafe {
        OleInitialize(None).unwrap();
    }
    let expected = files();
    let object = make_data(&expected).unwrap();
    assert_eq!(read_files(&object).unwrap(), expected);
    let mut moved = expected.clone();
    moved.intent = FileClipboardIntent::Move;
    assert!(make_data(&moved).is_err());
    drop(object);
    unsafe {
        OleUninitialize();
    }
}

#[test]
fn source_cancellation_and_release_are_explicit_and_do_not_request_a_move() {
    let cancel = Cancellation::default();
    let lifetime = Cancellation::default();
    assert_eq!(continue_drag(false, 1, &cancel, &lifetime), HRESULT(0));
    assert_eq!(continue_drag(false, 0, &cancel, &lifetime), DRAGDROP_S_DROP);
    assert_eq!(
        continue_drag(true, 1, &cancel, &lifetime),
        DRAGDROP_S_CANCEL
    );
    cancel.cancel();
    assert_eq!(
        continue_drag(false, 1, &cancel, &lifetime),
        DRAGDROP_S_CANCEL
    );
}

#[test]
fn untrusted_ole_payloads_are_rejected_whole_before_decoding_unbounded_memory() {
    let _ole = Ole::new().unwrap();
    for bytes in [
        vec![0u8; MAX_FILE_CLIPBOARD_BYTES + 1],
        vec![0u8; 17],
        vec![0xffu8; 64],
    ] {
        let object = data::malformed_offer(bytes);
        assert!(read_files(&object).is_err());
    }
}

#[test]
fn timer_teardown_removes_an_unconsumed_cancel_wake_before_the_next_drag() {
    let _ole = Ole::new().unwrap();
    let cancel = Cancellation::default();
    let timer = Timer::new(cancel.clone(), Cancellation::default()).unwrap();
    cancel.cancel();
    unsafe {
        cancel_timer(timer.window, 0, timer.id, 0);
    }
    drop(timer);
    let mut message: MSG = unsafe { std::mem::zeroed() };
    let leftover = unsafe {
        PeekMessageW(
            &mut message,
            ptr::null_mut(),
            WM_KEYDOWN,
            WM_KEYDOWN,
            PM_REMOVE,
        )
    };
    assert_eq!(
        leftover, 0,
        "stale actor cancellation must not cancel a later drag"
    );
}

struct Capture {
    refuse: AtomicBool,
    requests: Mutex<Vec<FileDropRequest>>,
}
impl FileDropSink for Capture {
    fn try_accept(&self, request: FileDropRequest) -> Result<(), FsError> {
        if self.refuse.load(Ordering::Relaxed) {
            return Err(error("queue full"));
        }
        self.requests.lock().unwrap().push(request);
        Ok(())
    }
}

#[test]
fn ole_target_uses_drop_coordinates_and_reports_copy_only_after_sink_admission() {
    unsafe {
        OleInitialize(None).unwrap();
    }
    let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            200,
            100,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    assert!(!hwnd.is_null());
    let host = DragHost {
        generation: 1,
        native: NativeDragWindow::Windows {
            hwnd: hwnd as usize,
        },
        lifetime: Cancellation::default(),
    };
    let targets = Arc::new(Mutex::new(DropTargets {
        host_generation: 1,
        revision: 1,
        targets: vec![FileDropTarget {
            pane: PaneId::Right,
            navigation: NavigationId(2),
            directory: Arc::new(PathBuf::from("C:\\destination")),
            rect: DropRect {
                x: 100,
                y: 0,
                width: 100,
                height: 100,
            },
        }],
    }));
    let sink = Arc::new(Capture {
        refuse: AtomicBool::new(false),
        requests: Mutex::new(Vec::new()),
    });
    let target = make_target(host.clone(), targets, sink.clone()).unwrap();
    let data = make_data(&files()).unwrap();
    let mut point = POINT { x: 150, y: 50 };
    assert_ne!(unsafe { ClientToScreen(hwnd, &mut point) }, 0);
    let at = POINTL {
        x: point.x,
        y: point.y,
    };
    let mut effect = DROPEFFECT_MOVE;
    unsafe {
        target
            .DragEnter(&data, MODIFIERKEYS_FLAGS(1), at, &mut effect)
            .unwrap();
    }
    assert_eq!(effect, DROPEFFECT_NONE);
    effect = DROPEFFECT_COPY;
    unsafe {
        target
            .DragEnter(&data, MODIFIERKEYS_FLAGS(1), at, &mut effect)
            .unwrap();
    }
    assert_eq!(effect, DROPEFFECT_COPY);
    sink.refuse.store(true, Ordering::Relaxed);
    effect = DROPEFFECT_COPY;
    unsafe {
        target
            .Drop(&data, MODIFIERKEYS_FLAGS(0), at, &mut effect)
            .unwrap();
    }
    assert_eq!(effect, DROPEFFECT_NONE);
    assert!(sink.requests.lock().unwrap().is_empty());
    sink.refuse.store(false, Ordering::Relaxed);
    effect = DROPEFFECT_COPY;
    unsafe {
        target
            .Drop(&data, MODIFIERKEYS_FLAGS(0), at, &mut effect)
            .unwrap();
    }
    assert_eq!(effect, DROPEFFECT_COPY);
    assert_eq!(sink.requests.lock().unwrap()[0].target.pane, PaneId::Right);
    host.lifetime.cancel();
    effect = DROPEFFECT_COPY;
    unsafe {
        target
            .DragOver(MODIFIERKEYS_FLAGS(1), at, &mut effect)
            .unwrap();
    }
    assert_eq!(effect, DROPEFFECT_NONE);
    drop(target);
    drop(data);
    unsafe {
        DestroyWindow(hwnd);
        OleUninitialize();
    }
}
