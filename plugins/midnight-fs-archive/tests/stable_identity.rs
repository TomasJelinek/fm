//! Persistent host caches rely on indexing order, including implicit folders and duplicate names.
use midnight_filesystem_api::{Format, Kind};
use midnight_fs_archive::{Archive, Limits};

#[test]
fn reopening_identical_tar_preserves_ids_and_duplicate_member_content() {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, data) in [("folder/same.png", b"first"), ("folder/same.png", b"other")] {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, name, &data[..]).unwrap();
    }
    let bytes = builder.into_inner().unwrap();
    let first = Archive::open(Format::Tar, bytes.clone(), Limits::default()).unwrap();
    let mut reopened = Archive::open(Format::Tar, bytes, Limits::default()).unwrap();
    let folder = first.list(1, 0).unwrap().remove(0);
    assert_eq!(folder.kind, Kind::Directory);
    assert_eq!(reopened.list(1, 0).unwrap()[0].id, folder.id);
    let members = first.list(folder.id, 0).unwrap();
    let again = reopened.list(folder.id, 0).unwrap();
    assert_eq!(members.len(), 2);
    assert_ne!(members[0].id, members[1].id);
    assert_eq!(members[0].name, members[1].name);
    assert_eq!(members[0].id, again[0].id);
    assert_eq!(members[1].id, again[1].id);
    assert_eq!(reopened.read(members[0].id, 0, 5).unwrap().0, b"first");
    assert_eq!(reopened.read(members[1].id, 0, 5).unwrap().0, b"other");
}
