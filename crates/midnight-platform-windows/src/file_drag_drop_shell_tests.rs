//! Real Shell destination interoperability; only explicitly owned fixture paths are supplied.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{fs, mem::ManuallyDrop, os::windows::ffi::OsStrExt, path::PathBuf};
use windows::core::Interface;
use windows::{
    Win32::{
        System::{
            Com::{DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL},
            Ole::{DROPEFFECT_MOVE, ReleaseStgMedium},
        },
        UI::Shell::{BHID_SFUIObject, IShellItem, SHCreateItemFromParsingName},
    },
    core::PCWSTR,
};
use windows_sys::Win32::System::{
    DataExchange::RegisterClipboardFormatW,
    Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
};

#[implement(IDropSource, Agile = false)]
struct MediumRelease {
    memory: usize,
    drops: Arc<AtomicUsize>,
}
impl Drop for MediumRelease {
    fn drop(&mut self) {
        // This is the STGMEDIUM's explicit release owner; ReleaseStgMedium
        // delegates the HGLOBAL cleanup here instead of freeing it itself.
        unsafe {
            windows_sys::Win32::Foundation::GlobalFree(self.memory as _);
        }
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
impl IDropSource_Impl for MediumRelease_Impl {
    fn QueryContinueDrag(&self, _: BOOL, _: MODIFIERKEYS_FLAGS) -> HRESULT {
        DRAGDROP_S_CANCEL
    }
    fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

#[test]
fn shell_feedback_releases_only_successfully_transferred_media() {
    let _ole = Ole::new().unwrap();
    let data = make_data(&FileClipboard {
        paths: vec![PathBuf::from("C:\\owned\\file")],
        intent: FileClipboardIntent::Copy,
    })
    .unwrap();
    for (release, valid) in [(false, true), (true, true), (true, false)] {
        let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, 4) };
        assert!(!memory.is_null());
        let pointer = unsafe { GlobalLock(memory) };
        assert!(!pointer.is_null());
        unsafe {
            pointer
                .cast::<u32>()
                .write(if valid { 2 } else { 0xffff_ffff });
            GlobalUnlock(memory);
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let owner: IDropSource = MediumRelease {
            memory: memory as usize,
            drops: drops.clone(),
        }
        .into();
        let unknown = owner.cast().unwrap();
        drop(owner);
        let mut medium = STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 {
                hGlobal: windows::Win32::Foundation::HGLOBAL(memory),
            },
            pUnkForRelease: ManuallyDrop::new(Some(unknown)),
        };
        let result = unsafe { data.SetData(&format("Performed DropEffect"), &medium, release) };
        assert_eq!(result.is_ok(), valid);
        assert_eq!(drops.load(Ordering::SeqCst), usize::from(release && valid));
        if !release || !valid {
            unsafe {
                ReleaseStgMedium(&mut medium);
            }
        }
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

fn format(name: &str) -> FORMATETC {
    let name: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
    let id = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
    assert_ne!(id, 0);
    FORMATETC {
        cfFormat: id as u16,
        ptd: ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

fn feedback(data: &IDataObject, name: &str, value: u32) -> windows::core::Result<()> {
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, 4) };
    assert!(!memory.is_null());
    let pointer = unsafe { GlobalLock(memory) };
    assert!(!pointer.is_null());
    unsafe {
        pointer.cast::<u32>().write(value);
        GlobalUnlock(memory);
    }
    let mut medium = STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 {
            hGlobal: windows::Win32::Foundation::HGLOBAL(memory),
        },
        pUnkForRelease: ManuallyDrop::new(None),
    };
    let result = unsafe { data.SetData(&format(name), &medium, false) };
    // fRelease=false: the caller remains responsible even when SetData succeeds.
    unsafe {
        ReleaseStgMedium(&mut medium);
    }
    result
}

#[test]
fn outgoing_data_accepts_bounded_performed_and_logical_feedback() {
    let _ole = Ole::new().unwrap();
    let files = FileClipboard {
        paths: vec![PathBuf::from("C:\\owned\\file")],
        intent: FileClipboardIntent::Copy,
    };
    let data = make_data(&files).unwrap();
    for (name, value) in [
        ("Performed DropEffect", 0),
        ("Logical Performed DropEffect", 2),
    ] {
        feedback(&data, name, value).expect("Shell completion feedback must be accepted");
        let mut medium = unsafe { data.GetData(&format(name)).unwrap() };
        let pointer = unsafe { GlobalLock(medium.u.hGlobal.0) };
        assert!(!pointer.is_null());
        let observed = unsafe { pointer.cast::<u32>().read() };
        unsafe {
            GlobalUnlock(medium.u.hGlobal.0);
            ReleaseStgMedium(&mut medium);
        }
        assert_eq!(observed, value);
    }
    assert!(feedback(&data, "Performed DropEffect", 0xffff_ffff).is_err());
    assert_eq!(read_files(&data).unwrap(), files);
}

#[test]
fn unoptimized_move_feedback_never_deletes_and_missing_feedback_stays_unknown() {
    let _ole = Ole::new().unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("retained.txt");
    fs::write(&path, b"source retained").unwrap();
    let (data, receipt) = data::make_offer(&FileClipboard {
        paths: vec![path.clone()],
        intent: FileClipboardIntent::Copy,
    })
    .unwrap();
    assert_eq!(
        receipt.get().finish(DROPEFFECT_MOVE),
        DragFinish::Unconfirmed
    );
    assert_eq!(
        receipt.get().finish(DROPEFFECT_NONE),
        DragFinish::Unconfirmed
    );
    feedback(&data, "Performed DropEffect", 2).unwrap();
    feedback(&data, "Logical Performed DropEffect", 2).unwrap();
    assert_eq!(
        receipt.get().finish(DROPEFFECT_MOVE),
        DragFinish::SourceRetained
    );
    assert_eq!(fs::read(&path).unwrap(), b"source retained");
    feedback(&data, "Performed DropEffect", 0).unwrap();
    assert_eq!(receipt.get().finish(DROPEFFECT_NONE), DragFinish::Moved);
    assert_eq!(fs::read(path).unwrap(), b"source retained");
}

fn shell_drop(source_root: &Path, destination_root: &Path, keys: u32, moved: bool) {
    let source = tempfile::tempdir_in(source_root).unwrap();
    let destination = tempfile::tempdir_in(destination_root).unwrap();
    let path = source.path().join("drag-文档.txt");
    fs::write(&path, b"native Shell owned payload").unwrap();
    let directory = source.path().join("folder-文档");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("child.txt"), b"nested Shell payload").unwrap();
    let wide: Vec<_> = destination
        .path()
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let folder: IShellItem =
        unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).unwrap() };
    let target: IDropTarget = unsafe { folder.BindToHandler(None, &BHID_SFUIObject).unwrap() };
    let files = FileClipboard {
        paths: vec![path.clone(), directory.clone()],
        intent: FileClipboardIntent::Copy,
    };
    let (data, receipt) = data::make_offer(&files).unwrap();
    let point = POINTL { x: 0, y: 0 };
    let mut effect = outgoing_effects();
    unsafe {
        target
            .DragEnter(&data, MODIFIERKEYS_FLAGS(keys | 1), point, &mut effect)
            .unwrap();
    }
    assert_eq!(
        effect,
        if moved {
            DROPEFFECT_MOVE
        } else {
            DROPEFFECT_COPY
        },
        "actual Shell default/modifier effect"
    );
    unsafe {
        target
            .Drop(&data, MODIFIERKEYS_FLAGS(keys), point, &mut effect)
            .unwrap();
    }
    eprintln!(
        "Shell completion: effect={effect:?}, feedback={:?}",
        receipt.get()
    );
    assert_eq!(
        receipt.get().finish(effect),
        if moved {
            DragFinish::Moved
        } else {
            DragFinish::Copied
        }
    );
    assert_eq!(
        fs::read(destination.path().join("drag-文档.txt")).unwrap(),
        b"native Shell owned payload"
    );
    assert_eq!(
        path.exists(),
        !moved,
        "only the actual Shell target may remove source"
    );
    assert_eq!(directory.exists(), !moved);
    assert_eq!(
        fs::read(destination.path().join("folder-文档/child.txt")).unwrap(),
        b"nested Shell payload"
    );
    if !moved {
        assert_eq!(fs::read(path).unwrap(), b"native Shell owned payload");
    }
}

#[test]
#[ignore = "actual Shell transfers on an interactive desktop; owned fixture paths only"]
fn native_shell_same_volume_default_moves_and_control_copies() {
    let _ole = Ole::new().unwrap();
    let root = std::env::temp_dir();
    shell_drop(&root, &root, 0, true);
    shell_drop(&root, &root, 8, false);
}

#[test]
#[ignore = "requires owned NATIVE_MOVE_OTHER_VOLUME_ROOT on a distinct volume and a desktop"]
fn native_shell_cross_volume_default_copies_and_shift_moves() {
    let _ole = Ole::new().unwrap();
    let root = std::env::temp_dir();
    let other = PathBuf::from(
        std::env::var_os("NATIVE_MOVE_OTHER_VOLUME_ROOT").expect("owned second-volume root"),
    );
    let cancel = Cancellation::default();
    assert_ne!(
        WindowsFileSystem::new(None)
            .native_drop_device(&root, &cancel)
            .unwrap(),
        WindowsFileSystem::new(None)
            .native_drop_device(&other, &cancel)
            .unwrap(),
        "fixture must span actual volumes"
    );
    shell_drop(&root, &other, 0, false);
    shell_drop(&root, &other, 4, true);
}
