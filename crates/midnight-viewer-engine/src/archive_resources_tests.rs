use super::*;
use midnight_core::{ArchiveNode, EntryKind, FsError, VirtualEntryId};

#[derive(Debug, Default)]
struct Assets {
    changed: AtomicBool,
}

fn node(entry: u64, parent: u64, name: &str, kind: EntryKind) -> ArchiveNode {
    ArchiveNode {
        id: VirtualEntryId { mount: 9, entry },
        parent,
        raw_name: name.as_bytes().into(),
        display_name: "display names are not identities".into(),
        kind,
        size: Some(6),
        readable: true,
        modified: None,
    }
}

impl ArchiveMount for Assets {
    fn id(&self) -> u64 {
        9
    }
    fn validate(&self) -> Result<(), FsError> {
        if self.changed.load(Ordering::Acquire) {
            Err(FsError::from_io(
                "archive resource",
                std::path::Path::new(""),
                std::io::Error::other("changed"),
            ))
        } else {
            Ok(())
        }
    }
    fn list(&self, directory: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.validate()?;
        Ok(if directory == 7 {
            vec![
                node(8, 7, "styles", EntryKind::Directory),
                node(10, 7, "dup.css", EntryKind::File),
                node(11, 7, "dup.css", EntryKind::File),
                node(12, 7, "link.css", EntryKind::Symlink),
            ]
        } else if directory == 8 {
            vec![node(13, 8, "文档.css", EntryKind::File)]
        } else {
            vec![]
        })
    }
    fn read(&self, _: u64, offset: u64, length: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        self.validate()?;
        let start = (offset as usize).min(6);
        let end = (start + length as usize).min(6);
        Ok(b"body{}"[start..end].to_vec())
    }
}

fn asset_main(mount: Arc<Assets>, closed: Arc<AtomicBool>) -> CapturedFile {
    capture_member(Arc::new(Member {
        mount,
        entry: 2,
        parent: 7,
        length: 6,
        name: "main.html".into(),
        version: vec![2, 3],
        closed,
    }))
    .unwrap()
}

#[test]
fn archive_assets_resolve_raw_names_in_captured_parent_and_retain_mount_revision() {
    let mount = Arc::new(Assets::default());
    let main = asset_main(mount.clone(), Arc::new(AtomicBool::new(false)));
    let resources = main
        .resources()
        .expect("archive source must provide its captured subtree");
    let asset = resources
        .capture(&["styles".into(), "文档.css".into()])
        .unwrap();
    assert!(asset.path().as_os_str().is_empty());
    assert_eq!(asset.read_at(0, 6).unwrap(), b"body{}");
    assert!(
        resources
            .capture(&["display names are not identities".into()])
            .is_err()
    );
    mount.changed.store(true, Ordering::Release);
    assert!(asset.read_at(0, 6).is_err());
    assert!(
        resources
            .capture(&["styles".into(), "文档.css".into()])
            .is_err()
    );
}

#[test]
fn archive_assets_deny_duplicate_raw_names_links_escapes_and_closed_leases() {
    let closed = Arc::new(AtomicBool::new(false));
    let main = asset_main(Arc::new(Assets::default()), closed.clone());
    let resources = main
        .resources()
        .expect("archive source must provide its captured subtree");
    for name in ["dup.css", "link.css", "..", ".", "a/b", "a\\b", "", "C:"] {
        assert!(resources.capture(&[name.into()]).is_err(), "{name}");
    }
    let asset = resources
        .capture(&["styles".into(), "文档.css".into()])
        .unwrap();
    closed.store(true, Ordering::Release);
    assert!(asset.read_at(0, 6).is_err());
    assert!(
        resources
            .capture(&["styles".into(), "文档.css".into()])
            .is_err()
    );
}
