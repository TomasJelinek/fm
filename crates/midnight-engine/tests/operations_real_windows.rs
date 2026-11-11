//! End-to-end Windows engine operations on retained, owned QA fixtures.
#![cfg(windows)]

#[path = "operations_real_windows/native.rs"]
mod native;
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
            assert_eq!(progress.state, CopyOperationState::CompletedWithErrors);
            assert_eq!(progress.skipped, 1);
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
        harness
            .enqueue(
                id,
                &[source],
                &destination,
                TransferVerification::default(),
                false,
            )
            .await;
    }
    harness
        .commands
        .send(Command::CancelCopy(OperationId(5)))
        .await
        .unwrap();
    harness.cancel_requested(5).await;
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
    harness.decide(1, conflict.version, CopyChoice::Skip).await;
    let skipped = harness.terminal(1).await;
    assert_eq!(skipped.state, CopyOperationState::CompletedWithErrors);
    assert_eq!(skipped.skipped, 1);
    for id in 2..=9 {
        if id == 5 {
            assert_eq!(
                harness.terminal(id).await.state,
                CopyOperationState::Cancelled
            );
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
    assert_eq!(harness.terminal_order, vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
    fixture.record("PASS nine queued jobs: first conflict deterministically gates mutation; queued #5 cancelled; seven successors copied");
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
