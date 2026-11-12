//! Native file owner shared by mailbox lifetime regressions.
use crate::providers::{MountedFilesystem, ProviderEntry, ProviderError};
use midnight_core::*;
use std::{
    fs::File,
    path::Path,
    sync::{Arc, mpsc},
    thread::{self, ThreadId},
};

#[derive(Debug)]
pub(crate) struct Owner {
    file: Option<File>,
    dropped: mpsc::Sender<ThreadId>,
    gate: std::sync::Mutex<Option<mpsc::Receiver<()>>>,
}
impl Owner {
    pub(crate) fn hold_drop(&mut self, gate: mpsc::Receiver<()>) {
        *self.gate.get_mut().unwrap() = Some(gate);
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = self.dropped.send(thread::current().id());
        if let Some(gate) = self.gate.get_mut().unwrap().take() {
            gate.recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
    }
}
pub(crate) fn owner() -> (tempfile::TempDir, Arc<Owner>, mpsc::Receiver<ThreadId>) {
    let directory = tempfile::tempdir().unwrap();
    let (dropped, report) = mpsc::channel();
    let file = File::create(directory.path().join("native-owner.bin")).unwrap();
    (
        directory,
        Arc::new(Owner {
            file: Some(file),
            dropped,
            gate: std::sync::Mutex::new(None),
        }),
        report,
    )
}
pub(crate) fn token(id: u8) -> NodeToken {
    NodeToken::new(&[id]).unwrap()
}
impl MountedFilesystem for Owner {
    fn list(
        &self,
        directory: &NodeToken,
        _: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        if directory == &token(0) || directory == &token(1) {
            emit(ProviderEntry {
                token: token(if directory == &token(0) { 1 } else { 2 }),
                raw_name: Arc::from(b"child".as_slice()),
                display_name: "child".into(),
                kind: EntryKind::Directory,
                metadata: EntryMetadata::default(),
            });
        }
        Ok(())
    }
    fn parent(&self, _: &NodeToken, _: &Cancellation) -> Result<Option<NodeToken>, ProviderError> {
        Ok(None)
    }
}
impl ArchiveMount for Owner {
    fn id(&self) -> u64 {
        71
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        Ok(vec![])
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("metadata-only mailbox test must not extract content")
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
}
pub(crate) fn archive_listing(mount: Arc<Owner>, path: &Path) -> ArchiveListing {
    ArchiveListing {
        location: ArchiveLocation {
            mount: Some(mount),
            address: ArchiveAddress {
                provider: "test.mailbox".into(),
                format: Some(ArchiveFormat::Zip),
                container: path.join("fixture.zip"),
                source_version: vec![1],
                components: vec![],
            },
            directory: 1,
            trail: vec![(1, "fixture.zip".into())],
        },
        entries: Arc::from([]),
        index: Arc::new(DirectoryIndex::from_entries(&[])),
    }
}
