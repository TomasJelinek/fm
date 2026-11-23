//! What another application receives from an outgoing drag: the Shell's item list, our
//! lossless CF_HDROP list, a drag image, effects, and the early-release rule.
use super::handoff_tests::{
    Desktop, NativeContext, ReleaseCapture, SetCapture, Window, button, per_monitor_aware, pump,
};
use super::*;
use std::{
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    sync::{Mutex, mpsc},
    thread,
};
use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows_sys::Win32::System::DataExchange::RegisterClipboardFormatW;

fn has(data: &IDataObject, name: windows::core::PCWSTR) -> bool {
    let id = unsafe { RegisterClipboardFormatW(name.as_ptr()) } as u16;
    let format = FORMATETC {
        cfFormat: id,
        ptd: ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    unsafe { data.QueryGetData(&format) }.is_ok()
}

fn offer(files: &FileClipboard) -> IDataObject {
    let (inner, _) = data::make_offer(files).unwrap();
    shell::wrap(files, inner)
}

#[test]
fn outgoing_offer_carries_shell_items_a_drag_image_and_the_lossless_file_list() {
    let _ole = Ole::new().unwrap();
    let fixture = crate::test_fixtures::build_temp("outgoing-");
    // The long spelling of the folder: a TEMP given in 8.3 form (as on hosted runners) would
    // not convert back exactly, and the offer then rightly stays CF_HDROP only.
    let root = {
        let canonical = std::fs::canonicalize(fixture.path()).unwrap();
        let text = canonical.to_string_lossy().into_owned();
        std::path::PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
    };
    let mut paths = Vec::new();
    for name in ["a.txt", "Ünïcode 文档.txt"] {
        let path = root.join(name);
        std::fs::write(&path, b"x").unwrap();
        paths.push(path);
    }
    std::fs::create_dir(root.join("folder")).unwrap();
    paths.push(root.join("folder"));
    let files = FileClipboard {
        paths: paths.clone(),
        intent: FileClipboardIntent::Copy,
    };
    let data = offer(&files);
    // Explorer's own formats come with the Shell object.
    assert!(has(&data, w!("Shell IDList Array")));
    assert_eq!(read_files(&data).unwrap().paths, paths);
    shell::attach_drag_image(&data);
    assert!(has(&data, w!("DragImageBits")), "the Shell drag image");
    // A target's completion report on the outer object reaches the source.
    assert_eq!(shell::feedback(&data), (None, None));

    // A name the Shell cannot convert back exactly keeps only our list, unchanged:
    // an unpaired surrogate, as Windows allows in names.
    let odd = fixture
        .path()
        .join(std::ffi::OsString::from_wide(&[0x61, 0xd800, 0x62]));
    let files = FileClipboard {
        paths: vec![paths[0].clone(), odd.clone()],
        intent: FileClipboardIntent::Copy,
    };
    let data = offer(&files);
    assert!(!has(&data, w!("Shell IDList Array")));
    assert_eq!(
        read_files(&data).unwrap().paths,
        vec![paths[0].clone(), odd]
    );

    // More items than the Shell list takes: still every path, in order.
    let many: Vec<PathBuf> = (0..=shell::MAX_SHELL_ITEMS)
        .map(|index| fixture.path().join(format!("{index}.txt")))
        .collect();
    let files = FileClipboard {
        paths: many.clone(),
        intent: FileClipboardIntent::Copy,
    };
    let data = offer(&files);
    assert!(!has(&data, w!("Shell IDList Array")));
    assert_eq!(read_files(&data).unwrap().paths, many);
}

/// What the foreign test target saw.
#[derive(Debug, Default, Clone)]
struct Seen {
    entered: bool,
    left: bool,
    dropped: Option<Vec<PathBuf>>,
    shell_items: bool,
    drag_image: bool,
    offered: u32,
}

#[implement(IDropTarget, Agile = false)]
struct ForeignTarget {
    answer: DROPEFFECT,
    seen: Arc<Mutex<Seen>>,
}
impl IDropTarget_Impl for ForeignTarget_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        _: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let mut seen = self.seen.lock().unwrap();
        seen.entered = true;
        if let Some(data) = data.as_ref() {
            seen.shell_items = has(data, w!("Shell IDList Array"));
            seen.drag_image = has(data, w!("DragImageBits"));
        }
        unsafe {
            seen.offered = (*effect).0;
            *effect = self.answer;
        }
        Ok(())
    }
    fn DragOver(
        &self,
        _: MODIFIERKEYS_FLAGS,
        _: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        unsafe { *effect = self.answer };
        Ok(())
    }
    fn DragLeave(&self) -> windows::core::Result<()> {
        self.seen.lock().unwrap().left = true;
        Ok(())
    }
    fn Drop(
        &self,
        data: Ref<IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        _: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let paths = data
            .as_ref()
            .and_then(|data| read_files(data).ok())
            .map(|files| files.paths);
        // An optimized Move, as Explorer reports one on the same volume: the target
        // moved the files itself and says so.
        if self.answer == DROPEFFECT_MOVE
            && let Some(data) = data.as_ref()
        {
            for name in [
                w!("Performed DropEffect"),
                w!("Logical Performed DropEffect"),
            ] {
                let id = unsafe { RegisterClipboardFormatW(name.as_ptr()) } as u16;
                let memory = unsafe {
                    windows_sys::Win32::System::Memory::GlobalAlloc(
                        windows_sys::Win32::System::Memory::GMEM_MOVEABLE,
                        4,
                    )
                };
                unsafe {
                    let pointer = windows_sys::Win32::System::Memory::GlobalLock(memory);
                    pointer.cast::<u32>().write(2);
                    windows_sys::Win32::System::Memory::GlobalUnlock(memory);
                    let medium = windows::Win32::System::Com::STGMEDIUM {
                        tymed: TYMED_HGLOBAL.0 as u32,
                        u: windows::Win32::System::Com::STGMEDIUM_0 {
                            hGlobal: windows::Win32::Foundation::HGLOBAL(memory),
                        },
                        pUnkForRelease: std::mem::ManuallyDrop::new(None),
                    };
                    let format = FORMATETC {
                        cfFormat: id,
                        ptd: ptr::null_mut(),
                        dwAspect: DVASPECT_CONTENT.0,
                        lindex: -1,
                        tymed: TYMED_HGLOBAL.0 as u32,
                    };
                    data.SetData(&format, &medium, true).unwrap();
                }
            }
        }
        self.seen.lock().unwrap().dropped = paths;
        // Optimized moves answer "no effect": the source must not delete anything.
        unsafe {
            *effect = if self.answer == DROPEFFECT_MOVE {
                DROPEFFECT_NONE
            } else {
                self.answer
            };
        }
        Ok(())
    }
}

/// A foreign drop target window at x=340 on its own STA thread, until `stop`.
fn foreign_target(
    answer: DROPEFFECT,
    seen: Arc<Mutex<Seen>>,
) -> (mpsc::Sender<()>, thread::JoinHandle<()>, usize) {
    let (stop, stopped) = mpsc::channel::<()>();
    let (ready, window) = mpsc::channel();
    let handle = thread::spawn(move || {
        let _ole = Ole::new().unwrap();
        per_monitor_aware();
        let window_owner = Window::new(340);
        let target: IDropTarget = ForeignTarget { answer, seen }.into();
        unsafe { RegisterDragDrop(HWND(window_owner.0 as _), &target).unwrap() };
        ready.send(window_owner.0).unwrap();
        while stopped.try_recv().is_err() {
            pump();
            thread::sleep(Duration::from_millis(1));
        }
        unsafe {
            let _ = RevokeDragDrop(HWND(window_owner.0 as _));
        }
    });
    let hwnd = window.recv_timeout(Duration::from_secs(5)).unwrap();
    (stop, handle, hwnd)
}

#[derive(Default)]
struct NoSink;
impl FileDropSink for NoSink {
    fn try_accept(&self, _: FileDropRequest) -> Result<(), FsError> {
        Err(error("this test's own window takes no drop"))
    }
}

/// Presses on an owned source window, lets the production source run the drag, moves
/// onto the foreign target and releases `release_after` later.
fn drag_out(answer: DROPEFFECT, early: bool) -> (Seen, DragFinish) {
    let _context = NativeContext::new();
    let mut desktop = Desktop::capture();
    let _ole = Ole::new().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (stop, target_thread, target_hwnd) = foreign_target(answer, seen.clone());
    let source_window = Window::new(120);
    let fixture = crate::test_fixtures::build_temp("outgoing-");
    let source_path = fixture.path().join("drag-out 文档.txt");
    std::fs::write(&source_path, b"owned drag fixture").unwrap();
    let files = FileClipboard {
        paths: vec![source_path.clone()],
        intent: FileClipboardIntent::Copy,
    };
    let host = DragHost {
        generation: 3,
        native: NativeDragWindow::Windows {
            hwnd: source_window.0,
        },
        lifetime: Cancellation::default(),
    };
    let targets = Arc::new(Mutex::new(DropTargets::default()));
    let (start_tx, start_rx) = mpsc::channel::<Instant>();
    let (result_tx, result_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut session = Session::open(host, targets, Arc::new(NoSink)).unwrap();
        let earliest = loop {
            if let Ok(earliest) = start_rx.try_recv() {
                break earliest;
            }
            session.service_pending().unwrap();
            thread::sleep(Duration::from_millis(1));
        };
        let result = session.start_drag(&files, &Cancellation::default(), earliest);
        result_tx.send(result).unwrap();
    });
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(source_window.0 as _);
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
    unsafe { ReleaseCapture() };
    let earliest = if early {
        Instant::now() + Duration::from_secs(5)
    } else {
        Instant::now()
    };
    start_tx.send(earliest).unwrap();
    let handoff = Instant::now();
    let (mut moved, mut released) = (false, false);
    let outcome = loop {
        pump();
        if !moved && handoff.elapsed() >= Duration::from_millis(150) {
            let target = Window(target_hwnd);
            target.move_pointer(380);
            std::mem::forget(target); // owned and destroyed by its own thread
            moved = true;
        }
        if !released && handoff.elapsed() >= Duration::from_millis(450) {
            assert!(button(4));
            desktop.pressed = false;
            released = true;
        }
        if let Ok(result) = result_rx.try_recv() {
            break result.unwrap();
        }
        assert!(
            handoff.elapsed() < Duration::from_secs(6),
            "the drag did not end"
        );
        thread::sleep(Duration::from_millis(1));
    };
    worker.join().unwrap();
    stop.send(()).unwrap();
    target_thread.join().unwrap();
    let seen = seen.lock().unwrap().clone();
    if let Some(dropped) = &seen.dropped {
        assert_eq!(dropped, &vec![source_path.clone()]);
    }
    // The target, not this source, owns any transfer: the file is untouched here.
    assert!(source_path.is_file());
    (seen, outcome)
}

#[test]
#[ignore = "interactive owned-window SendInput diagnostic; run alone on an idle test desktop"]
fn real_mouse_drag_out_reaches_a_foreign_target_with_shell_formats_and_effects() {
    let (seen, outcome) = drag_out(DROPEFFECT_COPY, false);
    eprintln!("copy: {seen:?} -> {outcome:?}");
    assert!(seen.entered && seen.dropped.is_some());
    assert!(seen.shell_items, "Explorer's Shell item list");
    assert!(seen.drag_image, "the Shell drag image");
    assert_eq!(seen.offered, (DROPEFFECT_COPY | DROPEFFECT_MOVE).0);
    assert_eq!(outcome, DragFinish::Copied);

    let (seen, outcome) = drag_out(DROPEFFECT_MOVE, false);
    eprintln!("move: {seen:?} -> {outcome:?}");
    assert!(seen.dropped.is_some());
    assert_eq!(
        outcome,
        DragFinish::Moved,
        "the source panel then refreshes"
    );

    // Released before the earliest drop time: the target only sees the drag leave.
    let (seen, outcome) = drag_out(DROPEFFECT_COPY, true);
    eprintln!("early: {seen:?} -> {outcome:?}");
    assert!(seen.entered && seen.left && seen.dropped.is_none());
    assert_eq!(outcome, DragFinish::ReleasedTooSoon);
}
