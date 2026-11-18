//! Unix: case-sensitive sibling conflicts, a link next to its case alias and recursive
//! permission edits through the real adapter.
use super::*;
use std::os::unix::fs::PermissionsExt;

fn lower(times: bool) -> MetadataPatch {
    MetadataPatch {
        case: Some(CaseChange::default()),
        modified: if times {
            TimeEdit::Set(edited_time())
        } else {
            TimeEdit::Keep
        },
        recursive: true,
        ..Default::default()
    }
}

fn names_in(folder: &Path) -> Vec<OsString> {
    let mut names: Vec<_> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn recursive_permissions_reach_1000_entries() {
    let mut h = Harness::new(&[("PARENT", true)]);
    let parent = h.directory.path().join("PARENT");
    files(&parent, "FILE", 1000);
    fs::create_dir(parent.join("SUB")).unwrap();
    files(&parent.join("SUB"), "NESTED", 10);
    let group_write = MetadataPatch {
        unix_mask: 0o020,
        unix_value: 0o020,
        recursive: true,
        ..Default::default()
    };
    let request = h.request.clone();
    confirmed(&mut h, request, group_write, MetadataConflictPolicy::Skip).await;
    let done = finished(&mut h, &[OperationId(1)]).await;
    let (progress, result) = &done[0];
    assert_eq!(progress.state, OperationState::Completed, "{progress:?}");
    assert_eq!((result.processed, result.failed), (1012, 0));
    let mut paths = vec![parent.clone(), parent.join("SUB")];
    paths.extend((0..1000).map(|index| parent.join(format!("FILE{index:04}.TXT"))));
    paths.extend((0..10).map(|index| parent.join(format!("SUB/NESTED{index:04}.TXT"))));
    for path in paths {
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o020, 0o020, "{path:?}");
    }
}

#[tokio::test]
async fn case_sensitive_sibling_collisions_are_conflicts() {
    let mut h = Harness::new(&[("PARENT", true)]);
    let parent = h.directory.path().join("PARENT");
    for name in ["Ab", "AB", "Cd"] {
        fs::write(parent.join(name), name).unwrap();
    }
    let request = h.request.clone();
    confirmed(
        &mut h,
        request,
        lower(false),
        MetadataConflictPolicy::SkipAll,
    )
    .await;
    let done = finished(&mut h, &[OperationId(1)]).await;
    let (progress, result) = &done[0];
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    // "Ab" and "AB" would both become "ab": both are skipped.
    assert_eq!(
        (result.processed, result.skipped, result.failed),
        (4, 2, 0),
        "{progress:?}"
    );
    let parent = h.directory.path().join("parent");
    assert_eq!(names_in(&parent), ["AB", "Ab", "cd"]);
    for name in ["Ab", "AB"] {
        assert_eq!(fs::read(parent.join(name)).unwrap(), name.as_bytes());
    }
}

#[tokio::test]
async fn an_unchanged_name_keeps_its_time_next_to_a_link_that_takes_none() {
    let mut h = Harness::new(&[("PARENT", true)]);
    let parent = h.directory.path().join("PARENT");
    fs::write(parent.join("ab"), b"file").unwrap();
    std::os::unix::fs::symlink("ab", parent.join("AB")).unwrap();
    let request = h.request.clone();
    confirmed(
        &mut h,
        request,
        lower(true),
        MetadataConflictPolicy::SkipAll,
    )
    .await;
    let done = finished(&mut h, &[OperationId(1)]).await;
    let (progress, result) = &done[0];
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    // The link cannot take a time; "ab" keeps its name, so it is no conflict.
    assert_eq!(
        (result.processed, result.skipped, result.failed),
        (3, 0, 1),
        "{progress:?}"
    );
    assert_eq!(progress.errors[0].kind, io::ErrorKind::Unsupported);
    let parent = h.directory.path().join("parent");
    assert_eq!(names_in(&parent), ["AB", "ab"]);
    assert_eq!(modified_at(&parent.join("ab")), edited_time());
    assert!(
        fs::symlink_metadata(parent.join("AB"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}
