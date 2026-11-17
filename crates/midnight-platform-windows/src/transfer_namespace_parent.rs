//! Pooled metadata-only directory authority and explicitly qualified native lookup rules.
use super::*;
use std::{
    fs::OpenOptions,
    mem::size_of,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
        io::AsRawHandle,
    },
};
use windows_sys::Win32::System::{
    IO::DeviceIoControl,
    Ioctl::{
        FILE_FS_PERSISTENT_VOLUME_INFORMATION, FSCTL_QUERY_PERSISTENT_VOLUME_STATE,
        PERSISTENT_VOLUME_STATE_SHORT_NAME_CREATION_DISABLED,
    },
};

#[cfg(debug_assertions)]
static LOOKUP_TRACE_ENABLED: OnceLock<bool> = OnceLock::new();

#[cfg(debug_assertions)]
fn lookup_trace_enabled() -> bool {
    *LOOKUP_TRACE_ENABLED.get_or_init(|| {
        std::env::var_os("MIDNIGHT_TEST_TRACE_NATIVE_LOOKUP").as_deref() == Some(OsStr::new("1"))
    })
}

#[cfg(debug_assertions)]
fn lookup_trace_first(stage: u32) -> bool {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEEN: AtomicU32 = AtomicU32::new(0);
    let bit = 1_u32 << stage;
    LOOKUP_TRACE_ENABLED.get().copied().unwrap_or(false)
        && SEEN.fetch_or(bit, Ordering::Relaxed) & bit == 0
}

#[cfg(debug_assertions)]
macro_rules! trace_lookup {
    ($stage:expr, $($args:tt)*) => {
        if lookup_trace_first($stage) {
            eprintln!("[midnight-native-lookup] {}", format_args!($($args)*));
        }
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LookupMode {
    ShortNamesDisabled { case_sensitive: bool },
    Unqualified,
}
impl LookupMode {
    pub(super) fn key(self, name: &OsStr) -> Option<Arc<[u8]>> {
        let Self::ShortNamesDisabled { case_sensitive } = self else {
            #[cfg(debug_assertions)]
            trace_lookup!(8, "key refused mode={self:?} name={name:?}");
            return None;
        };
        let mut result = Vec::new();
        for mut unit in name.encode_wide() {
            // An insensitive NTFS directory uses a native upcase table. Only ASCII is
            // qualified here; Unicode lowercasing is not evidence of native equivalence.
            if !case_sensitive {
                if unit > 0x7f {
                    #[cfg(debug_assertions)]
                    trace_lookup!(9, "key refused non_ascii unit={unit:#06x} name={name:?}");
                    return None;
                }
                if (b'a' as u16..=b'z' as u16).contains(&unit) {
                    unit -= 32;
                }
            }
            result.extend_from_slice(&unit.to_le_bytes());
        }
        Some(result.into())
    }
    fn domain(self) -> Arc<[u8]> {
        match self {
            Self::ShortNamesDisabled { case_sensitive } => {
                let mut domain = b"windows.ntfs.short-names-disabled.v1".to_vec();
                domain.push(u8::from(case_sensitive));
                domain.into()
            }
            Self::Unqualified => Arc::from([]),
        }
    }
}

struct Anchor {
    file: File,
    identity: TransferIdentity,
    physical: PathBuf,
}
struct Volume {
    file: File,
    root: Arc<Anchor>,
}
pub(super) struct Parent {
    anchor: Arc<Anchor>,
    // Flat Arc owners avoid a recursive native-handle ownership chain.
    chain: Vec<Arc<Anchor>>,
    pub(super) snapshot: Arc<TransferNamespaceParent>,
    pub(super) mode: LookupMode,
    volume: Option<Arc<Volume>>,
}
type Pool<T> = Mutex<HashMap<TransferIdentity, Weak<T>>>;
static ANCHORS: OnceLock<Pool<Anchor>> = OnceLock::new();
static PARENTS: OnceLock<Pool<Parent>> = OnceLock::new();
static VOLUMES: OnceLock<Pool<Volume>> = OnceLock::new();

fn metadata_directory(path: &Path) -> Result<File, FsError> {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_TRAVERSE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|e| error(path, e))?;
    let metadata = file.metadata().map_err(|e| error(path, e))?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(invalid(
            path,
            "native namespace ancestor is not an ordinary directory",
        ));
    }
    Ok(file)
}

fn anchor(path: &Path) -> Result<Arc<Anchor>, FsError> {
    let file = metadata_directory(path)?;
    let identity = identity(&file, path)?;
    let pool = ANCHORS.get_or_init(Default::default);
    if let Some(found) = pool
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&identity)
        .and_then(Weak::upgrade)
    {
        return Ok(found);
    }
    let candidate = Arc::new(Anchor {
        file,
        identity,
        physical: path.to_owned(),
    });
    let mut pool = pool.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = pool.get(&identity).and_then(Weak::upgrade) {
        return Ok(found);
    }
    pool.insert(identity, Arc::downgrade(&candidate));
    Ok(candidate)
}

pub(super) fn capture(
    file: &File,
    path: &Path,
    cancel: &Cancellation,
) -> Result<Arc<Parent>, FsError> {
    checked(cancel, path)?;
    let expected = identity(file, path)?;
    let pool = PARENTS.get_or_init(Default::default);
    if let Some(found) = pool
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&expected)
        .and_then(Weak::upgrade)
    {
        return Ok(found);
    }
    let physical = physical_path(file).map_err(|e| error(path, e))?;
    let mut chain = Vec::new();
    for component in physical.ancestors().filter(|path| path.has_root()) {
        checked(cancel, path)?;
        chain.push(anchor(component)?);
    }
    let first = chain
        .first()
        .ok_or_else(|| invalid(path, "native parent has no root"))?;
    if first.identity != expected {
        return Err(invalid(
            path,
            "native parent changed while capturing its ancestry",
        ));
    }
    let root = chain
        .last()
        .ok_or_else(|| invalid(path, "native parent has no root"))?;
    let volume = match open_volume(root) {
        Ok(volume) => Some(volume),
        Err(_failure) => {
            #[cfg(debug_assertions)]
            trace_lookup!(
                2,
                "volume refused root={:?} identity={:?} kind={:?} message={:?}",
                root.physical,
                root.identity,
                _failure.kind,
                _failure.message
            );
            None
        }
    };
    let mode = lookup_mode(&first.file, volume.as_deref());
    #[cfg(debug_assertions)]
    trace_lookup!(
        0,
        "capture parent={physical:?} identity={expected:?} root={:?} root_identity={:?} mode={mode:?}",
        root.physical,
        root.identity
    );
    let snapshot = Arc::new(TransferNamespaceParent {
        identity: expected,
        lookup_domain: mode.domain(),
        ancestors: chain
            .iter()
            .map(|anchor| anchor.identity)
            .collect::<Vec<_>>()
            .into(),
    });
    let candidate = Arc::new(Parent {
        anchor: first.clone(),
        chain,
        snapshot,
        mode,
        volume,
    });
    candidate.validate(cancel)?;
    let mut pool = pool.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = pool.get(&expected).and_then(Weak::upgrade) {
        return Ok(found);
    }
    pool.insert(expected, Arc::downgrade(&candidate));
    Ok(candidate)
}

impl Parent {
    pub(super) fn file(&self) -> &File {
        &self.anchor.file
    }
    pub(super) fn physical(&self) -> &Path {
        &self.anchor.physical
    }
    pub(super) fn validate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        for anchor in &self.chain {
            checked(cancel, &anchor.physical)?;
            if identity(&anchor.file, &anchor.physical)? != anchor.identity {
                return Err(invalid(
                    &anchor.physical,
                    "pinned namespace ancestor changed",
                ));
            }
        }
        for edge in self.chain.windows(2) {
            let child = &edge[0];
            let parent = &edge[1];
            let name = child
                .physical
                .file_name()
                .ok_or_else(|| invalid(&child.physical, "native namespace edge has no basename"))?;
            let visible = native::open_relative(
                &parent.file,
                name,
                FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_OPEN,
                0,
            )
            .map_err(|e| error(&child.physical, e))?;
            if identity(&visible, &child.physical)? != child.identity {
                return Err(invalid(
                    &child.physical,
                    "original namespace ancestor binding changed",
                ));
            }
        }
        let root = self
            .chain
            .last()
            .ok_or_else(|| invalid(self.physical(), "missing root"))?;
        let visible = metadata_directory(&root.physical)?;
        if identity(&visible, &root.physical)? != root.identity {
            return Err(invalid(
                &root.physical,
                "native namespace root binding changed",
            ));
        }
        if lookup_mode(self.file(), self.volume.as_deref()) != self.mode {
            return Err(invalid(
                self.physical(),
                "native case or short-name rules changed",
            ));
        }
        Ok(())
    }
}

fn open_volume(root: &Arc<Anchor>) -> Result<Arc<Volume>, FsError> {
    #[cfg(debug_assertions)]
    lookup_trace_enabled();
    let pool = VOLUMES.get_or_init(Default::default);
    if let Some(found) = pool
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&root.identity)
        .and_then(Weak::upgrade)
    {
        return Ok(found);
    }
    let path: Vec<_> = root.physical.as_os_str().encode_wide().collect();
    // Retain the physical volume GUID qualification, but query the filesystem's
    // already pinned root directory. A DASD volume-device handle does not serve
    // this FSCTL on the qualified native path.
    if !path.starts_with(&r"\\?\Volume{".encode_utf16().collect::<Vec<_>>()) {
        #[cfg(debug_assertions)]
        trace_lookup!(
            1,
            "volume path refused root={:?} identity={:?}",
            root.physical,
            root.identity
        );
        return Err(unsupported(
            &root.physical,
            "native volume rule query unavailable",
        ));
    }
    // Duplicate the existing authority rather than reopening its display path or
    // broadening access. The duplicate retains the same directory object/rights.
    let file = root.file.try_clone().map_err(|e| {
        #[cfg(debug_assertions)]
        trace_lookup!(
            3,
            "root directory duplicate failed root={:?} identity={:?} raw_os_error={:?} kind={:?}",
            root.physical,
            root.identity,
            e.raw_os_error(),
            e.kind()
        );
        error(&root.physical, e)
    })?;
    #[cfg(debug_assertions)]
    trace_lookup!(
        3,
        "pinned root directory duplicated root={:?} identity={:?}",
        root.physical,
        root.identity
    );
    let volume = Arc::new(Volume {
        file,
        root: root.clone(),
    });
    let mut pool = pool.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = pool.get(&root.identity).and_then(Weak::upgrade) {
        return Ok(found);
    }
    pool.insert(root.identity, Arc::downgrade(&volume));
    Ok(volume)
}

// Remove only this allocation's weak entry. Per-insert whole-pool scans would turn a
// large plan with many native parents into quadratic work.
impl Drop for Anchor {
    fn drop(&mut self) {
        if let Some(pool) = ANCHORS.get() {
            let mut pool = pool.lock().unwrap_or_else(|e| e.into_inner());
            if pool
                .get(&self.identity)
                .is_some_and(|value| std::ptr::eq(value.as_ptr(), self))
            {
                pool.remove(&self.identity);
            }
        }
    }
}
impl Drop for Parent {
    fn drop(&mut self) {
        if let Some(pool) = PARENTS.get() {
            let mut pool = pool.lock().unwrap_or_else(|e| e.into_inner());
            if pool
                .get(&self.snapshot.identity)
                .is_some_and(|value| std::ptr::eq(value.as_ptr(), self))
            {
                pool.remove(&self.snapshot.identity);
            }
        }
    }
}
impl Drop for Volume {
    fn drop(&mut self) {
        if let Some(pool) = VOLUMES.get() {
            let mut pool = pool.lock().unwrap_or_else(|e| e.into_inner());
            if pool
                .get(&self.root.identity)
                .is_some_and(|value| std::ptr::eq(value.as_ptr(), self))
            {
                pool.remove(&self.root.identity);
            }
        }
    }
}

fn persistent_short_names_disabled(
    query_result: i32,
    returned: u32,
    response: &FILE_FS_PERSISTENT_VOLUME_INFORMATION,
) -> bool {
    // Version and FlagMask describe the request; successful native replies may
    // leave those output fields zero. Only consume policy flags after a complete
    // successful reply, never from an error or a caller-preinitialized buffer.
    query_result != 0
        && returned >= size_of::<FILE_FS_PERSISTENT_VOLUME_INFORMATION>() as u32
        && response.VolumeFlags & PERSISTENT_VOLUME_STATE_SHORT_NAME_CREATION_DISABLED != 0
}

fn lookup_mode(directory: &File, volume: Option<&Volume>) -> LookupMode {
    #[cfg(debug_assertions)]
    lookup_trace_enabled();
    let Some(volume) = volume else {
        #[cfg(debug_assertions)]
        trace_lookup!(4, "lookup refused missing_volume");
        return LookupMode::Unqualified;
    };
    let mut fs_name = [0u16; 32];
    #[cfg(debug_assertions)]
    let trace_volume = lookup_trace_first(5);
    // SAFETY: live native directory, writable bounded filesystem-name buffer; other
    // optional outputs are null. Failure means unqualified lookup, never inferred NTFS.
    let volume_result = unsafe {
        GetVolumeInformationByHandleW(
            directory.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs_name.as_mut_ptr(),
            fs_name.len() as u32,
        )
    };
    #[cfg(debug_assertions)]
    let volume_error = if trace_volume && volume_result == 0 {
        io::Error::last_os_error().raw_os_error()
    } else {
        None
    };
    let filesystem = fs_name.split(|u| *u == 0).next().unwrap_or_default();
    #[cfg(debug_assertions)]
    if trace_volume {
        eprintln!(
            "[midnight-native-lookup] volume_info result={volume_result} last_error={volume_error:?} filesystem={:?} root={:?} root_identity={:?}",
            String::from_utf16_lossy(filesystem),
            volume.root.physical,
            volume.root.identity
        );
    }
    if volume_result == 0 || !filesystem.iter().copied().eq("NTFS".encode_utf16()) {
        return LookupMode::Unqualified;
    }
    let mut case = FILE_CASE_SENSITIVE_INFO::default();
    #[cfg(debug_assertions)]
    let trace_case = lookup_trace_first(6);
    // SAFETY: live native directory and correctly sized writable metadata structure.
    let case_result = unsafe {
        GetFileInformationByHandleEx(
            directory.as_raw_handle(),
            FileCaseSensitiveInfo,
            (&mut case as *mut FILE_CASE_SENSITIVE_INFO).cast(),
            size_of::<FILE_CASE_SENSITIVE_INFO>() as u32,
        )
    };
    #[cfg(debug_assertions)]
    let case_error = if trace_case && case_result == 0 {
        io::Error::last_os_error().raw_os_error()
    } else {
        None
    };
    #[cfg(debug_assertions)]
    if trace_case {
        eprintln!(
            "[midnight-native-lookup] case_info result={case_result} last_error={case_error:?} flags={:#010x} root={:?}",
            case.Flags, volume.root.physical
        );
    }
    if case_result == 0 || case.Flags & !1 != 0 {
        return LookupMode::Unqualified;
    }
    let request = FILE_FS_PERSISTENT_VOLUME_INFORMATION {
        FlagMask: PERSISTENT_VOLUME_STATE_SHORT_NAME_CREATION_DISABLED,
        Version: 1,
        ..Default::default()
    };
    let mut response = FILE_FS_PERSISTENT_VOLUME_INFORMATION::default();
    let mut returned = 0;
    #[cfg(debug_assertions)]
    let trace_query = lookup_trace_first(7);
    // SAFETY: live duplicate of the pinned filesystem-root directory. Fixed-size
    // input/output buffers remain valid for synchronous DeviceIoControl; no overlapped I/O.
    let query_result = unsafe {
        DeviceIoControl(
            volume.file.as_raw_handle(),
            FSCTL_QUERY_PERSISTENT_VOLUME_STATE,
            (&request as *const FILE_FS_PERSISTENT_VOLUME_INFORMATION).cast(),
            size_of::<FILE_FS_PERSISTENT_VOLUME_INFORMATION>() as u32,
            (&mut response as *mut FILE_FS_PERSISTENT_VOLUME_INFORMATION).cast(),
            size_of::<FILE_FS_PERSISTENT_VOLUME_INFORMATION>() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    #[cfg(debug_assertions)]
    let query_error = if trace_query && query_result == 0 {
        io::Error::last_os_error().raw_os_error()
    } else {
        None
    };
    #[cfg(debug_assertions)]
    if trace_query {
        eprintln!(
            "[midnight-native-lookup] persistent_volume result={query_result} last_error={query_error:?} returned={returned} expected_bytes={} request_version={} request_mask={:#010x} response_version={} response_mask={:#010x} volume_flags={:#010x} root={:?}",
            size_of::<FILE_FS_PERSISTENT_VOLUME_INFORMATION>(),
            request.Version,
            request.FlagMask,
            response.Version,
            response.FlagMask,
            response.VolumeFlags,
            volume.root.physical
        );
    }
    if !persistent_short_names_disabled(query_result, returned, &response) {
        return LookupMode::Unqualified;
    }
    // The root still belongs to the originally pinned volume after the query. A path
    // rebound cannot silently supply a different volume's alias policy.
    let root_identity = metadata_directory(&volume.root.physical)
        .and_then(|file| identity(&file, &volume.root.physical));
    #[cfg(debug_assertions)]
    trace_lookup!(
        10,
        "root_revalidation path={:?} expected={:?} observed={root_identity:?}",
        volume.root.physical,
        volume.root.identity
    );
    if root_identity.ok() != Some(volume.root.identity) {
        return LookupMode::Unqualified;
    }
    LookupMode::ShortNamesDisabled {
        case_sensitive: case.Flags & 1 != 0,
    }
}

pub(super) fn physical_path(file: &File) -> io::Result<PathBuf> {
    final_path(file, VOLUME_NAME_GUID).or_else(|_| final_path(file, VOLUME_NAME_DOS))
}
fn final_path(file: &File, flags: u32) -> io::Result<PathBuf> {
    let mut buffer = vec![0u16; 256];
    loop {
        let capacity = u32::try_from(buffer.len())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        // SAFETY: live native handle and writable UTF-16 buffer; returned length is
        // checked before reading initialized units. No fixed path-length limit.
        let count = unsafe {
            GetFinalPathNameByHandleW(file.as_raw_handle(), buffer.as_mut_ptr(), capacity, flags)
        };
        if count == 0 {
            return Err(io::Error::last_os_error());
        }
        if count >= capacity {
            buffer.resize(count as usize + 1, 0);
            continue;
        }
        buffer.truncate(count as usize);
        return Ok(PathBuf::from(OsString::from_wide(&buffer)));
    }
}

#[cfg(test)]
#[path = "transfer_namespace_parent_tests.rs"]
mod tests;
