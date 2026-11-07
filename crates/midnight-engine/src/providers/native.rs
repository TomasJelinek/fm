//! Transitional adapter over existing native services, not the final dynamic plugin.
use super::{
    FilesystemProvider, MountedFilesystem, OpenedLocation, ProviderEntry, ProviderError,
    registry::check_cancel,
};
use midnight_core::{Cancellation, EntryKind, FileSystem, NodeToken, ProviderId, StoredLocation};
use midnight_plugin_host::{CapturedFile, ContentSource};
use std::{
    num::NonZeroU32,
    path::Path,
    sync::{Arc, Mutex},
};

#[path = "native_nodes.rs"]
mod nodes;
use nodes::NativeNodes;

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
}

impl NativeProvider {
    pub fn new(filesystem: Arc<dyn FileSystem>) -> Self {
        Self { filesystem }
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
                nodes: Mutex::new(nodes),
            }),
            root,
        })
    }
}

struct NativeMount {
    filesystem: Arc<dyn FileSystem>,
    nodes: Mutex<NativeNodes>,
}

impl MountedFilesystem for NativeMount {
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
            .read_directory(&node.path, cancel, &mut |entry| {
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
                    let token = self
                        .nodes
                        .lock()
                        .map_err(|_| super::registry::poisoned())?
                        .bind(path.to_owned(), entry.kind)?;
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
        result.map_err(|error| ProviderError::Failed(error.to_string()))?;
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
        let source = CapturedFile::capture(&node.path)
            .map_err(|error| ProviderError::Failed(error.to_string()))?;
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
