//! Shell integration of an outgoing drag: a Shell data object with item IDs and a drag
//! image. No file I/O beyond the Shell's own parsing of the dragged paths.
use super::*;
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
};
use windows::{
    Win32::{
        Foundation::POINT as WPOINT,
        System::Com::{
            CLSCTX_INPROC_SERVER, CoCreateInstance, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL,
        },
        UI::Shell::{
            CLSID_DragDropHelper, Common::ITEMIDLIST, DSH_ALLOWDROPDESCRIPTIONTEXT, GPFIDL_DEFAULT,
            IDataObjectAsyncCapability, IDragSourceHelper2, ILFindLastID, ILFree, ILIsParent,
            SHCreateDataObject, SHGetPathFromIDListEx, SHParseDisplayName,
        },
    },
    core::Interface,
};
use windows_sys::Win32::{
    System::DataExchange::RegisterClipboardFormatW,
    System::Memory::{GlobalLock, GlobalUnlock},
};

/// Item IDs are parsed only for this many dragged items: each parse may touch the
/// filesystem, and a larger drag still offers its complete CF_HDROP list.
pub(super) const MAX_SHELL_ITEMS: usize = 256;
/// The longest path the Shell's item-ID path conversion returns.
const MAX_SHELL_PATH: usize = 32_768;

/// Absolute item IDs, freed on drop.
struct ItemIds(Vec<*mut ITEMIDLIST>);
impl Drop for ItemIds {
    fn drop(&mut self) {
        for pidl in self.0.drain(..) {
            // SAFETY: each pointer came from SHParseDisplayName and is freed once.
            unsafe { ILFree(Some(pidl)) };
        }
    }
}

/// Parses `path` into an absolute item ID owned by `ids`, only when it converts back to
/// exactly the same path.
fn parse(ids: &mut ItemIds, path: &std::path::Path) -> Option<*mut ITEMIDLIST> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    // UNC (two leading backslashes) and verbatim paths stay CF_HDROP only.
    if wide.len() >= MAX_SHELL_PATH - 1 || wide.starts_with(&[92, 92]) || wide.contains(&0) {
        return None;
    }
    let mut terminated = wide;
    terminated.push(0);
    let mut pidl = ptr::null_mut();
    // SAFETY: a NUL-terminated owned path; the out pointer is owned by `ids` at once.
    unsafe {
        SHParseDisplayName(
            windows::core::PCWSTR(terminated.as_ptr()),
            None,
            &mut pidl,
            0,
            None,
        )
    }
    .ok()?;
    if pidl.is_null() {
        return None;
    }
    ids.0.push(pidl);
    let mut back = vec![0u16; MAX_SHELL_PATH];
    // SAFETY: the item ID is live; the buffer length bounds the written path.
    if !unsafe { SHGetPathFromIDListEx(pidl, &mut back, GPFIDL_DEFAULT) }.as_bool() {
        return None;
    }
    let length = back.iter().position(|unit| *unit == 0)?;
    (OsString::from_wide(&back[..length]) == *path.as_os_str()).then_some(pidl)
}

/// The Shell items of `files` in their common folder: the folder's item ID first, then
/// each item's. Only when every item lies directly in one folder and each path converts
/// back exactly: the Shell then derives the same CF_HDROP list as ours, so nothing
/// dragged is renamed by its parsing. None for network paths (their parsing may wait on
/// the network while the user holds the button), long lists, or any doubtful path.
fn item_ids(files: &FileClipboard) -> Option<ItemIds> {
    if files.paths.is_empty() || files.paths.len() > MAX_SHELL_ITEMS {
        return None;
    }
    let folder = files.paths[0].parent()?;
    if files.paths.iter().any(|path| path.parent() != Some(folder)) {
        return None;
    }
    let mut ids = ItemIds(Vec::with_capacity(files.paths.len() + 1));
    let parent = parse(&mut ids, folder)?;
    for path in &files.paths {
        let pidl = parse(&mut ids, path)?;
        // SAFETY: both item IDs are live and owned by `ids`.
        if !unsafe { ILIsParent(parent, pidl, true) }.as_bool() {
            return None;
        }
    }
    Some(ids)
}

/// The offer as a Shell data object. When every path has an exact Shell item, the
/// Shell builds it from those items: Explorer then gets its item list, and the Shell's
/// CF_HDROP list equals ours, as `item_ids` checked. Otherwise the Shell object wraps
/// our lossless CF_HDROP offer (`inner`) and adds only what the drag image needs. A
/// Shell object stores what targets and the drag helper set on it. It is made
/// synchronous: a target finishes its operation before the drag returns, so its
/// completion feedback is read on return. Falls back to `inner` alone when the Shell
/// refuses.
pub(super) fn wrap(files: &FileClipboard, inner: IDataObject) -> IDataObject {
    let ids = item_ids(files);
    let outer: windows::core::Result<IDataObject> = match &ids {
        Some(ids) => {
            // SAFETY: the last IDs point into the live absolute item IDs.
            let children: Vec<*const ITEMIDLIST> = ids.0[1..]
                .iter()
                .map(|pidl| unsafe { ILFindLastID(*pidl) }.cast_const())
                .collect();
            // SAFETY: the item IDs stay alive for the call; the Shell copies them.
            unsafe {
                SHCreateDataObject(
                    Some(ids.0[0].cast_const()),
                    Some(&children),
                    None::<&IDataObject>,
                )
            }
        }
        // SAFETY: the inner object is a live COM object on this STA.
        None => unsafe { SHCreateDataObject(None, None, &inner) },
    };
    let Ok(outer) = outer else {
        return inner;
    };
    if let Ok(capability) = outer.cast::<IDataObjectAsyncCapability>() {
        // SAFETY: a plain flag on the Shell object created above.
        let _ = unsafe { capability.SetAsyncMode(false) };
    }
    outer
}

fn registered(name: windows::core::PCWSTR) -> Option<u16> {
    // SAFETY: a NUL-terminated fixed format name.
    let id = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
    u16::try_from(id).ok().filter(|id| *id != 0)
}

fn hglobal_format(id: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: id,
        ptd: ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

/// A DWORD format of `data`, as a drop target or the drag helper stored it.
pub(super) fn read_dword(data: &IDataObject, name: windows::core::PCWSTR) -> Option<u32> {
    let format = hglobal_format(registered(name)?);
    // SAFETY: the medium is owned and released here; only 4 bytes are copied out.
    unsafe {
        let mut medium = data.GetData(&format).ok()?;
        let mut value = None;
        if medium.tymed == TYMED_HGLOBAL.0 as u32 && !medium.u.hGlobal.0.is_null() {
            let memory = medium.u.hGlobal.0;
            if windows_sys::Win32::System::Memory::GlobalSize(memory) >= 4 {
                let pointer = GlobalLock(memory);
                if !pointer.is_null() {
                    value = Some(pointer.cast::<u32>().read_unaligned());
                    GlobalUnlock(memory);
                }
            }
        }
        windows::Win32::System::Ole::ReleaseStgMedium(&mut medium);
        value
    }
}

/// The Shell's drag image for `data` (icons or thumbnails of the items and their
/// count), with room for the target's drop description ("Move to Documents").
/// A missing helper only leaves the drag without an image.
pub(super) fn attach_drag_image(data: &IDataObject) {
    // SAFETY: an in-process Shell helper on this STA; the data object outlives the call.
    unsafe {
        let Ok(helper) = CoCreateInstance::<_, IDragSourceHelper2>(
            &CLSID_DragDropHelper,
            None,
            CLSCTX_INPROC_SERVER,
        ) else {
            return;
        };
        let _ = helper.SetFlags(DSH_ALLOWDROPDESCRIPTIONTEXT.0 as u32);
        let mut cursor = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
        windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut cursor);
        let point = WPOINT {
            x: cursor.x,
            y: cursor.y,
        };
        let _ = helper.InitializeFromWindow(None, Some(&point), data);
    }
}

/// Whether the drag helper draws the drag image as a layered window, when the source
/// shows the plain arrow instead of the default drag cursors under it.
pub(super) fn showing_layered(data: &IDataObject) -> bool {
    read_dword(data, w!("IsShowingLayered")).is_some_and(|value| value != 0)
}

/// Feedback the target left on the offer for this source (`Performed DropEffect`,
/// `Logical Performed DropEffect`), whichever object stored it.
pub(super) fn feedback(data: &IDataObject) -> (Option<u32>, Option<u32>) {
    let valid = |value: Option<u32>| value.filter(|value| *value <= 2);
    (
        valid(read_dword(data, w!("Performed DropEffect"))),
        valid(read_dword(data, w!("Logical Performed DropEffect"))),
    )
}
