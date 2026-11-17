//! The descriptor and library are inseparable until every native callback quiesces.
use crate::{Error, PluginInfo, api::abi, native::Library};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};
static LIBRARIES: OnceLock<Mutex<HashMap<PathBuf, Arc<Loaded>>>> = OnceLock::new();
const MAX_LIBRARIES: usize = 256;
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
        Self::acquire_within(info, MAX_LIBRARIES)
    }
    fn acquire_within(info: &PluginInfo, limit: usize) -> Result<Arc<Self>, Error> {
        let libraries = LIBRARIES.get_or_init(|| Mutex::new(HashMap::new()));
        // A full registry refuses before any validation, load or entry call runs.
        if let Some(library) = Self::registered(&crate::session::lock(libraries), info, limit)? {
            return Ok(library);
        }
        // Trust validation, native loading and the entry call run without the registry
        // lock, so `is_loaded` and first loads of other plugins never wait behind them.
        // A racing duplicate open of the same module is reference counted by the OS
        // loader; the loser is dropped while the registered library stays pinned.
        let opened = Arc::new(Self::open(info)?);
        let mut registry = crate::session::lock(libraries);
        if let Some(library) = Self::registered(&registry, info, limit)? {
            return Ok(library);
        }
        registry.insert(info.entry_library.clone(), opened.clone());
        Ok(opened)
    }
    /// The registered library, or None when a new one may be added within `limit`.
    fn registered(
        registry: &HashMap<PathBuf, Arc<Loaded>>,
        info: &PluginInfo,
        limit: usize,
    ) -> Result<Option<Arc<Self>>, Error> {
        match registry.get(&info.entry_library) {
            Some(library) if library.id != info.id || library.version != info.version => Err(
                Error("Plugin metadata changed after loading; restart required".into()),
            ),
            Some(library) => Ok(Some(library.clone())),
            None if registry.len() >= limit => Err(Error("Loaded library limit reached".into())),
            None => Ok(None),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_registry_refuses_before_validating_or_loading_the_library() {
        let directory = tempfile::tempdir().unwrap();
        let package = directory.path().join("package");
        // Nothing exists here: any trust check, load or entry call would fail differently.
        let info = PluginInfo {
            id: "full.registry".into(),
            name: None,
            version: "1".into(),
            description: String::new(),
            models: Vec::new(),
            modes: Vec::new(),
            extensions: Vec::new(),
            priority: 0,
            entry_library: package.join("missing-plugin-library"),
            package_dir: package,
        };
        let Err(full) = Loaded::acquire_within(&info, 0) else {
            panic!("a full registry admitted a new library");
        };
        assert_eq!(full.0, "Loaded library limit reached");
        let Err(admitted) = Loaded::acquire_within(&info, MAX_LIBRARIES) else {
            panic!("a missing library loaded");
        };
        assert_ne!(
            admitted.0, full.0,
            "below the limit the request reaches validation"
        );
    }
}
