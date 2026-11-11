use crate::WindowsFileSystem;
use midnight_core::{
    Cancellation, DiscoveryScope, DiscoveryState, MutationService, OperationDiscovery,
    TransferService,
};
use std::{fs, path::Path};

fn fixture() -> tempfile::TempDir {
    let qa = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/qa/discovery");
    fs::create_dir_all(&qa).unwrap();
    let qa = qa.canonicalize().unwrap();
    tempfile::Builder::new()
        .prefix("metadata-")
        .tempdir_in(qa)
        .unwrap()
}
#[test]
fn discovery_counts_nested_unicode_without_consuming_copy_iterator_or_mutating_sources() {
    let fixture = fixture();
    let root = fixture.path().join("Příliš_中文_🧪");
    fs::create_dir_all(root.join("nested")).unwrap();
    fs::write(root.join("a.txt"), b"abc").unwrap();
    fs::write(root.join("nested/b.txt"), b"12345").unwrap();
    let service = WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let mut source = service.prepare_transfer_source(&root, &cancel).unwrap();
    let mut discovery = OperationDiscovery::scanning(DiscoveryScope::Recursive);
    source
        .discover(&cancel, &mut |item| discovery.observe(item))
        .unwrap();
    discovery.finish();
    assert_eq!(
        (discovery.files, discovery.directories, discovery.bytes),
        (2, 2, Some(8))
    );
    assert_eq!(discovery.state, DiscoveryState::Complete);
    let mut direct = 0;
    while source.next_child(&cancel).unwrap().is_some() {
        direct += 1;
    }
    assert_eq!(
        direct, 2,
        "discovery must own an independent enumeration cursor"
    );
    drop(source);
    let target = service.prepare(&root, &cancel).unwrap();
    let mut deletion = OperationDiscovery::scanning(DiscoveryScope::Recursive);
    target
        .discover(&cancel, &mut |item| deletion.observe(item))
        .unwrap();
    assert_eq!(
        (deletion.files, deletion.directories, deletion.bytes),
        (2, 2, Some(8))
    );
    assert_eq!(fs::read(root.join("nested/b.txt")).unwrap(), b"12345");
}
#[test]
fn discovery_cancels_before_visiting_a_retained_source() {
    let fixture = fixture();
    let root = fixture.path().join("source");
    fs::create_dir(&root).unwrap();
    let service = WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let source = service.prepare_transfer_source(&root, &cancel).unwrap();
    cancel.cancel();
    assert!(
        source
            .discover(&cancel, &mut |_| panic!(
                "cancelled discovery must not visit"
            ))
            .is_err()
    );
}
