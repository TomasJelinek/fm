use super::*;
use midnight_core::{
    ArchiveAddress, ArchiveLocation, ArchiveMount, ArchiveNode, Cancellation, VirtualEntryId,
};
use midnight_plugin_host::CapturedFile;
use std::sync::atomic::{AtomicBool, AtomicUsize};

/// The enclosing archive's mount: it counts validations and the references given up.
#[derive(Debug, Default)]
struct Enclosing {
    changed: AtomicBool,
    /// The native archive file was changed (`InvalidData`, `Mount::source_failure`).
    gone: AtomicBool,
    tainted: AtomicBool,
    validations: AtomicUsize,
    releases: AtomicUsize,
}
impl ArchiveMount for Enclosing {
    fn id(&self) -> u64 {
        3
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        unreachable!("the nested source never lists")
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        unreachable!("the nested source reads its staged content")
    }
    fn validate(&self) -> Result<(), FsError> {
        self.validations.fetch_add(1, Ordering::SeqCst);
        if self.gone.load(Ordering::Acquire) {
            return Err(FsError::from_io(
                "archive source changed",
                Path::new("outer.zip"),
                io::Error::new(io::ErrorKind::InvalidData, "changed for good"),
            ));
        }
        if self.changed.load(Ordering::Acquire) {
            return Err(FsError::from_io(
                "archive source changed",
                Path::new("outer.zip"),
                io::Error::other("changed"),
            ));
        }
        Ok(())
    }
    fn cleanup_unconfirmed(&self) -> bool {
        self.tainted.load(Ordering::Acquire)
    }
    /// The test keeps its own reference, so each release gives up a shared one.
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        self.releases.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Debug)]
struct Bytes(&'static [u8]);
impl ArchiveContent for Bytes {
    fn length(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, FsError> {
        let start = (offset as usize).min(self.0.len());
        let end = start.saturating_add(length as usize).min(self.0.len());
        Ok(self.0[start..end].to_vec())
    }
}

fn parent(mount: &Arc<Enclosing>) -> ArchiveParent {
    ArchiveParent {
        location: ArchiveLocation {
            mount: Some(mount.clone()),
            address: ArchiveAddress {
                provider: "midnight.fs.archive".into(),
                format: Some(midnight_core::ArchiveFormat::Zip),
                container: std::env::temp_dir().join("outer.zip"),
                source_version: vec![1, 2],
                components: vec![b"folder".to_vec()],
            },
            directory: 4,
            trail: vec![(1, "outer.zip".into()), (4, "folder".into())],
        },
        member: VirtualEntryId { mount: 3, entry: 7 },
        name: "inner.zip".into(),
    }
}

fn source(mount: &Arc<Enclosing>) -> (Arc<Link>, Arc<NestedSource>) {
    let link = Arc::new(Link::new(parent(mount), true));
    let source = Arc::new(NestedSource::new(
        link.clone(),
        Arc::new(Bytes(b"inner archive")),
        revision(&[1, 2], 7, true).0,
        "inner.zip".into(),
    ));
    (link, source)
}

#[test]
fn the_staged_member_is_read_through_the_enclosing_archive_revision() {
    let mount = Arc::new(Enclosing::default());
    let (link, source) = source(&mount);
    let separator = std::path::MAIN_SEPARATOR;
    assert!(
        link.label
            .to_string_lossy()
            .ends_with(&format!("outer.zip{separator}folder{separator}inner.zip")),
        "{}",
        link.label.display()
    );
    let captured = CapturedFile::from_source(source.clone()).unwrap();
    assert!(
        captured.path().as_os_str().is_empty(),
        "no path is invented"
    );
    assert_eq!(captured.length(), 13);
    assert_eq!(captured.display_name(), "inner.zip");
    assert_eq!(captured.read_at(6, 64).unwrap(), b"archive");
    assert!(mount.validations.load(Ordering::SeqCst) >= 2);
    assert!(!captured.cleanup_unconfirmed());
    mount.tainted.store(true, Ordering::Release);
    assert!(
        captured.cleanup_unconfirmed(),
        "the enclosing taint carries over"
    );
    mount.tainted.store(false, Ordering::Release);
    mount.changed.store(true, Ordering::Release);
    assert!(captured.read_at(0, 5).is_err());
    assert!(captured.revalidate().is_err());
    assert_eq!(
        link.parent().unwrap().location.mount_id(),
        Some(3),
        "the mount reports its enclosing folder"
    );
}

/// An archive inside an archive whose native archive file was changed reports that too,
/// so its own mount fails as `InvalidData`; another failure of the chain does not.
#[test]
fn a_lasting_change_of_the_enclosing_chain_carries_over() {
    let mount = Arc::new(Enclosing::default());
    let (_link, source) = source(&mount);
    let captured = CapturedFile::from_source(source.clone()).unwrap();
    mount.changed.store(true, Ordering::Release);
    assert!(captured.revalidate().is_err());
    assert!(!captured.changed_since_capture());
    mount.changed.store(false, Ordering::Release);
    captured.revalidate().unwrap();
    mount.gone.store(true, Ordering::Release);
    assert!(captured.revalidate().is_err());
    assert!(captured.changed_since_capture());
}

#[test]
fn only_the_last_source_owner_gives_the_enclosing_archive_up() {
    let mount = Arc::new(Enclosing::default());
    let (link, source) = source(&mount);
    let shared = source.clone();
    shared.release_on_worker().unwrap();
    assert_eq!(mount.releases.load(Ordering::SeqCst), 0);
    assert!(link.parent().is_some());
    source.release_on_worker().unwrap();
    assert_eq!(mount.releases.load(Ordering::SeqCst), 1);
    assert!(link.parent().is_none(), "the enclosing folder was given up");
    // A source whose enclosing archive is gone fails closed.
    let (link, source) = self::source(&mount);
    link.release().unwrap();
    assert_eq!(mount.releases.load(Ordering::SeqCst), 2);
    assert!(source.validate().is_err());
    source.release_on_worker().unwrap();
    assert_eq!(mount.releases.load(Ordering::SeqCst), 2);
}

#[test]
fn revisions_name_stable_chains_and_stay_unique_otherwise() {
    let (first, persistent) = revision(b"native", 7, true);
    assert!(persistent);
    assert_eq!(
        revision(b"native", 7, true).0,
        first,
        "reopening names it again"
    );
    assert_ne!(revision(b"native", 8, true).0, first, "another member");
    assert_ne!(revision(b"changed", 7, true).0, first, "another revision");
    let (local, persistent) = revision(b"native", 7, false);
    assert!(!persistent);
    assert_ne!(
        revision(b"native", 7, false).0,
        local,
        "unique to its mount"
    );
    let (long, persistent) = revision(&[1; 250], 7, true);
    assert!(!persistent && long.len() <= MAX_REVISION);
    assert!(first.len() <= MAX_REVISION && !first.is_empty());
}
