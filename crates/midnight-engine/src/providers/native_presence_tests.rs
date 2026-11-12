use super::*;
use midnight_core::{
    DirectoryService, Entry, EntryId, EntryMetadata, FsError, SpaceInfo, StorageService, Volume,
    VolumeService,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Files {
    files: usize,
    folder: bool,
    outside: bool,
    duplicate: bool,
    cancel_at: Option<usize>,
    visited: AtomicUsize,
}
impl DirectoryService for Files {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        for position in 0..self.files + usize::from(self.folder) {
            self.visited.fetch_add(1, Ordering::SeqCst);
            if self.cancel_at == Some(position) {
                cancel.cancel();
            }
            let base = if self.outside {
                path.parent().unwrap()
            } else {
                path
            };
            let name = if self.duplicate {
                "repeated".into()
            } else {
                format!("entry{position}")
            };
            let kind = if position == self.files {
                EntryKind::DirectoryLink
            } else {
                EntryKind::File
            };
            if !emit(Entry::new(EntryId(position as u64), base.join(name), kind)) {
                break;
            }
        }
        Ok(())
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        panic!("presence must not enrich irrelevant file metadata")
    }
}
impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        panic!("presence must not query volumes")
    }
}
impl StorageService for Files {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        panic!("presence must not query volume space")
    }
}

fn filesystem(files: usize) -> Arc<Files> {
    Arc::new(Files {
        files,
        folder: false,
        outside: false,
        duplicate: false,
        cancel_at: None,
        visited: AtomicUsize::new(0),
    })
}
fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/native-presence")
}

#[test]
fn file_only_presence_does_not_spend_the_mount_node_budget() {
    let files = filesystem(100_001);
    assert_eq!(
        NativeProvider::new(files.clone()).has_subfolders(&root(), &Cancellation::default()),
        Ok(false)
    );
    assert_eq!(files.visited.load(Ordering::SeqCst), 100_001);
}

#[test]
fn presence_can_find_a_folder_after_more_files_than_the_mount_node_budget() {
    let mut files = filesystem(100_001);
    Arc::get_mut(&mut files).unwrap().folder = true;
    assert_eq!(
        NativeProvider::new(files.clone()).has_subfolders(&root(), &Cancellation::default()),
        Ok(true)
    );
    assert_eq!(files.visited.load(Ordering::SeqCst), 100_002);
}

#[test]
fn repeated_files_do_not_change_presence_but_foreign_children_are_rejected() {
    let mut files = filesystem(3);
    Arc::get_mut(&mut files).unwrap().duplicate = true;
    assert_eq!(
        NativeProvider::new(files.clone()).has_subfolders(&root(), &Cancellation::default()),
        Ok(false)
    );
    Arc::get_mut(&mut files).unwrap().outside = true;
    assert!(matches!(
        NativeProvider::new(files).has_subfolders(&root(), &Cancellation::default()),
        Err(ProviderError::InvalidResponse(_))
    ));
}

#[test]
fn cancellation_interrupts_presence_and_never_returns_empty() {
    let mut files = filesystem(100);
    Arc::get_mut(&mut files).unwrap().cancel_at = Some(3);
    assert_eq!(
        NativeProvider::new(files.clone()).has_subfolders(&root(), &Cancellation::default()),
        Err(ProviderError::Cancelled)
    );
    assert_eq!(files.visited.load(Ordering::SeqCst), 4);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        NativeProvider::new(files.clone()).has_subfolders(&root(), &cancel),
        Err(ProviderError::Cancelled)
    );
    assert_eq!(files.visited.load(Ordering::SeqCst), 4);
}
