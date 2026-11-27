use super::{Fixture, Harness};
use midnight_core::{Cancellation, CopyChoice, TransferService, TransferVerification};
use midnight_platform::LocalFileSystem;
use midnight_platform_windows::test_network_ids::{Response, Scope};
use std::{fs, io};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idless_file_retains_its_required_modified_time() {
    use midnight_core::{
        Command, CopyRequest, FileClipboard, FileClipboardIntent, MetadataTransferInput,
        MetadataTransferRequest, OperationId, TransferIntent, TransferMetadataPolicy,
    };
    let fixture = Fixture::new("network-modified-time");
    let share = fixture.directory("share");
    let source = share.join("time.txt");
    fs::write(&source, b"exact time").unwrap();
    let modified = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    let target = fixture.directory("target");
    let _scope = Scope::new(&share, Response::AllZero).unwrap();
    let mut harness = Harness::new();
    let request = CopyRequest {
        id: OperationId(1),
        start_policy: Default::default(),
        verification: TransferVerification::FullSha256,
        clipboard: FileClipboard {
            paths: vec![source],
            intent: FileClipboardIntent::Copy,
        },
        destination: target.clone(),
    };
    harness
        .commands
        .send(Command::EnqueueTransferWithMetadata(Box::new(
            MetadataTransferRequest {
                input: MetadataTransferInput::Paths(request),
                intent: TransferIntent::Copy,
                metadata: TransferMetadataPolicy::RequireModifiedTime,
            },
        )))
        .await
        .unwrap();
    assert_eq!(harness.completed(1).await.verified_bytes, 10);
    harness.close().await;
    assert_eq!(
        fs::metadata(target.join("time.txt"))
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
}

#[test]
fn idless_folder_copy_rejects_its_descendant_at_both_visible_names() {
    let fixture = Fixture::new("network-descendant");
    let source = fixture.directory("source");
    let child = source.join("child");
    fs::create_dir(&child).unwrap();
    let alias = fixture.root.join("alias");
    midnight_platform_windows::new_junction(&alias, &child).unwrap();
    let _scope = Scope::new(&source, Response::Unsupported).unwrap();
    let provider = LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    let held = provider.prepare_transfer_source(&source, &cancel).unwrap();
    assert!(held.requires_copy_directory_guard());
    for path in [&child, &alias] {
        let target = provider
            .prepare_transfer_destination(path, &cancel)
            .unwrap();
        let error = target
            .reject_source_ancestor(held.entry().identity, &cancel)
            .unwrap_err();
        assert_eq!(error.kind, io::ErrorKind::InvalidInput, "{error:?}");
    }
    assert_eq!(fs::read_dir(&child).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idless_folder_merge_guards_a_single_root_and_protects_other_selected_folders() {
    let fixture = Fixture::new("network-folder-merge");
    let share = fixture.directory("share");
    let _scope = Scope::new(&share, Response::Unsupported).unwrap();
    let source = share.join("source");
    let other = share.join("other");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&other).unwrap();
    fs::write(source.join("content"), b"source").unwrap();
    let mut harness = Harness::new();
    for (id, sources) in [(1, vec![source.clone()]), (2, vec![source.clone(), other])] {
        let target = fixture.directory(&format!("target-{id}"));
        fs::create_dir(target.join("source")).unwrap();
        harness
            .enqueue(
                id,
                &sources,
                &target,
                TransferVerification::FullSha256,
                false,
            )
            .await;
        let conflict = harness.conflict(id).await;
        assert_eq!(conflict.can_merge, sources.len() == 1);
        let (choice, output) = if conflict.can_merge {
            (CopyChoice::Merge { all: false }, "source")
        } else {
            (CopyChoice::Rename("second".into()), "second")
        };
        harness.decide(id, conflict.version, choice).await;
        assert_eq!(harness.completed(id).await.verified_bytes, 6);
        assert_eq!(
            fs::read(target.join(output).join("content")).unwrap(),
            b"source"
        );
    }
    harness.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idless_destination_conflict_offers_rename_when_the_folder_cannot_be_guarded() {
    let fixture = Fixture::new("network-merge-sharing");
    let share = fixture.directory("share");
    let source = fixture.directory("source");
    fs::write(source.join("file"), b"copied").unwrap();
    let occupied = share.join("source");
    fs::create_dir(&occupied).unwrap();
    let _scope = Scope::new(&share, Response::AllZero).unwrap();
    let cancel = Cancellation::default();
    let mut reader = LocalFileSystem::new(None)
        .prepare_transfer_source(&occupied, &cancel)
        .unwrap();
    assert!(reader.next_child(&cancel).unwrap().is_none());
    let mut harness = Harness::new();
    harness
        .enqueue(
            1,
            &[source],
            &share,
            TransferVerification::FullSha256,
            false,
        )
        .await;
    let conflict = harness.conflict(1).await;
    assert!(!conflict.can_merge);
    harness
        .decide(1, conflict.version, CopyChoice::Rename("available".into()))
        .await;
    assert_eq!(harness.completed(1).await.verified_bytes, 6);
    harness.close().await;
    assert_eq!(fs::read(share.join("available/file")).unwrap(), b"copied");
    assert_eq!(fs::read_dir(occupied).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streamed_idless_sources_keep_the_next_read_handle_through_a_conflict() {
    let fixture = Fixture::new("network-stream");
    let share = fixture.directory("share");
    let _scope = Scope::new(&share, Response::AllZero).unwrap();
    let target = fixture.directory("target");
    let mut paths = Vec::new();
    for value in 0..130u8 {
        let path = share.join(format!("item-{value}"));
        fs::write(&path, [value]).unwrap();
        paths.push(path);
    }
    fs::write(target.join("item-0"), b"keep").unwrap();
    let mut harness = Harness::new();
    harness
        .enqueue(1, &paths, &target, TransferVerification::FullSha256, false)
        .await;
    let conflict = harness.conflict(1).await;
    assert!(!conflict.can_replace);
    // The next root is pinned now. Roots beyond it have not been opened.
    let other = share.join("moved");
    assert_eq!(
        fs::rename(&paths[1], &other).unwrap_err().raw_os_error(),
        Some(32)
    );
    fs::rename(&paths[2], &other).unwrap();
    fs::rename(&other, &paths[2]).unwrap();
    harness
        .decide(1, conflict.version, CopyChoice::Rename("renamed".into()))
        .await;
    let result = harness.completed(1).await;
    assert_eq!(result.completed_roots, paths.len() as u64);
    assert_eq!(result.verified_bytes, paths.len() as u64);
    harness.close().await;
    assert_eq!(fs::read(target.join("item-0")).unwrap(), b"keep");
    assert_eq!(fs::read(target.join("renamed")).unwrap(), [0]);
    for (value, path) in paths.iter().enumerate().skip(1) {
        assert_eq!(
            fs::read(target.join(path.file_name().unwrap())).unwrap(),
            [value as u8]
        );
    }
}

/// Folder sizes, Properties and the folder tree tell folders apart by their size
/// identity. On a share that reports index 0 for every entry, each folder gets a key of
/// its canonical path instead, so no folder repeats its ancestor.
#[test]
fn zero_index_folders_have_distinct_size_identities() {
    use midnight_core::DirectoryService;
    let fixture = Fixture::new("network-zero-index");
    let share = fixture.directory("share");
    let child = share.join("child");
    fs::create_dir_all(child.join("grandchild")).unwrap();
    let _scope = Scope::new(&share, Response::ZeroIndex).unwrap();
    let provider = LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    let identity = |path: &std::path::Path| {
        provider
            .resolve_size_directory(path, &cancel)
            .unwrap()
            .unwrap()
            .identity
    };
    let identities = [
        identity(&share),
        identity(&child),
        identity(&child.join("grandchild")),
    ];
    assert_eq!(identities[1], identity(&child), "stable for one folder");
    assert_ne!(identities[0], identities[1]);
    assert_ne!(identities[1], identities[2]);
    assert_ne!(identities[0], identities[2]);
    assert!(identities.iter().all(|identity| identity.object != [0; 16]));
}
