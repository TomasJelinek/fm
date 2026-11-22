//! A viewer of an archive member holds its archive's session and every enclosing one.
use super::*;
use std::sync::Arc;

#[derive(Debug)]
struct Archive(u64, Option<ArchiveParent>);
impl ArchiveMount for Archive {
    fn id(&self) -> u64 {
        self.0
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        panic!("the census lists nothing")
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("the census reads nothing")
    }
    fn validate(&self) -> Result<(), FsError> {
        panic!("the census validates nothing")
    }
    fn parent(&self) -> Option<ArchiveParent> {
        self.1.clone()
    }
}

fn launch(archive: Option<Arc<dyn ArchiveMount>>) -> ViewerLaunch {
    let entries: Arc<[Entry]> = Arc::from([]);
    ViewerLaunch {
        find: None,
        associations: Default::default(),
        tree: None,
        generated: None,
        preferences: Arc::new(Default::default()),
        archive: archive.map(|mount| ArchiveViewer {
            focused: VirtualEntryId {
                mount: mount.id(),
                entry: 2,
            },
            mount,
            source_version: vec![1],
        }),
        view: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
        focused: std::path::PathBuf::new(),
        basic: true,
        disabled_plugins: Arc::from([]),
    }
}

#[test]
fn a_member_viewer_holds_its_archive_and_every_enclosing_one() {
    let outer: Arc<dyn ArchiveMount> = Arc::new(Archive(4, None));
    let parent = ArchiveParent {
        location: ArchiveLocation {
            mount: Some(outer),
            address: ArchiveAddress {
                provider: "midnight.archive".into(),
                format: Some(ArchiveFormat::Zip),
                container: std::env::temp_dir().join("outer.zip"),
                source_version: vec![1],
                components: vec![],
            },
            directory: 1,
            trail: vec![(1, "outer.zip".into())],
        },
        member: VirtualEntryId { mount: 4, entry: 9 },
        name: "inner.zip".into(),
    };
    let inner: Arc<dyn ArchiveMount> = Arc::new(Archive(11, Some(parent)));
    assert_eq!(sessions_of(&launch(Some(inner))), [11, 4]);
    assert!(
        sessions_of(&launch(None)).is_empty(),
        "a native file holds none"
    );
}
