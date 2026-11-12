//! Lazy composition of the installed filesystem; no hidden native fallback.
use crate::services::fs_error;
use crate::{Cancel, CaptureCache, NativeProvider, client::check_cancel, discovery::invalid};
use midnight_core::{
    Cancellation, DirectoryService, Entry, EntryMetadata, FsError, SpaceInfo, StorageKind,
    StorageService, Volume, VolumeInformation, VolumeService,
};
use midnight_filesystem_plugin_api::Failure;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

/// Construction performs no IO. All service calls and final drops belong on backend workers.
/// Both successful discovery and failures remain fixed for this application session.
#[derive(Default)]
pub struct InstalledProvider {
    directory: Option<PathBuf>,
    home: Option<PathBuf>,
    provider: OnceLock<Result<NativeProvider, Failure>>,
    captures: OnceLock<Arc<CaptureCache>>,
    size_directories: OnceLock<Arc<dyn DirectoryService>>,
}

impl std::fmt::Debug for InstalledProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstalledProvider")
            .field("directory", &self.directory)
            .finish_non_exhaustive()
    }
}

impl InstalledProvider {
    /// Install the explicit host-native directory-size identity resolver once.
    /// This never substitutes for the installed provider's listing/stat service.
    pub fn set_size_directory_service(&self, service: Arc<dyn DirectoryService>) -> bool {
        self.size_directories.set(service).is_ok()
    }
    /// The application's singleton is pinned until exit to avoid native cleanup on the UI thread.
    pub fn shared(home: Option<PathBuf>) -> Arc<Self> {
        static SERVICE: OnceLock<Arc<InstalledProvider>> = OnceLock::new();
        SERVICE
            .get_or_init(|| {
                Arc::new(Self {
                    home,
                    ..Self::default()
                })
            })
            .clone()
    }

    /// Explicit installation for embedding/conformance. It receives the same trust checks.
    /// The caller must destroy this owner and captured sources on backend workers.
    pub fn from_installation(directory: PathBuf, home: Option<PathBuf>) -> Self {
        Self {
            directory: Some(directory),
            home,
            ..Self::default()
        }
    }

    pub fn provider(&self) -> Result<&NativeProvider, Failure> {
        self.provider
            .get_or_init(|| {
                let directory = if let Some(directory) = &self.directory {
                    directory.clone()
                } else {
                    midnight_plugin_host::installed_executable()
                        .map_err(invalid)?
                        .parent()
                        .ok_or_else(|| invalid("Installation directory unavailable"))?
                        .to_owned()
                };
                let catalog = crate::Catalog::discover(&directory)?;
                let package = catalog.native_package().map_err(|error| {
                    if catalog.diagnostics.is_empty() {
                        error
                    } else {
                        invalid(format!("{error}: {}", catalog.diagnostics.join("; ")))
                    }
                })?;
                NativeProvider::open(package)
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    pub fn capture(
        &self,
        path: &Path,
        cancelled: Cancel<'_>,
    ) -> Result<midnight_plugin_host::CapturedFile, Failure> {
        check_cancel(cancelled)?;
        let provider = self.provider()?;
        self.captures
            .get_or_init(|| CaptureCache::new(provider.clone()))
            .capture(path, cancelled)
    }

    /// Capture through the installed provider and require a persisted native revision, if supplied.
    /// Supports only exact current-platform v1 revisions and their legacy equivalent.
    /// Mismatches fail with `SourceChanged`; provider failures never use a native fallback.
    pub fn capture_matching(
        &self,
        path: &Path,
        expected: Option<&[u8]>,
        cancelled: Cancel<'_>,
    ) -> Result<midnight_plugin_host::CapturedFile, Failure> {
        let file = self.capture(path, cancelled)?;
        check_cancel(cancelled)?;
        if expected
            .is_some_and(|expected| !crate::revision::matches(expected, &file.stable_version()))
        {
            return Err(Failure::new(
                midnight_filesystem_plugin_api::ErrorCode::SourceChanged,
                "Native file no longer matches the saved revision",
            ));
        }
        check_cancel(cancelled)?;
        Ok(file)
    }

    fn service(&self, operation: &'static str, path: &Path) -> Result<&NativeProvider, FsError> {
        self.provider()
            .map_err(|failure| fs_error(operation, path, failure))
    }
}

#[cfg(test)]
#[path = "installed_capture_tests.rs"]
mod capture_tests;

#[cfg(test)]
#[path = "installed_size_tests.rs"]
mod size_tests;

impl DirectoryService for InstalledProvider {
    fn resolve_size_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Option<midnight_core::SizeDirectoryTarget>, FsError> {
        if cancel.is_cancelled() {
            return Err(FsError::from_io(
                "resolve directory size target",
                path,
                std::io::ErrorKind::Interrupted.into(),
            ));
        }
        self.service("resolve directory size target", path)?;
        let Some(service) = self.size_directories.get() else {
            return Err(FsError::from_io(
                "resolve directory size target",
                path,
                std::io::ErrorKind::Unsupported.into(),
            ));
        };
        service.resolve_size_directory(path, cancel)
    }
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        if cancel.is_cancelled() {
            return Err(fs_error(
                "list directory",
                path,
                Failure::new(
                    midnight_filesystem_plugin_api::ErrorCode::Cancelled,
                    "Listing canceled",
                ),
            ));
        }
        DirectoryService::read_directory(self.service("list directory", path)?, path, cancel, emit)
    }
    fn listing_metadata_complete(&self, entry: &Entry) -> bool {
        entry.metadata.modified.is_some()
            && (entry.metadata.native_attributes.is_some() || entry.metadata.unix_mode.is_some())
    }
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        self.service("read metadata", path)?.entry_metadata(path)
    }
}
impl VolumeService for InstalledProvider {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        let mut volumes = self
            .service("list volumes", Path::new(""))?
            .list_volumes()?;
        if let Some(home) = &self.home
            && !volumes.iter().any(|volume| volume.id == "home")
        {
            volumes.push(Volume {
                id: "home".into(),
                label: "Home".into(),
                root: home.clone(),
            });
        }
        Ok(volumes)
    }
    fn volume_label(&self, path: &Path) -> Result<Option<OsString>, FsError> {
        self.service("query volume label", path)?.volume_label(path)
    }
}
impl StorageService for InstalledProvider {
    fn storage_kind(&self, path: &Path) -> Result<StorageKind, FsError> {
        self.service("query storage kind", path)?.storage_kind(path)
    }
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
        self.service("query filesystem space", path)?
            .space_info(path)
    }
    fn volume_information(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<VolumeInformation, FsError> {
        check_cancel(&|| cancel.is_cancelled())
            .map_err(|failure| fs_error("query volume information", path, failure))?;
        self.service("query volume information", path)?
            .volume_information(path, cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_plugin_never_falls_back_even_for_existing_native_files() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("existing.txt");
        std::fs::write(&path, b"must not be read by a fallback").unwrap();
        let installed = InstalledProvider::from_installation(
            fixture.path().join("absent-installation"),
            Some(fixture.path().to_owned()),
        );
        assert!(
            installed.provider.get().is_none(),
            "Construction performs no discovery"
        );
        assert!(
            installed.list_volumes().is_err(),
            "Home cannot fabricate native availability"
        );
        assert!(installed.entry_metadata(&path).is_err());
        assert!(installed.capture(&path, &|| false).is_err());
        assert!(installed.space_info(fixture.path()).is_err());
        assert!(
            installed
                .read_directory(fixture.path(), &Cancellation::default(), &mut |_| panic!(
                    "Builtin listing must not run"
                ))
                .is_err()
        );
    }

    #[test]
    fn cancelled_queries_do_not_start_discovery() {
        let installed = InstalledProvider::default();
        let cancel = Cancellation::default();
        cancel.cancel();
        let path = Path::new("/");
        assert!(
            installed
                .read_directory(path, &cancel, &mut |_| true)
                .is_err()
        );
        assert!(installed.volume_information(path, &cancel).is_err());
        assert!(installed.capture(path, &|| true).is_err());
        assert!(installed.provider.get().is_none());
    }
}
