//! Bounded Unicode CF_HDROP offering/reading. No clipboard or filesystem calls.
use super::*;
use std::{mem::ManuallyDrop, rc::Rc};
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
    System::DataExchange::RegisterClipboardFormatW,
    System::Memory::{
        GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
    },
};

#[derive(Clone, Copy, Default, Debug)]
pub(super) struct Feedback {
    performed: Option<u32>,
    logical: Option<u32>,
}
impl Feedback {
    pub(super) fn finish(self, effect: DROPEFFECT) -> DragFinish {
        // These are receiver reports, never authority to delete a source. Conventional
        // MOVE asks the source to delete its copy; this application deliberately refuses.
        if effect.0 == 2 && self.performed == Some(2) {
            return DragFinish::SourceRetained;
        }
        match (self.performed, self.logical, effect.0) {
            (Some(2), Some(2), 0 | 1) => DragFinish::Moved,
            (Some(0), Some(2) | None, _) | (None, Some(2), _) => DragFinish::Moved,
            (Some(1), Some(1) | None, _) | (None, Some(1), _) => DragFinish::Copied,
            (None, None, 1) => DragFinish::Copied,
            _ => DragFinish::Unconfirmed,
        }
    }
}

fn feedback_formats() -> Result<[u16; 2], FsError> {
    // Registered names are fixed OS protocol identifiers; no target-controlled allocation.
    let ids = unsafe {
        [
            RegisterClipboardFormatW(w!("Performed DropEffect").as_ptr()),
            RegisterClipboardFormatW(w!("Logical Performed DropEffect").as_ptr()),
        ]
    };
    if ids.contains(&0) {
        return Err(error(io::Error::last_os_error()));
    }
    Ok(ids.map(|id| id as u16))
}

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

#[cfg(test)]
pub(super) fn make_data(files: &FileClipboard) -> Result<IDataObject, FsError> {
    make_offer(files).map(|(data, _)| data)
}

pub(super) fn make_offer(
    files: &FileClipboard,
) -> Result<(IDataObject, Rc<Cell<Feedback>>), FsError> {
    if files.intent != FileClipboardIntent::Copy {
        return Err(error("Only copy file offers are supported."));
    }
    let bytes = crate::file_clipboard::encode_hdrop(files)?;
    let feedback = Rc::new(Cell::new(Feedback::default()));
    let data = Data {
        bytes: bytes.into(),
        formats: feedback_formats()?,
        feedback: feedback.clone(),
    }
    .into();
    Ok((data, feedback))
}

#[cfg(test)]
pub(super) fn malformed_offer(bytes: Vec<u8>) -> IDataObject {
    Data {
        bytes: bytes.into(),
        formats: feedback_formats().unwrap(),
        feedback: Rc::default(),
    }
    .into()
}

#[implement(IDataObject, Agile = false)]
struct Data {
    bytes: Arc<[u8]>,
    formats: [u16; 2],
    feedback: Rc<Cell<Feedback>>,
}
impl IDataObject_Impl for Data_Impl {
    fn GetData(&self, requested: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        self.QueryGetData(requested).ok()?;
        let id = unsafe { (*requested).cfFormat };
        if id != CF_HDROP.0 {
            let feedback = self.feedback.get();
            let value = if id == self.formats[0] {
                feedback.performed
            } else {
                feedback.logical
            };
            return medium_bytes(
                &value
                    .ok_or_else(|| windows::core::Error::from(DV_E_FORMATETC))?
                    .to_le_bytes(),
            );
        }
        medium_bytes(&self.bytes)
    }
    fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn QueryGetData(&self, requested: *const FORMATETC) -> HRESULT {
        let Some(requested) = (unsafe { requested.as_ref() }) else {
            return E_POINTER;
        };
        let feedback = self.feedback.get();
        let available = requested.cfFormat == CF_HDROP.0
            || (requested.cfFormat == self.formats[0] && feedback.performed.is_some())
            || (requested.cfFormat == self.formats[1] && feedback.logical.is_some());
        if available && valid_format(requested) {
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
        requested: *const FORMATETC,
        medium: *const STGMEDIUM,
        release: BOOL,
    ) -> windows::core::Result<()> {
        // COM provides borrowed descriptors. Copy only a bounded DWORD; never retain target
        // pointers or use completion feedback to perform filesystem operations.
        let (Some(requested), Some(medium)) =
            (unsafe { requested.as_ref() }, unsafe { medium.as_ref() })
        else {
            return Err(E_POINTER.into());
        };
        if !valid_format(requested)
            || !self.formats.contains(&requested.cfFormat)
            || medium.tymed != TYMED_HGLOBAL.0 as u32
        {
            return Err(DV_E_FORMATETC.into());
        }
        let memory = unsafe { medium.u.hGlobal.0 };
        if memory.is_null() || !(4..=64).contains(&unsafe { GlobalSize(memory) }) {
            return Err(DV_E_FORMATETC.into());
        }
        let pointer = unsafe { GlobalLock(memory) };
        if pointer.is_null() {
            return Err(windows::core::Error::from_thread());
        }
        let value = unsafe { pointer.cast::<u32>().read_unaligned() };
        unsafe {
            GlobalUnlock(memory);
        }
        if !matches!(value, 0..=2) {
            return Err(DV_E_FORMATETC.into());
        }
        let mut feedback = self.feedback.get();
        if requested.cfFormat == self.formats[0] {
            feedback.performed = Some(value);
        } else {
            feedback.logical = Some(value);
        }
        self.feedback.set(feedback);
        if release.as_bool() {
            // On success only, fRelease transfers ownership. Release a shallow descriptor
            // copy: its ManuallyDrop COM field must not be cloned or released a second time.
            let mut owned = unsafe { ptr::read(medium) };
            unsafe {
                ReleaseStgMedium(&mut owned);
            }
        }
        Ok(())
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

fn valid_format(requested: &FORMATETC) -> bool {
    requested.ptd.is_null()
        && requested.dwAspect == DVASPECT_CONTENT.0
        && requested.lindex == -1
        && requested.tymed & TYMED_HGLOBAL.0 as u32 != 0
}

fn medium_bytes(bytes: &[u8]) -> windows::core::Result<STGMEDIUM> {
    // SAFETY: the native allocation is bounded by the prevalidated encoded file list.
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len()) };
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
        ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast(), bytes.len());
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
