//! End-to-end Windows engine operations on owned QA fixtures, kept only when a test fails.
#![cfg(windows)]

#[path = "operations_real_windows/idless_tree.rs"]
mod idless_tree;
#[path = "operations_real_windows/native.rs"]
mod native;
#[path = "operations_real_windows/network_copy.rs"]
mod network_copy;
#[path = "support/qa_fixture.rs"]
mod qa_fixture;
#[path = "operations_real_windows/support.rs"]
mod support;

use midnight_core::{
    Cancellation, Command, CopyChoice, CopyOperationState, OperationId, TransferService,
    TransferVerification,
};
use midnight_platform::LocalFileSystem;
use std::{fs, time::Instant};
use support::{Fixture, Harness, manifest, populate};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_directory_corpus_matches_independent_sha256_for_both_policies() {
    let started = Instant::now();
    let fixture = Fixture::new("corpus");
    let source = fixture.directory("source");
    let (files, bytes) = populate(&source);
    let expected = manifest(&source);
    let mut harness = Harness::new();
    for (id, policy, name) in [
        (1, TransferVerification::default(), "sha256"),
        (2, TransferVerification::SizeAndSamples, "samples"),
    ] {
        let destination = fixture.directory(name);
        harness
            .enqueue(
                id,
                std::slice::from_ref(&source),
                &destination,
                policy,
                false,
            )
            .await;
        let progress = harness.completed(id).await;
        assert_eq!(progress.files_completed, files);
        assert_eq!(progress.copied_bytes, bytes);
        assert_eq!(progress.verification, policy);
        if policy == TransferVerification::default() {
            assert_eq!(progress.verified_bytes, bytes);
        } else {
            assert!(progress.verified_bytes < bytes);
        }
        assert_eq!(manifest(&destination.join("source")), expected);
        assert_eq!(manifest(&source), expected);
        fixture.record(&format!(
            "PASS corpus policy={policy:?} files={files} copied={bytes} verified={} elapsed={:?}",
            progress.verified_bytes,
            started.elapsed()
        ));
    }
    harness.close().await;
}

/// Run with MIDNIGHT_TEST_FAT_DIRECTORY pointing to a writable FAT32/exFAT folder.
/// Only the uniquely named temporary subdirectory on that volume receives any writes.
/// The tree copied there is then copied back to NTFS: a FAT source has no file IDs, so
/// Copy reads it through bindings its open handles keep (before, it was refused).
/// Skipped when the variable is unset.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "opt-in: needs MIDNIGHT_TEST_FAT_DIRECTORY on FAT32/exFAT media"]
async fn verified_copy_to_fat_preserves_tree_and_existing_file() {
    let Some(base) = std::env::var_os("MIDNIGHT_TEST_FAT_DIRECTORY").map(std::path::PathBuf::from)
    else {
        eprintln!("skipped: MIDNIGHT_TEST_FAT_DIRECTORY is not set");
        return;
    };
    let facts = LocalFileSystem::new(None)
        .link_destination(&base, &Cancellation::default())
        .unwrap();
    assert!(
        matches!(facts.filesystem.as_deref(), Some("FAT32" | "exFAT")),
        "expected FAT32/exFAT media, got {facts:?}"
    );
    let target = tempfile::Builder::new()
        .prefix("midnight-copy-")
        .tempdir_in(base)
        .unwrap();
    let fixture = Fixture::new("copy-to-fat");
    verified_copy_and_conflict(&fixture.root, target.path()).await;
    let back = fixture.directory("back");
    let mut harness = Harness::new();
    let on_fat = target.path().join("source");
    harness
        .enqueue(
            1,
            std::slice::from_ref(&on_fat),
            &back,
            TransferVerification::FullSha256,
            false,
        )
        .await;
    assert!(harness.completed(1).await.verified_bytes > 0);
    harness.close().await;
    assert_eq!(manifest(&back.join("source")), manifest(&on_fat));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verified_copy_to_native_directory_preserves_tree_and_existing_file() {
    let fixture = Fixture::new("copy-to-native");
    verified_copy_and_conflict(&fixture.root, &fixture.directory("target")).await;
}

async fn verified_copy_and_conflict(parent: &std::path::Path, target: &std::path::Path) {
    use midnight_core::{
        CopyLinkPolicy, CopyOptionsInput, CopyOptionsRequest, CopyRequest, FileClipboard,
        FileClipboardIntent, MetadataTransferInput, MetadataTransferRequest, TransferIntent,
        TransferMetadataPolicy,
    };
    let source = parent.join("source");
    fs::create_dir_all(source.join("nested/empty")).unwrap();
    let name = "a long filename — červená.bin";
    let file = source.join("nested").join(name);
    let bytes = vec![0x75; 180_123];
    fs::write(&file, &bytes).unwrap();
    // An even second is representable exactly on both NTFS and FAT32.
    let modified = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    fs::OpenOptions::new()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    let expected = manifest(&source);
    let verify = TransferVerification::FullSha256;
    let mut harness = Harness::new();
    let request = CopyRequest {
        id: OperationId(1),
        start_policy: Default::default(),
        verification: verify,
        clipboard: FileClipboard {
            paths: vec![source.clone()],
            intent: FileClipboardIntent::Copy,
        },
        destination: target.into(),
    };
    harness
        .commands
        .send(Command::EnqueueCopyWithOptions(Box::new(
            CopyOptionsRequest {
                input: CopyOptionsInput::Native(MetadataTransferRequest {
                    input: MetadataTransferInput::Paths(request),
                    intent: TransferIntent::Copy,
                    metadata: TransferMetadataPolicy::PreserveTimesAndAttributes,
                }),
                links: CopyLinkPolicy::FollowLinks,
            },
        )))
        .await
        .unwrap();
    let copied = harness.completed(1).await;
    assert_eq!(copied.verified_bytes, bytes.len() as u64);
    assert_eq!(manifest(&target.join("source")), expected);
    assert_eq!(
        fs::metadata(target.join("source/nested").join(name))
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );

    fs::write(target.join(name), b"original destination").unwrap();
    harness.enqueue(2, &[file], target, verify, false).await;
    let conflict = harness.conflict(2).await;
    harness
        .decide(
            2,
            conflict.version,
            CopyChoice::Rename("kept copy.bin".into()),
        )
        .await;
    let renamed = harness.completed(2).await;
    assert_eq!(renamed.verified_bytes, bytes.len() as u64);
    harness.close().await;
    assert_eq!(
        fs::read(target.join(name)).unwrap(),
        b"original destination"
    );
    assert_eq!(fs::read(target.join("kept copy.bin")).unwrap(), bytes);
    assert_eq!(fs::read_dir(target).unwrap().count(), 3);
    assert_eq!(manifest(&source), expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_same_volume_directory_move_preserves_identity_and_reads_zero_content() {
    let fixture = Fixture::new("native-move");
    let source = fixture.directory("source");
    let (files, bytes) = populate(&source);
    let expected = manifest(&source);
    let destination = fixture.directory("destination");
    let provider = LocalFileSystem::new(None);
    let identity = provider
        .prepare_move_source(&source, &Cancellation::default())
        .unwrap()
        .source
        .entry()
        .identity;
    let mut harness = Harness::new();
    harness
        .enqueue(
            1,
            std::slice::from_ref(&source),
            &destination,
            TransferVerification::default(),
            true,
        )
        .await;
    let progress = harness.completed(1).await;
    assert_eq!(progress.native_entries_moved, 1);
    assert_eq!(progress.copied_bytes, 0);
    assert_eq!(progress.verified_bytes, 0);
    assert_eq!(progress.validation_bytes, 0);
    assert!(!source.exists());
    assert_eq!(manifest(&destination.join("source")), expected);
    let moved_identity = provider
        .prepare_move_source(&destination.join("source"), &Cancellation::default())
        .unwrap()
        .source
        .entry()
        .identity;
    assert_eq!(identity, moved_identity);
    fixture.record(&format!("PASS native rename directory: files={files} bytes={bytes}; identity retained; zero copied/verified/validation bytes"));
    harness.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_overwrite_skip_rename_cancel_and_recoverable_replace_preserve_originals() {
    let fixture = Fixture::new("overwrite");
    let source = fixture.directory("source").join("same.txt");
    fs::write(&source, b"new source bytes").unwrap();
    let mut harness = Harness::new();
    for (id, choice, case) in [
        (1, CopyChoice::Skip, "skip"),
        (2, CopyChoice::Rename("renamed.txt".into()), "rename"),
        (3, CopyChoice::Cancel, "cancel"),
        (4, CopyChoice::ReplaceRecoverable { all: false }, "replace"),
    ] {
        let target = fixture.directory(case);
        fs::write(target.join("same.txt"), b"original destination").unwrap();
        harness
            .enqueue(
                id,
                std::slice::from_ref(&source),
                &target,
                TransferVerification::default(),
                false,
            )
            .await;
        let conflict = harness.conflict(id).await;
        assert!(conflict.can_replace);
        harness.decide(id, conflict.version, choice).await;
        let progress = harness.terminal(id).await;
        if case == "cancel" {
            assert_eq!(progress.state, CopyOperationState::Cancelled);
        } else if case == "skip" {
            // A skipped conflict is the user's choice, not an error.
            assert_eq!(progress.state, CopyOperationState::Completed);
            assert_eq!((progress.skipped, progress.failed), (1, 0));
        } else {
            assert_eq!(
                progress.state,
                CopyOperationState::Completed,
                "{progress:?}"
            );
        }
        assert_eq!(progress.failed, 0);
        assert_eq!(fs::read(&source).unwrap(), b"new source bytes");
        if case == "replace" {
            assert_eq!(
                fs::read(target.join("same.txt")).unwrap(),
                b"new source bytes"
            );
            assert_eq!(progress.recovery_count, 1);
            assert_eq!(progress.recovery_paths.len(), 1);
            assert!(progress.recovery_paths[0].starts_with(&fixture.root));
            assert_eq!(
                fs::read(&progress.recovery_paths[0]).unwrap(),
                b"original destination"
            );
        } else {
            assert_eq!(
                fs::read(target.join("same.txt")).unwrap(),
                b"original destination"
            );
            if case == "rename" {
                assert_eq!(
                    fs::read(target.join("renamed.txt")).unwrap(),
                    b"new source bytes"
                );
            } else {
                assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
            }
        }
        fixture.record(&format!("PASS conflict {case}: {progress:?}"));
    }
    harness.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_native_move_replacement_retains_displaced_file() {
    let fixture = Fixture::new("move-replace");
    let source = fixture.directory("source").join("same.txt");
    let destination = fixture.directory("destination");
    fs::write(&source, b"moving original").unwrap();
    fs::write(destination.join("same.txt"), b"displaced original").unwrap();
    let mut harness = Harness::new();
    harness
        .enqueue(
            1,
            std::slice::from_ref(&source),
            &destination,
            TransferVerification::default(),
            true,
        )
        .await;
    let conflict = harness.conflict(1).await;
    assert!(conflict.can_replace);
    harness
        .decide(
            1,
            conflict.version,
            CopyChoice::ReplaceRecoverable { all: false },
        )
        .await;
    let progress = harness.completed(1).await;
    assert_eq!(progress.native_entries_moved, 1);
    assert_eq!(progress.copied_bytes, 0);
    assert_eq!(progress.recovery_count, 1);
    assert!(!source.exists());
    assert_eq!(
        fs::read(destination.join("same.txt")).unwrap(),
        b"moving original"
    );
    assert!(progress.recovery_paths[0].starts_with(&fixture.root));
    assert_eq!(
        fs::read(&progress.recovery_paths[0]).unwrap(),
        b"displaced original"
    );
    fixture.record("PASS native Move replacement: source removed; previous target retained in owned recovery path");
    harness.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_missing_destination_retry_has_fresh_decision_version_and_no_stale_consent() {
    let fixture = Fixture::new("retry");
    let source = fixture.directory("source").join("file.bin");
    fs::write(&source, b"retained source").unwrap();
    let destination = fixture.root.join("not-ready");
    let mut harness = Harness::new();
    harness
        .enqueue(
            1,
            std::slice::from_ref(&source),
            &destination,
            TransferVerification::default(),
            false,
        )
        .await;
    let first = harness.failure(1).await;
    assert!(first.can_retry);
    harness.decide(1, first.version, CopyChoice::Retry).await;
    let second = harness.failure(1).await;
    assert!(second.version > first.version);
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("file.bin"), b"new target winner").unwrap();
    harness.decide(1, second.version, CopyChoice::Retry).await;
    let conflict = harness.conflict(1).await;
    assert!(conflict.version > second.version);
    harness
        .decide(
            1,
            first.version,
            CopyChoice::ReplaceRecoverable { all: true },
        )
        .await;
    harness
        .decide(
            1,
            conflict.version,
            CopyChoice::Rename("recovered.bin".into()),
        )
        .await;
    harness.completed(1).await;
    assert_eq!(
        fs::read(destination.join("file.bin")).unwrap(),
        b"new target winner"
    );
    assert_eq!(
        fs::read(destination.join("recovered.bin")).unwrap(),
        b"retained source"
    );
    assert_eq!(fs::read(&source).unwrap(), b"retained source");
    fixture.record("PASS actual missing-directory failure, repeated Retry, fresh conflict, stale decision rejection");
    harness.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_fifo_queue_cancels_queued_job_behind_a_conflict_gate() {
    let fixture = Fixture::new("fifo-cancel");
    let sources = fixture.directory("sources");
    let destination = fixture.directory("destination");
    let blocker = sources.join("blocker");
    fs::write(&blocker, b"source blocker").unwrap();
    fs::write(destination.join("blocker"), b"target blocker").unwrap();
    let mut harness = Harness::new();
    harness
        .enqueue(
            1,
            &[blocker],
            &destination,
            TransferVerification::default(),
            false,
        )
        .await;
    let conflict = harness.conflict(1).await;
    for id in 2..=9 {
        let source = sources.join(format!("job-{id}"));
        fs::write(&source, format!("payload-{id}")).unwrap();
        if id != 5 {
            fs::write(
                destination.join(format!("job-{id}")),
                format!("target-{id}"),
            )
            .unwrap();
        }
        harness
            .enqueue_with_policy(
                id,
                &[source],
                &destination,
                TransferVerification::default(),
                false,
                midnight_core::TransferStartPolicy::WaitForEarlier,
            )
            .await;
    }
    harness
        .commands
        .send(Command::CancelCopy(OperationId(5)))
        .await
        .unwrap();
    harness.cancel_requested(5).await;
    // Queued cancellation must finish while job1 still owns the conflict gate.
    let cancelled = harness.terminal(5).await;
    assert_eq!(cancelled.state, CopyOperationState::Cancelled);
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 8);
    assert!(!destination.join("job-5").exists());
    for id in [2, 3, 4, 6, 7, 8, 9] {
        assert_eq!(
            fs::read(destination.join(format!("job-{id}"))).unwrap(),
            format!("target-{id}").as_bytes()
        );
    }
    harness.decide(1, conflict.version, CopyChoice::Skip).await;
    // Native decision gates prove FIFO. Terminal delivery may arrive later.
    for id in [2, 3, 4, 6, 7, 8, 9] {
        let conflict = harness.next_conflict().await;
        assert_eq!(conflict.id, OperationId(id));
        harness
            .decide(
                id,
                conflict.version,
                CopyChoice::ReplaceRecoverable { all: false },
            )
            .await;
    }
    let skipped = harness.terminal(1).await;
    assert_eq!(skipped.state, CopyOperationState::Completed);
    assert_eq!((skipped.skipped, skipped.failed), (1, 0));
    for id in 2..=9 {
        if id == 5 {
            assert_eq!(cancelled.state, CopyOperationState::Cancelled);
            continue;
        }
        harness.completed(id).await;
        assert_eq!(
            fs::read(destination.join(format!("job-{id}"))).unwrap(),
            format!("payload-{id}").as_bytes()
        );
    }
    assert!(!destination.join("job-5").exists());
    assert_eq!(fs::read(sources.join("job-5")).unwrap(), b"payload-5");
    let mut terminal_ids = harness.terminal_order.clone();
    terminal_ids.sort_unstable();
    assert_eq!(terminal_ids, (1..=9).collect::<Vec<_>>());
    fixture.record("PASS opt-in FIFO native conflict gates; queued #5 cancelled before releasing #1; seven successor replacements verified");
    harness.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_self_and_hardlink_conflicts_cannot_replace_the_source() {
    let fixture = Fixture::new("identity-guards");
    let source_directory = fixture.directory("source");
    let source = source_directory.join("same.bin");
    fs::write(&source, b"one physical file").unwrap();
    let links = fixture.directory("hardlink");
    fs::hard_link(&source, links.join("same.bin")).unwrap();
    let mut harness = Harness::new();
    for (id, destination) in [(1, &source_directory), (2, &links)] {
        harness
            .enqueue(
                id,
                std::slice::from_ref(&source),
                destination,
                TransferVerification::default(),
                false,
            )
            .await;
        let conflict = harness.conflict(id).await;
        assert!(!conflict.can_replace && !conflict.can_merge);
        harness
            .decide(
                id,
                conflict.version,
                CopyChoice::ReplaceRecoverable { all: true },
            )
            .await;
        harness
            .decide(id, conflict.version, CopyChoice::Cancel)
            .await;
        assert_eq!(
            harness.terminal(id).await.state,
            CopyOperationState::Cancelled
        );
        assert_eq!(fs::read(&source).unwrap(), b"one physical file");
        assert_eq!(
            fs::read(links.join("same.bin")).unwrap(),
            b"one physical file"
        );
    }
    let child = source_directory.join("descendant");
    fs::create_dir(&child).unwrap();
    harness
        .enqueue(
            3,
            std::slice::from_ref(&source_directory),
            &child,
            TransferVerification::default(),
            false,
        )
        .await;
    let rejected = harness.terminal(3).await;
    assert_eq!(rejected.state, CopyOperationState::Failed);
    assert_eq!(rejected.failed, 1);
    assert_eq!(rejected.errors[0].kind, std::io::ErrorKind::InvalidInput);
    assert_eq!(fs::read_dir(child).unwrap().count(), 0);
    fixture.record("PASS self/hardlink replacement disabled; forged replace refused; directory descendant rejected before writes");
    harness.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_directory_move_merge_keeps_existing_children_and_removes_finished_source() {
    let fixture = Fixture::new("move-merge");
    let source = fixture.directory("source/tree");
    let destination = fixture.directory("destination");
    let target = fixture.directory("destination/tree");
    fs::create_dir(source.join("nested")).unwrap();
    fs::write(source.join("nested/new.txt"), b"new child").unwrap();
    fs::write(source.join("root.txt"), b"root child").unwrap();
    fs::write(target.join("keep.txt"), b"retained old child").unwrap();
    let mut harness = Harness::new();
    harness
        .enqueue(
            1,
            std::slice::from_ref(&source),
            &destination,
            TransferVerification::SizeAndSamples,
            true,
        )
        .await;
    let conflict = harness.conflict(1).await;
    assert!(conflict.can_merge);
    harness
        .decide(1, conflict.version, CopyChoice::Merge { all: true })
        .await;
    let progress = harness.completed(1).await;
    assert!(!source.exists());
    assert_eq!(
        fs::read(target.join("keep.txt")).unwrap(),
        b"retained old child"
    );
    assert_eq!(fs::read(target.join("root.txt")).unwrap(), b"root child");
    assert_eq!(
        fs::read(target.join("nested/new.txt")).unwrap(),
        b"new child"
    );
    fixture.record(&format!("PASS real directory Move merge: {progress:?}"));
    harness.close().await;
}
