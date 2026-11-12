//! Internal identity adapter over injected native services; no implicit read fallback.
use super::{
    FilesystemProvider, MountedFilesystem, OpenedLocation, ProviderEntry, ProviderError,
    registry::check_cancel,
};
use midnight_core::{
    Cancellation, EntryKind, FileSystem, NodeToken, ProviderId, SizeDirectoryIdentity,
    StoredLocation,
};
use midnight_plugin_host::{CapturedFile, ContentSource};
use std::{
    num::NonZeroU32,
    path::Path,
    sync::{Arc, Mutex},
};

#[path = "native_nodes.rs"]
mod nodes;
use nodes::NativeNodes;

#[path = "native_presence.rs"]
mod presence;

#[cfg(test)]
#[path = "native_diagnostic_tests.rs"]
mod diagnostic_tests;

#[cfg(windows)]
#[path = "native_codec/windows.rs"]
mod codec;
#[cfg(target_os = "linux")]
#[path = "native_codec/linux.rs"]
mod codec;
#[cfg(not(any(windows, target_os = "linux")))]
#[path = "native_codec/unsupported.rs"]
mod codec;

pub struct NativeProvider {
    filesystem: Arc<dyn FileSystem>,
    capture: Option<NativeCapture>,
    locale: Option<Arc<dyn midnight_core::LocaleService>>,
}

pub type NativeCapture =
    Arc<dyn Fn(&Path, &Cancellation) -> Result<CapturedFile, ProviderError> + Send + Sync>;

impl NativeProvider {
    pub(crate) fn filesystem(&self) -> Arc<dyn FileSystem> {
        self.filesystem.clone()
    }
    /// Admit one already completed backend listing without native enumeration or stat.
    /// The first read consumes the snapshot; later root reads use this native mount.
    pub(crate) fn capture_root(
        &self,
        path: &Path,
        entries: &[midnight_core::Entry],
        cancel: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError> {
        super::root_seed::preflight(entries, cancel)?;
        native_location(path)?;
        let mut nodes = NativeNodes::default();
        let root = nodes.bind(path.to_owned(), EntryKind::Directory)?;
        let mut ids = std::collections::HashSet::new();
        let mut paths = std::collections::HashSet::new();
        let mut rows = Vec::with_capacity(entries.len());
        for entry in entries {
            check_cancel(cancel)?;
            let actual = entry.native_path().ok_or(ProviderError::InvalidResponse(
                "virtual entry in native root snapshot",
            ))?;
            if !actual.is_absolute()
                || actual.parent() != Some(path)
                || actual.as_os_str().as_encoded_bytes().contains(&0)
                || actual.file_name().is_none_or(|name| name != entry.name)
                || !ids.insert(entry.id)
                || !paths.insert(actual)
            {
                return Err(ProviderError::InvalidResponse(
                    "invalid native root snapshot",
                ));
            }
            rows.push(ProviderEntry {
                token: nodes.bind(actual.to_owned(), entry.kind)?,
                raw_name: Arc::from(codec::encode(&entry.name)?),
                display_name: entry.display_name.clone(),
                kind: entry.kind,
                metadata: entry.metadata.clone(),
            });
        }
        let opened = OpenedLocation {
            root,
            mount: Arc::new(NativeMount {
                filesystem: self.filesystem.clone(),
                capture: self.capture.clone(),
                locale: self.locale.clone(),
                nodes: Mutex::new(nodes),
            }),
        };
        super::root_seed::wrap(
            opened,
            rows,
            super::root_seed::RootReadPolicy::NativeRefresh,
            cancel,
        )
    }

    pub fn new(filesystem: Arc<dyn FileSystem>) -> Self {
        Self {
            filesystem,
            capture: None,
            locale: None,
        }
    }

    /// Production injects the installed filesystem plugin's capture service.
    /// A browse-only adapter returns Unsupported for content, never native I/O.
    pub fn with_capture(mut self, capture: NativeCapture) -> Self {
        self.capture = Some(capture);
        self
    }
    pub fn with_locale(mut self, locale: Arc<dyn midnight_core::LocaleService>) -> Self {
        self.locale = Some(locale);
        self
    }
}

/// Encode the exact native address; do not interpret a displayed archive path here.
pub fn native_location(path: &Path) -> Result<StoredLocation, ProviderError> {
    if path.as_os_str().as_encoded_bytes().len() > nodes::MAX_PATH_STORAGE {
        return Err(ProviderError::ResourceLimited);
    }
    if !path.is_absolute() || path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(ProviderError::InvalidResponse(
            "native location must be an absolute NUL-free path",
        ));
    }
    let bytes = codec::encode(path.as_os_str())?;
    let provider = ProviderId::new(codec::PROVIDER)
        .map_err(|error| ProviderError::Failed(error.to_string()))?;
    Ok(StoredLocation::new(
        provider,
        NonZeroU32::MIN,
        Arc::from(bytes),
    ))
}

impl FilesystemProvider for NativeProvider {
    fn resolve(
        &self,
        location: &StoredLocation,
        cancel: &Cancellation,
    ) -> Result<OpenedLocation, ProviderError> {
        check_cancel(cancel)?;
        if location.provider().as_str() != codec::PROVIDER
            || location.codec_version() != NonZeroU32::MIN
        {
            return Err(ProviderError::Unsupported);
        }
        if location.address().len() > codec::MAX_ENCODED_ADDRESS_BYTES {
            return Err(ProviderError::ResourceLimited);
        }
        let path = codec::decode(location.address())?;
        if !path.is_absolute() {
            return Err(ProviderError::InvalidResponse("relative native address"));
        }
        let mut nodes = NativeNodes::default();
        let root = nodes.bind(path, EntryKind::Directory)?;
        Ok(OpenedLocation {
            mount: Arc::new(NativeMount {
                filesystem: self.filesystem.clone(),
                capture: self.capture.clone(),
                locale: self.locale.clone(),
                nodes: Mutex::new(nodes),
            }),
            root,
        })
    }
}

struct NativeMount {
    filesystem: Arc<dyn FileSystem>,
    capture: Option<NativeCapture>,
    locale: Option<Arc<dyn midnight_core::LocaleService>>,
    nodes: Mutex<NativeNodes>,
}

impl MountedFilesystem for NativeMount {
    fn browse_directory_identity(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<Option<SizeDirectoryIdentity>, ProviderError> {
        check_cancel(cancel)?;
        let node = self
            .nodes
            .lock()
            .map_err(|_| super::registry::poisoned())?
            .get(node)?;
        if !matches!(node.kind, EntryKind::Directory | EntryKind::DirectoryLink) {
            return Err(ProviderError::Unsupported);
        }
        let identity = self
            .filesystem
            .browse_directory_identity(&node.path, cancel)
            .map_err(|error| match error.kind {
                std::io::ErrorKind::Unsupported => ProviderError::Unsupported,
                std::io::ErrorKind::Interrupted if cancel.is_cancelled() => {
                    ProviderError::Cancelled
                }
                _ => ProviderError::Failed(format!(
                    "{error}\nNative path: {:?}\nError kind: {:?}",
                    error.path, error.kind,
                )),
            })?;
        check_cancel(cancel)?;
        Ok(identity)
    }

    fn resolve_target(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<super::ProviderTarget, ProviderError> {
        check_cancel(cancel)?;
        let node = self
            .nodes
            .lock()
            .map_err(|_| super::registry::poisoned())?
            .get(node)?;
        check_cancel(cancel)?;
        Ok(super::ProviderTarget::Native(node.path.as_ref().clone()))
    }

    fn list(
        &self,
        directory: &NodeToken,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        check_cancel(cancel)?;
        let node = self
            .nodes
            .lock()
            .map_err(|_| super::registry::poisoned())?
            .get(directory)?;
        if !matches!(node.kind, EntryKind::Directory | EntryKind::DirectoryLink) {
            return Err(ProviderError::Unsupported);
        }
        let mut failure = None;
        let mut stopped = false;
        let result = self
            .filesystem
            .read_directory(&node.path, cancel, &mut |mut entry| {
                if stopped {
                    return false;
                }
                let result = (|| {
                    check_cancel(cancel)?;
                    let path = entry.native_path().ok_or(ProviderError::InvalidResponse(
                        "virtual entry in native listing",
                    ))?;
                    if !path.is_absolute()
                        || path.parent() != Some(node.path.as_path())
                        || path.file_name().is_none_or(|name| name != entry.name)
                    {
                        return Err(ProviderError::InvalidResponse(
                            "native entry outside its directory",
                        ));
                    }
                    let path = path.to_owned();
                    if !self.filesystem.listing_metadata_complete(&entry)
                        && let Ok(metadata) = self.filesystem.entry_metadata(&path)
                    {
                        entry.metadata = metadata;
                    }
                    check_cancel(cancel)?;
                    crate::localize_metadata(&mut entry.metadata, self.locale.as_deref());
                    let token = self
                        .nodes
                        .lock()
                        .map_err(|_| super::registry::poisoned())?
                        .bind(path, entry.kind)?;
                    Ok(ProviderEntry {
                        token,
                        raw_name: Arc::from(codec::encode(&entry.name)?),
                        display_name: entry.display_name,
                        kind: entry.kind,
                        metadata: entry.metadata,
                    })
                })();
                match result {
                    Ok(entry) => {
                        stopped = !emit(entry);
                        !stopped
                    }
                    Err(error) => {
                        failure = Some(error);
                        stopped = true;
                        false
                    }
                }
            });
        if let Some(error) = failure {
            return Err(error);
        }
        result.map_err(|error| {
            // Display alone replaces invalid native path units and omits kind.
            // Retain both in the backend description before tree summaries shorten it.
            ProviderError::Failed(format!(
                "{error}\nNative path: {:?}\nError kind: {:?}",
                error.path, error.kind,
            ))
        })?;
        check_cancel(cancel)
    }

    fn parent(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<Option<NodeToken>, ProviderError> {
        check_cancel(cancel)?;
        let mut nodes = self.nodes.lock().map_err(|_| super::registry::poisoned())?;
        let node = nodes.get(node)?;
        node.path
            .parent()
            .map(|parent| nodes.bind(parent.to_owned(), EntryKind::Directory))
            .transpose()
    }

    fn open_content(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn ContentSource>, ProviderError> {
        check_cancel(cancel)?;
        let node = self
            .nodes
            .lock()
            .map_err(|_| super::registry::poisoned())?
            .get(node)?;
        if node.kind != EntryKind::File {
            return Err(ProviderError::Unsupported);
        }
        let capture = self.capture.as_ref().ok_or(ProviderError::Unsupported)?;
        let source = capture(&node.path, cancel)?;
        check_cancel(cancel)?;
        Ok(Arc::new(NativeContent(source)))
    }
}

#[derive(Debug)]
struct NativeContent(CapturedFile);
impl ContentSource for NativeContent {
    fn length(&self) -> u64 {
        self.0.length()
    }
    fn display_name(&self) -> String {
        self.0.display_name()
    }
    fn version(&self) -> Vec<u8> {
        self.0.stable_version()
    }
    fn validate(&self) -> Result<(), midnight_plugin_host::Error> {
        self.0.revalidate()
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, midnight_plugin_host::Error> {
        self.0.read_at(offset, length)
    }
}

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "native_seed_tests.rs"]
mod seed_tests;
