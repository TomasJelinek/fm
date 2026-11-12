//! The descriptor and library are inseparable until every native callback quiesces.
use crate::{Error, PluginInfo, api::abi, native::Library};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};
static LIBRARIES: OnceLock<Mutex<HashMap<PathBuf, Arc<Loaded>>>> = OnceLock::new();
pub(crate) struct Loaded {
    pub api: abi::PluginApi,
    _library: Library,
    id: String,
    version: String,
}
// SAFETY: metadata/code are immutable; dispatcher alone invokes instance entry calls.
unsafe impl Send for Loaded {}
unsafe impl Sync for Loaded {}
impl Loaded {
    pub fn is_loaded(info: &PluginInfo) -> bool {
        LIBRARIES.get().is_some_and(|libraries| {
            crate::session::lock(libraries)
                .get(&info.entry_library)
                .is_some_and(|library| library.id == info.id && library.version == info.version)
        })
    }
    pub fn acquire(info: &PluginInfo) -> Result<Arc<Self>, Error> {
        let mut libraries =
            crate::session::lock(LIBRARIES.get_or_init(|| Mutex::new(HashMap::new())));
        if let Some(library) = libraries.get(&info.entry_library) {
            if library.id != info.id || library.version != info.version {
                return Err(Error(
                    "Plugin metadata changed after loading; restart required".into(),
                ));
            }
            return Ok(library.clone());
        }
        if libraries.len() >= 256 {
            return Err(Error("Loaded library limit reached".into()));
        }
        let library = Arc::new(Self::open(info)?);
        libraries.insert(info.entry_library.clone(), library.clone());
        Ok(library)
    }
    pub fn open(info: &PluginInfo) -> Result<Self, Error> {
        crate::trust::check(&info.entry_library)?;
        crate::trust::check(&info.package_dir)?;
        if info.entry_library.canonicalize()? != info.entry_library {
            return Err(Error("Library changed to redirected path".into()));
        }
        crate::package_trust::check(&info.package_dir)?;
        let library = Library::open(&info.entry_library)?;
        let entry = library.entry()?;
        // SAFETY: integer/pointer/optional-function table has an all-zero valid initial value.
        let mut api: abi::PluginApi = unsafe { std::mem::zeroed() };
        // SAFETY: aligned writable table, retained library, explicit caller-owned capacity.
        status(unsafe { entry(&mut api, size_of::<abi::PluginApi>() as u32) })?;
        api.validate(size_of::<abi::PluginApi>() as u32, abi::KIND_VIEWER)
            .map_err(|s| Error(format!("Invalid ABI descriptor ({s})")))?;
        // SAFETY: trusted library promises its validated immutable metadata remains mapped.
        let id = unsafe { bytes(api.id, 128) }?;
        let version = unsafe { bytes(api.version, 64) }?;
        if id != info.id.as_bytes() || version != info.version.as_bytes() {
            return Err(Error(
                "Descriptor identity/version differs from manifest".into(),
            ));
        }
        Ok(Self {
            api,
            _library: library,
            id: info.id.clone(),
            version: info.version.clone(),
        })
    }
}
pub(crate) fn status(status: abi::Status) -> Result<(), Error> {
    if status == abi::OK {
        Ok(())
    } else {
        Err(Error(format!("Plugin status {status}")))
    }
}
pub(crate) fn slice(bytes: &[u8]) -> abi::ByteSlice {
    abi::ByteSlice {
        data: bytes.as_ptr(),
        len: bytes.len() as u32,
        reserved: 0,
    }
}
pub(crate) fn empty() -> abi::Buffer {
    abi::Buffer {
        id: 0,
        bytes: slice(&[]),
    }
}
/// Caller guarantees mapped native memory for the validated range during the borrow.
pub(crate) unsafe fn bytes<'a>(value: abi::ByteSlice, max: u32) -> Result<&'a [u8], Error> {
    value
        .validate(max)
        .map_err(|_| Error("Invalid native buffer".into()))?;
    if value.len == 0 {
        return Ok(&[]);
    }
    // SAFETY: caller's mapping contract, bounded non-null length checked above.
    Ok(unsafe { std::slice::from_raw_parts(value.data, value.len as usize) })
}
