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
    Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, E_FAIL, E_NOTIMPL},
    System::{
        Com::{FORMATETC, IAdviseSink, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, STGMEDIUM},
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

#[derive(Default)]
struct Capture {
    refuse: AtomicBool,
    requests: Mutex<Vec<FileDropRequest>>,
    rejected: Mutex<Vec<FsError>>,
}
impl FileDropSink for Capture {
    fn try_accept(&self, request: FileDropRequest) -> Result<(), FsError> {
        if self.refuse.load(Ordering::Relaxed) {
            return Err(error("queue full"));
        }
        self.requests.lock().unwrap().push(request);
        Ok(())
    }
    fn reject(&self, error: FsError) {
        self.rejected.lock().unwrap().push(error);
    }
}

impl Capture {
    fn take_rejected(&self) -> Vec<String> {
        std::mem::take(&mut *self.rejected.lock().unwrap())
            .into_iter()
            .map(|error| error.message)
            .collect()
    }
}

/// An owned live window whose client columns 100..200 are the Right pane's drop target.
struct DropFixture {
    hwnd: windows_sys::Win32::Foundation::HWND,
    host: DragHost,
    targets: SharedDropTargets,
    sink: Arc<Capture>,
    target: IDropTarget,
    // Declared last: OLE is uninitialized only after the target is released.
    _ole: Ole,
}
impl DropFixture {
    fn new() -> Self {
        let ole = Ole::new().unwrap();
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
        let sink = Arc::new(Capture::default());
        let target = make_target(host.clone(), targets.clone(), sink.clone()).unwrap();
        Self {
            hwnd,
            host,
            targets,
            sink,
            target,
            _ole: ole,
        }
    }
    /// Screen coordinates of a client point, as OLE supplies them.
    fn at(&self, x: i32, y: i32) -> POINTL {
        let mut point = POINT { x, y };
        assert_ne!(unsafe { ClientToScreen(self.hwnd, &mut point) }, 0);
        POINTL {
            x: point.x,
            y: point.y,
        }
    }
    fn enter(&self, data: &IDataObject, offered: DROPEFFECT, at: POINTL) -> DROPEFFECT {
        let mut effect = offered;
        unsafe {
            self.target
                .DragEnter(data, MODIFIERKEYS_FLAGS(1), at, &mut effect)
                .unwrap();
        }
        effect
    }
    fn drop_files(&self, data: &IDataObject, keys: u32, at: POINTL) -> DROPEFFECT {
        let mut effect = DROPEFFECT_COPY;
        unsafe {
            self.target
                .Drop(data, MODIFIERKEYS_FLAGS(keys), at, &mut effect)
                .unwrap();
        }
        effect
    }
}
impl Drop for DropFixture {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}

#[test]
fn ole_target_uses_drop_coordinates_and_reports_copy_only_after_sink_admission() {
    let f = DropFixture::new();
    let data = make_data(&files()).unwrap();
    let at = f.at(150, 50);
    assert_eq!(f.enter(&data, DROPEFFECT_MOVE, at), DROPEFFECT_NONE);
    assert_eq!(f.enter(&data, DROPEFFECT_COPY, at), DROPEFFECT_COPY);
    f.sink.refuse.store(true, Ordering::Relaxed);
    assert_eq!(f.drop_files(&data, 0, at), DROPEFFECT_NONE);
    assert!(f.sink.requests.lock().unwrap().is_empty());
    assert_eq!(
        f.sink.take_rejected(),
        ["queue full"],
        "the sink's refusal must reach the user"
    );
    f.sink.refuse.store(false, Ordering::Relaxed);
    assert_eq!(f.enter(&data, DROPEFFECT_COPY, at), DROPEFFECT_COPY);
    assert_eq!(f.drop_files(&data, 0, at), DROPEFFECT_COPY);
    assert_eq!(
        f.sink.requests.lock().unwrap()[0].target.pane,
        PaneId::Right
    );
    assert!(f.sink.take_rejected().is_empty());
    f.host.lifetime.cancel();
    let mut effect = DROPEFFECT_COPY;
    unsafe {
        f.target
            .DragOver(MODIFIERKEYS_FLAGS(1), at, &mut effect)
            .unwrap();
    }
    assert_eq!(effect, DROPEFFECT_NONE);
    // A window that has closed refuses the drop and has nobody to tell.
    assert_eq!(f.drop_files(&data, 0, at), DROPEFFECT_NONE);
    assert!(f.sink.take_rejected().is_empty());
}

/// A CF_HDROP with `count` distinct paths; `wide: false` writes an ANSI header.
fn hdrop(count: usize, wide: bool) -> Vec<u8> {
    let mut bytes = 20u32.to_le_bytes().to_vec(); // DROPFILES.pFiles
    bytes.extend([0u8; 12]); // pt and fNC
    bytes.extend(u32::from(wide).to_le_bytes());
    for index in 0..count {
        for unit in format!("C:\\{index}").encode_utf16().chain([0]) {
            bytes.extend(unit.to_le_bytes());
        }
    }
    bytes.extend([0u8; 2]);
    bytes
}

#[test]
fn ole_target_reports_a_file_list_it_cannot_read_instead_of_dropping_silently() {
    let f = DropFixture::new();
    let at = f.at(150, 50);
    let data = data::malformed_offer(hdrop(1, false));
    // The cursor promises Copy for any CF_HDROP; only Drop decodes the list.
    assert_eq!(f.enter(&data, DROPEFFECT_COPY, at), DROPEFFECT_COPY);
    assert_eq!(f.drop_files(&data, 0, at), DROPEFFECT_NONE);
    assert_eq!(
        f.sink.take_rejected(),
        ["The dropped file list is malformed or not Unicode."],
        "an ANSI list: the read failure must be reported in drop terms"
    );
    assert!(f.sink.requests.lock().unwrap().is_empty());
}

#[test]
fn ole_target_reads_a_list_of_any_length_within_its_bytes() {
    // CF_HDROP has no item limit, and lists of more than 4,096 paths were once refused
    // here. The engine's sink bounds the roots one Copy takes (`FileDropRequest`).
    let f = DropFixture::new();
    let at = f.at(150, 50);
    let data = data::malformed_offer(hdrop(5000, true));
    assert_eq!(f.enter(&data, DROPEFFECT_COPY, at), DROPEFFECT_COPY);
    assert_eq!(f.drop_files(&data, 0, at), DROPEFFECT_COPY);
    assert!(f.sink.take_rejected().is_empty());
    let requests = f.sink.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].files.paths.len(), 5000);
    assert_eq!(requests[0].files.paths[4999], Path::new("C:\\4999"));
}

#[test]
fn ole_target_reports_a_release_that_no_longer_matches_its_copy_cursor() {
    let f = DropFixture::new();
    let data = make_data(&files()).unwrap();
    let at = f.at(150, 50);
    // OLE delivers Drop only after DragOver showed Copy, so each refusal follows a
    // Copy cursor: Shift pressed at release, the pointer left the pane, or the pane
    // started loading and withdrew its drop target.
    assert_eq!(f.enter(&data, DROPEFFECT_COPY, at), DROPEFFECT_COPY);
    assert_eq!(f.drop_files(&data, 4, at), DROPEFFECT_NONE);
    assert_eq!(
        f.sink.take_rejected(),
        ["Dropped files can only be copied; release them without Shift or Alt."]
    );
    assert_eq!(f.enter(&data, DROPEFFECT_COPY, at), DROPEFFECT_COPY);
    assert_eq!(f.drop_files(&data, 0, f.at(50, 50)), DROPEFFECT_NONE);
    assert_eq!(f.sink.take_rejected(), ["The drop destination changed."]);
    assert_eq!(f.enter(&data, DROPEFFECT_COPY, at), DROPEFFECT_COPY);
    {
        let mut targets = f.targets.lock().unwrap();
        targets.revision += 1;
        targets.targets.clear();
    }
    assert_eq!(f.drop_files(&data, 0, at), DROPEFFECT_NONE);
    assert_eq!(f.sink.take_rejected(), ["The drop destination changed."]);
    assert!(f.sink.requests.lock().unwrap().is_empty());
    // A Drop without a current DragEnter, or after DragLeave, had no Copy cursor.
    assert_eq!(f.drop_files(&data, 0, at), DROPEFFECT_NONE);
    assert_eq!(f.enter(&data, DROPEFFECT_COPY, at), DROPEFFECT_NONE);
    unsafe { f.target.DragLeave().unwrap() };
    assert_eq!(f.drop_files(&data, 0, at), DROPEFFECT_NONE);
    assert!(f.sink.take_rejected().is_empty());
}

const SOURCE_TEXT: &str = "text chosen by the drag source";

/// A drag source that offers a file list but fails to render it, with an error
/// description of its own choosing (any length, for a cross-process source).
#[implement(IDataObject)]
struct Unrenderable;
impl IDataObject_Impl for Unrenderable_Impl {
    fn GetData(&self, _: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        Err(windows::core::Error::new(E_FAIL, SOURCE_TEXT.repeat(8)))
    }
    fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn QueryGetData(&self, _: *const FORMATETC) -> HRESULT {
        HRESULT(0)
    }
    fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, _: *mut FORMATETC) -> HRESULT {
        E_NOTIMPL
    }
    fn SetData(
        &self,
        _: *const FORMATETC,
        _: *const STGMEDIUM,
        _: BOOL,
    ) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn EnumFormatEtc(&self, _: u32) -> windows::core::Result<IEnumFORMATETC> {
        Err(E_NOTIMPL.into())
    }
    fn DAdvise(
        &self,
        _: *const FORMATETC,
        _: u32,
        _: Ref<IAdviseSink>,
    ) -> windows::core::Result<u32> {
        Err(E_NOTIMPL.into())
    }
    fn DUnadvise(&self, _: u32) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
        Err(E_NOTIMPL.into())
    }
}

#[test]
fn ole_target_reports_an_unrenderable_file_list_by_code_only() {
    let f = DropFixture::new();
    let data: IDataObject = Unrenderable.into();
    let Err(failure) = (unsafe { data.GetData(&FORMATETC::default()) }) else {
        panic!("the fixture must fail to render its file list");
    };
    assert!(
        failure.message().contains(SOURCE_TEXT),
        "the fixture must carry the source's description: {failure}"
    );
    let at = f.at(150, 50);
    assert_eq!(f.enter(&data, DROPEFFECT_COPY, at), DROPEFFECT_COPY);
    assert_eq!(f.drop_files(&data, 0, at), DROPEFFECT_NONE);
    assert_eq!(
        f.sink.take_rejected(),
        ["The drag source did not provide its file list (0x80004005)."]
    );
}
