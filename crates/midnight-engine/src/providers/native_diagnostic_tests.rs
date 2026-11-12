use super::*;
use midnight_core::{
    DirectoryService, Entry, EntryMetadata, FsError, SpaceInfo, StorageService, Volume,
    VolumeService,
};
use std::{io, path::PathBuf};

#[cfg(target_os = "linux")]
#[path = "native_diagnostic_linux_tests.rs"]
mod platform;
#[cfg(windows)]
#[path = "native_diagnostic_windows_tests.rs"]
mod platform;

struct Denied;
impl DirectoryService for Denied {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        Err(FsError::from_io(
            "read_directory",
            path,
            io::Error::new(io::ErrorKind::PermissionDenied, "actual provider reason"),
        ))
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        panic!("failed listing must not stat a replacement")
    }
}
impl VolumeService for Denied {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(Vec::new())
    }
}
impl StorageService for Denied {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        panic!("unrelated query")
    }
}

fn capture_failure(path: &Path) -> String {
    let address = native_location(path).unwrap();
    let registry = super::super::ProviderRegistry::default();
    registry
        .register(
            address.provider().clone(),
            Arc::new(NativeProvider::new(Arc::new(Denied))),
        )
        .unwrap();
    let mount = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    mount
        .list(mount.root(), &Cancellation::default())
        .unwrap_err()
        .to_string()
}

fn failure(path: PathBuf) -> String {
    let error = capture_failure(&path);
    assert!(error.contains("actual provider reason"));
    assert!(error.contains("read_directory"));
    assert!(
        error.contains("PermissionDenied"),
        "provider error must retain native error kind"
    );
    assert!(
        error.contains(&format!("{path:?}")),
        "provider error must retain native path units"
    );
    error
}

#[test]
fn native_failure_preserves_exact_path_operation_kind_and_reason() {
    failure(
        std::env::current_dir()
            .unwrap()
            .join("東京 Žluťoučký\nfolder"),
    );
}
