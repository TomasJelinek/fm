use super::*;
use midnight_core::{ArchiveAddress, ArchiveMount, ArchiveNode, Cancellation, EntryKind, FsError};
use std::sync::atomic::AtomicUsize;

#[derive(Debug, Default)]
struct Mount {
    calls: AtomicUsize,
}
impl ArchiveMount for Mount {
    fn id(&self) -> u64 {
        42
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        panic!("thumbnail capture must not enumerate the archive")
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        Ok(vec![7])
    }
    fn validate(&self) -> Result<(), FsError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn native_container_locator(&self) -> Option<&Path> {
        Some(Path::new("pictures.zip"))
    }
    fn persistent_member_identity(
        &self,
        entry: u64,
        expected_source_version: &[u8],
    ) -> Result<Option<Vec<u8>>, FsError> {
        assert_eq!(entry, 2);
        assert_eq!(expected_source_version, b"revision");
        self.validate()?;
        Ok(Some(b"persistent-container-provider-member".to_vec()))
    }
}

fn target(mount: Arc<Mount>, network_allowed: bool) -> ThumbnailTarget {
    ThumbnailTarget {
        identity: "0:1:entry:2".into(),
        content: ThumbnailContent::Archive {
            mount,
            node: ArchiveNode {
                id: midnight_core::VirtualEntryId {
                    mount: 42,
                    entry: 2,
                },
                parent: 1,
                raw_name: b"picture.PNG".as_slice().into(),
                display_name: "misleading.exe".into(),
                kind: EntryKind::File,
                size: Some(1),
                readable: true,
                modified: None,
            },
            address: Arc::new(ArchiveAddress {
                provider: "test.archive".into(),
                format: Some(midnight_core::ArchiveFormat::Zip),
                // Admission must query the mount's locator, never this presentation snapshot.
                container: std::env::current_dir().unwrap().join("different-drive.zip"),
                source_version: b"revision".to_vec(),
                components: Vec::new(),
            }),
            network_allowed,
        },
        selected: false,
        hovered: false,
        confirmed_ssd: true, // Deliberately wrong native hint: archive adapter must ignore it.
        edge: 96,
    }
}

#[test]
fn archive_capture_is_deferred_and_never_uses_a_display_or_member_native_path() {
    let mount = Arc::new(Mount::default());
    let queries = Arc::new(AtomicUsize::new(0));
    let query_count = queries.clone();
    let factory = Factory {
        native: Arc::new(|_, _| panic!("archive member reached native capture")),
        storage: Arc::new(move |path| {
            assert_eq!(path.file_name().unwrap(), "pictures.zip");
            query_count.fetch_add(1, Ordering::Relaxed);
            StorageKind::LocalSsd
        }),
    };
    let prepared = factory
        .target(&target(mount.clone(), false), 10, 96)
        .unwrap();
    assert_eq!(queries.load(Ordering::Relaxed), 0);
    assert_eq!(mount.calls.load(Ordering::Relaxed), 0);
    assert_eq!(prepared.extension, "png", "route by raw provider name");
    let source = (prepared.capture)(&AtomicBool::new(false)).unwrap();
    assert_eq!(queries.load(Ordering::Relaxed), 1);
    assert!(!source.confirmed_local_ssd());
    assert_eq!(source.read_at(0, 1).unwrap(), [7]);
}

#[test]
fn archive_network_and_unknown_storage_require_explicit_opt_in() {
    for kind in [StorageKind::Network, StorageKind::Unknown] {
        let mount = Arc::new(Mount::default());
        let factory = Factory {
            native: Arc::new(|_, _| panic!("archive native fallback")),
            storage: Arc::new(move |_| kind),
        };
        let prepared = factory
            .target(&target(mount.clone(), false), 10, 96)
            .unwrap();
        let result = (prepared.capture)(&AtomicBool::new(false));
        assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::PermissionDenied));
        assert_eq!(
            mount.calls.load(Ordering::Relaxed),
            1,
            "only container validation"
        );
        let prepared = factory
            .target(&target(mount.clone(), true), 11, 96)
            .unwrap();
        assert!((prepared.capture)(&AtomicBool::new(false)).is_ok());
    }
}

#[test]
fn canceled_archive_capture_does_not_query_or_read_storage() {
    let factory = Factory {
        native: Arc::new(|_, _| panic!("archive native fallback")),
        storage: Arc::new(|_| panic!("canceled query")),
    };
    let mount = Arc::new(Mount::default());
    let prepared = factory
        .target(&target(mount.clone(), false), 10, 96)
        .unwrap();
    let result = (prepared.capture)(&AtomicBool::new(true));
    assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::Interrupted));
    assert_eq!(mount.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn changing_archive_network_permission_discards_the_previous_pinned_source_token() {
    let mount = Arc::new(Mount::default());
    let old = target(mount.clone(), true);
    let current = target(mount, false);
    let mut tokens = std::collections::HashMap::from([(old.identity.clone(), 55)]);
    super::super::retain_source_tokens(&mut tokens, &[old], &[current]);
    assert!(tokens.is_empty());
}

#[test]
fn cancellation_during_storage_query_prevents_member_capture() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let mount = Arc::new(Mount::default());
    let factory = Factory {
        native: Arc::new(|_, _| panic!("archive native fallback")),
        storage: Arc::new(move |_| {
            flag.store(true, Ordering::Release);
            StorageKind::LocalSsd
        }),
    };
    let prepared = factory
        .target(&target(mount.clone(), false), 10, 96)
        .unwrap();
    let result = (prepared.capture)(&cancelled);
    assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::Interrupted));
    assert_eq!(
        mount.calls.load(Ordering::Relaxed),
        1,
        "only initial mount validation"
    );
}

#[test]
fn executable_artwork_never_creates_an_image_decoder_capture() {
    let factory = Factory {
        native: Arc::new(|_, _| panic!("executable artwork reached image capture")),
        storage: Arc::new(|_| panic!("executable artwork reached thumbnail storage lookup")),
    };
    let executable = ThumbnailTarget {
        identity: "command-icon:1:editor".into(),
        content: ThumbnailContent::ExecutableIcon(
            std::env::current_dir().unwrap().join("editor.exe"),
        ),
        selected: false,
        hovered: false,
        confirmed_ssd: false,
        edge: 64,
    };
    assert!(factory.target(&executable, 1, 64).is_none());
}
