//! Compose deferred backend captures from typed presentation snapshots.
use midnight_core::{StorageKind, StorageService};
use midnight_thumbnail_host::{archive_source::ArchiveThumbnailSource, service};
use midnight_ui::{ThumbnailContent, ThumbnailTarget};
use std::{
    io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

type StorageQuery = Arc<dyn Fn(&Path) -> StorageKind + Send + Sync>;

pub(super) struct Factory {
    pub(super) native: crate::native_content::Capture,
    pub(super) storage: StorageQuery,
}

impl Factory {
    pub(super) fn installed(
        filesystem: Arc<midnight_native_filesystem_host::InstalledProvider>,
    ) -> Self {
        Self {
            native: crate::native_content::factory(filesystem.clone()),
            storage: Arc::new(move |path| filesystem.storage_kind(path).unwrap_or_default()),
        }
    }

    /// Copies bounded metadata only. Every filesystem query is deferred to the capture closure.
    pub(super) fn target(
        &self,
        row: &ThumbnailTarget,
        token: u64,
        edge: u32,
    ) -> Option<service::Target> {
        let (identity, extension, capture) = match &row.content {
            ThumbnailContent::ExecutableIcon(_) => return None,
            ThumbnailContent::Native(path) => (
                path.as_os_str().as_encoded_bytes().to_vec(),
                path.extension()
                    .and_then(|suffix| suffix.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase(),
                crate::native_content::thumbnail(
                    self.native.clone(),
                    path.clone(),
                    row.confirmed_ssd,
                ),
            ),
            ThumbnailContent::Archive {
                mount,
                node,
                address,
                network_allowed,
            } => {
                let source = ArchiveThumbnailSource {
                    mount: mount.clone(),
                    node: node.clone(),
                    source_version: address.source_version.clone(),
                }
                .capture();
                let address = address.clone();
                let mount = mount.clone();
                let storage = self.storage.clone();
                let network_allowed = *network_allowed;
                let capture: service::Capture = Arc::new(move |cancel| {
                    check_cancel(cancel)?;
                    address.validate().map_err(io::Error::other)?;
                    // A restored archive/Hot Path may live on a different device than the
                    // previous native panel. Never inherit that panel's cached SSD hint.
                    if !network_allowed {
                        mount.validate().map_err(io::Error::other)?;
                        let kind = mount
                            .native_container_locator()
                            .map_or(StorageKind::Unknown, |path| storage(path));
                        if matches!(kind, StorageKind::Network | StorageKind::Unknown) {
                            return Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "Archive previews require permission for network or unknown storage",
                            ));
                        }
                    }
                    check_cancel(cancel)?;
                    source(cancel)
                });
                (
                    // The source supplies an independently validated persistent identity,
                    // including provider implementation, container revision and member ID.
                    b"archive-preview-v1".to_vec(),
                    raw_extension(&node.raw_name),
                    capture,
                )
            }
        };
        Some(service::Target {
            token,
            edge,
            identity,
            extension,
            selected: row.selected,
            hovered: row.hovered,
            capture,
        })
    }
}

fn raw_extension(name: &[u8]) -> String {
    name.iter()
        .rposition(|byte| *byte == b'.')
        .and_then(|dot| std::str::from_utf8(&name[dot + 1..]).ok())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn check_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(io::ErrorKind::Interrupted.into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "thumbnail_content_tests.rs"]
mod tests;
