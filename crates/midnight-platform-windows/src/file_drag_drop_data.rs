//! Bounded Unicode CF_HDROP offering/reading. No clipboard or filesystem calls.
use super::*;
use std::mem::ManuallyDrop;
use windows::{
    Win32::{
        Foundation::{
            DATA_S_SAMEFORMATETC, DV_E_FORMATETC, E_NOTIMPL, HGLOBAL, OLE_E_ADVISENOTSUPPORTED,
        },
        System::{
            Com::{
                DATADIR_GET, DVASPECT_CONTENT, FORMATETC, IAdviseSink, IDataObject_Impl,
                IEnumFORMATETC, IEnumSTATDATA, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
            },
            Ole::{CF_HDROP, ReleaseStgMedium},
        },
        UI::Shell::SHCreateStdEnumFmtEtc,
    },
    core::implement,
};
use windows_sys::Win32::{
    Foundation::GlobalFree,
    System::Memory::{
        GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
    },
};

fn format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

pub(super) fn supports_files(data: &IDataObject) -> bool {
    // SAFETY: this stack format is initialized and is borrowed only for the COM call.
    unsafe { data.QueryGetData(&format()).is_ok() }
}

pub(super) fn make_data(files: &FileClipboard) -> Result<IDataObject, FsError> {
    if files.intent != FileClipboardIntent::Copy {
        return Err(error("Only copy file offers are supported."));
    }
    let bytes = crate::file_clipboard::encode_hdrop(files)?;
    Ok(Data {
        bytes: bytes.into(),
    }
    .into())
}

#[cfg(test)]
pub(super) fn malformed_offer(bytes: Vec<u8>) -> IDataObject {
    Data {
        bytes: bytes.into(),
    }
    .into()
}

#[implement(IDataObject, Agile = false)]
struct Data {
    bytes: Arc<[u8]>,
}
impl IDataObject_Impl for Data_Impl {
    fn GetData(&self, requested: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        self.QueryGetData(requested).ok()?;
        // SAFETY: the native allocation is bounded by the prevalidated encoded file list.
        let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, self.bytes.len()) };
        if memory.is_null() {
            return Err(windows::core::Error::from_thread());
        }
        let pointer = unsafe { GlobalLock(memory) };
        if pointer.is_null() {
            let failure = windows::core::Error::from_thread();
            unsafe {
                GlobalFree(memory);
            }
            return Err(failure);
        }
        unsafe {
            ptr::copy_nonoverlapping(self.bytes.as_ptr(), pointer.cast(), self.bytes.len());
            GlobalUnlock(memory);
        }
        // The caller owns this HGLOBAL and releases it through ReleaseStgMedium.
        Ok(STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 {
                hGlobal: HGLOBAL(memory),
            },
            pUnkForRelease: ManuallyDrop::new(None),
        })
    }
    fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn QueryGetData(&self, requested: *const FORMATETC) -> HRESULT {
        // SAFETY: COM callers provide a borrowed FORMATETC or null. No pointed-to target-device
        // memory is read; target-specific representations are unsupported.
        let Some(requested) = (unsafe { requested.as_ref() }) else {
            return E_POINTER;
        };
        if requested.cfFormat == CF_HDROP.0
            && requested.ptd.is_null()
            && requested.dwAspect == DVASPECT_CONTENT.0
            && requested.lindex == -1
            && requested.tymed & TYMED_HGLOBAL.0 as u32 != 0
        {
            HRESULT(0)
        } else {
            DV_E_FORMATETC
        }
    }
    fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        if !out.is_null() {
            unsafe {
                (*out).ptd = ptr::null_mut();
            }
        }
        DATA_S_SAMEFORMATETC
    }
    fn SetData(
        &self,
        _: *const FORMATETC,
        _: *const STGMEDIUM,
        _: BOOL,
    ) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn EnumFormatEtc(&self, direction: u32) -> windows::core::Result<IEnumFORMATETC> {
        if direction != DATADIR_GET.0 as u32 {
            return Err(E_NOTIMPL.into());
        }
        unsafe { SHCreateStdEnumFmtEtc(&[format()]) }
    }
    fn DAdvise(
        &self,
        _: *const FORMATETC,
        _: u32,
        _: Ref<IAdviseSink>,
    ) -> windows::core::Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
    fn DUnadvise(&self, _: u32) -> windows::core::Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
    fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

struct Medium(STGMEDIUM);
impl Drop for Medium {
    fn drop(&mut self) {
        unsafe {
            ReleaseStgMedium(&mut self.0);
        }
    }
}
struct Lock(windows_sys::Win32::Foundation::HGLOBAL);
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            GlobalUnlock(self.0);
        }
    }
}

pub(super) fn read_files(data: &IDataObject) -> Result<FileClipboard, FsError> {
    // SAFETY: OLE owns the supplied data object and returns an owned, initialized STGMEDIUM.
    let medium = Medium(unsafe { data.GetData(&format()).map_err(error)? });
    if medium.0.tymed != TYMED_HGLOBAL.0 as u32 {
        return Err(error("The native file list is not global-memory data."));
    }
    let memory = unsafe { medium.0.u.hGlobal.0 };
    if memory.is_null() {
        return Err(error("The native file list has no data."));
    }
    let length = unsafe { GlobalSize(memory) };
    if length == 0 || length > MAX_FILE_CLIPBOARD_BYTES {
        return Err(error("The native file list exceeds the 4 MiB limit."));
    }
    let pointer = unsafe { GlobalLock(memory) };
    if pointer.is_null() {
        return Err(error(io::Error::last_os_error()));
    }
    let _lock = Lock(memory);
    // The lock remains held while the bounded decoder copies each lossless native path.
    let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), length) };
    Ok(FileClipboard {
        paths: crate::file_clipboard::decode_hdrop(bytes)?,
        intent: FileClipboardIntent::Copy,
    })
}
