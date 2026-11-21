//! Run-time loading of the distribution's media libraries, before the worker locks down.
//!
//! Read rules. A system helper starts with a cleared environment (no `LD_LIBRARY_PATH`) and
//! has no RUNPATH, so the dynamic loader resolves a soname only through the root-owned loader
//! cache and the default library folders. Every object in the process is still checked after
//! loading: its canonical path, and every folder above it, must be owned by root and writable
//! by nobody else. GStreamer plugins are checked the same way before they are loaded, and
//! only plugins on a fixed list, from the plugin folder beside libgstreamer, are loaded. The
//! seccomp lockdown that follows denies every open, so nothing is read after it.
use std::{
    ffi::{CStr, CString, OsStr, c_char, c_int, c_void},
    fmt, io, mem,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Component, Path, PathBuf},
    ptr::NonNull,
};

/// Who may own a library and the folders above it: root, and in tests also the test user
/// (libraries built for a test live in its private folder).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Trust {
    pub(super) also_owner: Option<u32>,
}

impl Trust {
    pub(super) const ROOT: Self = Self { also_owner: None };

    fn owner(&self, uid: u32) -> bool {
        uid == 0 || self.also_owner == Some(uid)
    }

    /// The canonical file and each folder above it are owned by a trusted owner and are not
    /// writable by group or others (a sticky world-writable folder such as /tmp is refused too).
    pub(super) fn check(&self, path: &Path) -> Result<PathBuf, LoadError> {
        let canonical = path
            .canonicalize()
            .map_err(|error| LoadError::Unreadable(path.to_owned(), error.kind()))?;
        let mut current = PathBuf::new();
        for component in canonical.components() {
            match component {
                Component::RootDir => current.push("/"),
                Component::Normal(name) => current.push(name),
                _ => return Err(LoadError::Untrusted(canonical.clone())),
            }
            let metadata = std::fs::metadata(&current)
                .map_err(|error| LoadError::Unreadable(current.clone(), error.kind()))?;
            if !self.owner(metadata.uid()) || metadata.mode() & 0o022 != 0 {
                return Err(LoadError::Untrusted(current));
            }
        }
        Ok(canonical)
    }
}

/// Why a library is not used. Each case is reported in the helper's tests and diagnostics;
/// the host only learns whether a decoder family was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LoadError {
    /// The loader could not find or load it (message from dlerror).
    Missing(String),
    /// A path that is not owned by root or is writable by others.
    Untrusted(PathBuf),
    Unreadable(PathBuf, io::ErrorKind),
    /// A required symbol is absent: not the library version the bindings were made for.
    Symbol(String),
    /// The library reports another version than its file name promises.
    Version(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(detail) => write!(f, "not loadable: {detail}"),
            Self::Untrusted(path) => write!(
                f,
                "{} is not owned by root or is writable by others",
                path.display()
            ),
            Self::Unreadable(path, kind) => write!(f, "{} cannot be read: {kind}", path.display()),
            Self::Symbol(name) => write!(f, "missing symbol {name}"),
            Self::Version(detail) => write!(f, "unexpected version: {detail}"),
        }
    }
}

/// One loaded shared object. Handles are never closed: GLib and GStreamer cannot be unloaded,
/// and a helper process ends after its one request or session.
pub(super) struct Library {
    handle: NonNull<c_void>,
    pub(super) path: PathBuf,
}

impl Library {
    /// Load `name`: a soname resolved by the system loader, or an absolute path. The loaded
    /// object must pass `trust`; it stays loaded (its constructors ran) even when refused,
    /// which is why `audit` refuses the whole decoder family for any untrusted object.
    pub(super) fn open(name: &OsStr, trust: Trust) -> Result<Self, LoadError> {
        let c_name = CString::new(name.as_bytes())
            .map_err(|_| LoadError::Missing("library name contains NUL".into()))?;
        // SAFETY: dlopen runs the constructors of a system library and its dependencies, which
        // the loader resolves from root-controlled configuration only (see the module notes).
        let handle = unsafe { libc::dlopen(c_name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        let Some(handle) = NonNull::new(handle) else {
            return Err(LoadError::Missing(dlerror()));
        };
        let path = loaded_path(handle).ok_or_else(|| LoadError::Missing(dlerror()))?;
        let path = trust.check(&path)?;
        Ok(Self { handle, path })
    }

    /// Resolve `name` as a value of type `T`, which must be a function pointer (or another
    /// pointer-sized `Copy` type) with the exact C signature of that symbol.
    ///
    /// # Safety
    /// `T` must match the symbol's real type in the loaded library version.
    pub(super) unsafe fn symbol<T: Copy>(&self, name: &CStr) -> Result<T, LoadError> {
        const { assert!(mem::size_of::<T>() == mem::size_of::<*mut c_void>()) };
        // SAFETY: the handle is a live dlopen handle; the name is NUL-terminated.
        let symbol = unsafe { libc::dlsym(self.handle.as_ptr(), name.as_ptr()) };
        if symbol.is_null() {
            return Err(LoadError::Symbol(name.to_string_lossy().into_owned()));
        }
        // SAFETY: a non-null symbol address reinterpreted as the caller-asserted pointer type.
        Ok(unsafe { mem::transmute_copy::<*mut c_void, T>(&symbol) })
    }

    /// Like `symbol`, for an optional symbol.
    ///
    /// # Safety
    /// As for `symbol`.
    pub(super) unsafe fn optional<T: Copy>(&self, name: &CStr) -> Option<T> {
        // SAFETY: forwarded caller contract.
        unsafe { self.symbol(name) }.ok()
    }
}

// SAFETY: a dlopen handle is process-global and valid for the process lifetime here.
unsafe impl Send for Library {}
// SAFETY: as above; dlsym is thread-safe.
unsafe impl Sync for Library {}

fn dlerror() -> String {
    // SAFETY: dlerror returns null or a NUL-terminated message owned by the loader.
    let message = unsafe { libc::dlerror() };
    if message.is_null() {
        "unknown loader error".into()
    } else {
        // SAFETY: non-null loader message, copied immediately.
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    }
}

/// glibc's `struct link_map` prefix (link.h): the public part every glibc exposes.
#[repr(C)]
struct LinkMap {
    l_addr: usize,
    l_name: *const c_char,
    l_ld: *mut c_void,
    l_next: *mut LinkMap,
    l_prev: *mut LinkMap,
}

fn loaded_path(handle: NonNull<c_void>) -> Option<PathBuf> {
    let mut map: *mut LinkMap = std::ptr::null_mut();
    // SAFETY: RTLD_DI_LINKMAP stores the handle's link_map pointer into `map`.
    let status = unsafe {
        libc::dlinfo(
            handle.as_ptr(),
            libc::RTLD_DI_LINKMAP,
            (&raw mut map).cast(),
        )
    };
    if status != 0 || map.is_null() {
        return None;
    }
    // SAFETY: the loader's link_map for a live handle; l_name is a NUL-terminated path.
    let name = unsafe { (*map).l_name };
    if name.is_null() {
        return None;
    }
    // SAFETY: as above.
    let name = unsafe { CStr::from_ptr(name) };
    let path = PathBuf::from(OsStr::from_bytes(name.to_bytes()));
    path.is_absolute().then_some(path)
}

/// The file names of every shared object currently mapped into this process.
pub(super) fn loaded_objects() -> Vec<PathBuf> {
    unsafe extern "C" fn visit(
        info: *mut libc::dl_phdr_info,
        _: libc::size_t,
        data: *mut c_void,
    ) -> c_int {
        // SAFETY: dl_iterate_phdr passes a valid info record and our Vec as `data`.
        let (info, objects) = unsafe { (&*info, &mut *data.cast::<Vec<PathBuf>>()) };
        if !info.dlpi_name.is_null() {
            // SAFETY: a NUL-terminated object name owned by the loader.
            let name = unsafe { CStr::from_ptr(info.dlpi_name) };
            if !name.is_empty() {
                objects.push(PathBuf::from(OsStr::from_bytes(name.to_bytes())));
            }
        }
        0
    }
    let mut objects = Vec::new();
    // SAFETY: the callback only reads each record and appends to the Vec passed as data.
    unsafe { libc::dl_iterate_phdr(Some(visit), (&raw mut objects).cast()) };
    objects
}

/// Every mapped object passes `trust` (the kernel's vDSO has no file and is skipped).
pub(super) fn audit(trust: Trust) -> Result<(), LoadError> {
    for object in loaded_objects() {
        if !object.is_absolute() {
            continue;
        }
        trust.check(&object)?;
    }
    Ok(())
}

/// Sonames of the FFmpeg libraries mapped into this process, by library stem
/// (`libavcodec.so.60` → `("libavcodec", 60)`). Two majors of one stem would mix versions.
pub(super) fn ffmpeg_sonames() -> Vec<(String, u32)> {
    let mut found: Vec<(String, u32)> = Vec::new();
    for object in loaded_objects() {
        let Some(name) = object.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        for stem in [
            "libavcodec",
            "libavformat",
            "libavutil",
            "libswscale",
            "libswresample",
        ] {
            let Some(rest) = name
                .strip_prefix(stem)
                .and_then(|rest| rest.strip_prefix(".so."))
            else {
                continue;
            };
            let major = rest.split('.').next().and_then(|major| major.parse().ok());
            if let Some(major) = major
                && !found
                    .iter()
                    .any(|entry| entry.0 == stem && entry.1 == major)
            {
                found.push((stem.into(), major));
            }
        }
    }
    found
}

/// Tests that use the dynamic loader in the test process, and tests that fork, hold this lock:
/// a fork while another thread holds the loader's lock would leave the child unable to load.
#[cfg(test)]
pub(super) static LOADER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn user() -> u32 {
        // SAFETY: getuid has no preconditions.
        unsafe { libc::getuid() }
    }

    #[test]
    fn system_libraries_are_trusted_and_writable_or_foreign_paths_are_not() {
        let _serial = LOADER_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let libc_path = loaded_objects()
            .into_iter()
            .find(|path| {
                path.file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|name| name.starts_with("libc.so"))
            })
            .expect("libc is mapped");
        assert!(Trust::ROOT.check(&libc_path).is_ok(), "{libc_path:?}");

        let root = std::env::temp_dir().join(format!("midnight-trust-{}", std::process::id()));
        let folder = root.join("lib");
        std::fs::create_dir_all(&folder).unwrap();
        let library = folder.join("libfake.so.1");
        std::fs::write(&library, b"not a library").unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Owned by the test user: refused for production.
        assert!(matches!(
            Trust::ROOT.check(&library),
            Err(LoadError::Untrusted(_))
        ));
        let test = Trust {
            also_owner: Some(user()),
        };
        // The rest needs a temporary folder whose ancestors are private (TMPDIR inside the
        // checkout's _build, as the WSL runs set it); a shared sticky /tmp is refused itself.
        if test.check(&std::env::temp_dir()).is_ok() {
            assert_eq!(test.check(&library), Ok(library.canonicalize().unwrap()));
            // A group-writable folder is refused even for a trusted owner.
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o775)).unwrap();
            assert_eq!(
                test.check(&library),
                Err(LoadError::Untrusted(folder.canonicalize().unwrap()))
            );
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
            // So is a world-writable library file.
            std::fs::set_permissions(&library, std::fs::Permissions::from_mode(0o666)).unwrap();
            assert_eq!(
                test.check(&library),
                Err(LoadError::Untrusted(library.canonicalize().unwrap()))
            );
        } else {
            eprintln!("TMPDIR has shared ancestors; owner-trust assertions skipped");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_missing_library_and_a_missing_symbol_are_reported() {
        let _serial = LOADER_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert!(matches!(
            Library::open(OsStr::new("libmidnight-does-not-exist.so.7"), Trust::ROOT),
            Err(LoadError::Missing(_))
        ));
        let libc = Library::open(OsStr::new("libc.so.6"), Trust::ROOT).unwrap();
        // SAFETY: getpid's real signature.
        let getpid: unsafe extern "C" fn() -> c_int = unsafe { libc.symbol(c"getpid") }.unwrap();
        // SAFETY: getpid has no preconditions.
        assert_eq!(unsafe { getpid() } as u32, std::process::id());
        // SAFETY: never called.
        let missing = unsafe { libc.symbol::<unsafe extern "C" fn()>(c"midnight_no_such_symbol") };
        assert!(matches!(missing, Err(LoadError::Symbol(_))));
        assert!(libc.path.is_absolute());
    }
}
