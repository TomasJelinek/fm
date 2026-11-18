//! Windows: recursive attribute edits through the real adapter.
use super::*;
use std::os::windows::fs::MetadataExt;

#[tokio::test]
async fn recursive_hidden_attribute_reaches_1000_entries() {
    let mut h = Harness::new(&[("PARENT", true)]);
    let parent = h.directory.path().join("PARENT");
    files(&parent, "FILE", 1000);
    fs::create_dir(parent.join("SUB")).unwrap();
    files(&parent.join("SUB"), "NESTED", 10);
    let hidden = MetadataPatch {
        windows_mask: EDIT_HIDDEN,
        windows_value: EDIT_HIDDEN,
        recursive: true,
        ..Default::default()
    };
    let request = h.request.clone();
    confirmed(&mut h, request, hidden, MetadataConflictPolicy::Skip).await;
    let done = finished(&mut h, &[OperationId(1)]).await;
    let (progress, result) = &done[0];
    assert_eq!(progress.state, OperationState::Completed, "{progress:?}");
    assert_eq!(
        (result.processed, result.changed, result.failed),
        (1012, 1012, 0)
    );
    let mut paths = vec![parent.clone(), parent.join("SUB")];
    paths.extend((0..1000).map(|index| parent.join(format!("FILE{index:04}.TXT"))));
    paths.extend((0..10).map(|index| parent.join(format!("SUB/NESTED{index:04}.TXT"))));
    for path in paths {
        let attributes = fs::metadata(&path).unwrap().file_attributes();
        assert_eq!(attributes & EDIT_HIDDEN, EDIT_HIDDEN, "{path:?}");
    }
}
