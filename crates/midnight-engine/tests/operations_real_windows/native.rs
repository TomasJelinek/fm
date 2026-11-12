//! Native disk boundary checks. No sleeps or global Recycle Bin cleanup.
use super::support::{Fixture, Harness, manifest, populate};
use midnight_core::{
    Cancellation, CopyChoice, DeleteMode, MutationService, TransferService, TransferVerification,
};
use midnight_platform::LocalFileSystem;
use midnight_platform_common::transfer::{CopyPhase, VerifiedFileCopy};
use std::{ffi::OsStr, fs, io, os::windows::fs::OpenOptionsExt, path::Path};

fn copy(source: &Path, destination: &Path, policy: TransferVerification) -> VerifiedFileCopy {
    let provider = LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    let source = provider.prepare_transfer_source(source, &cancel).unwrap();
    let destination = provider
        .prepare_transfer_destination(destination, &cancel)
        .unwrap();
    VerifiedFileCopy::new_with_verification(
        source,
        destination.stage_file(&cancel).unwrap(),
        policy,
    )
    .unwrap()
}

fn advance(job: &mut VerifiedFileCopy, phase: CopyPhase) {
    let cancel = Cancellation::default();
    for _ in 0..10_000 {
        if job.phase() == phase {
            return;
        }
        job.step(&cancel).unwrap();
    }
    panic!("copy failed to reach {phase:?}");
}

#[test]
fn real_cancel_after_write_sync_and_verify_cleans_owned_stage() {
    let fixture = Fixture::new("phase-cancel");
    let source = fixture.root.join("source.bin");
    let bytes: Vec<u8> = (0..1_048_593)
        .map(|index| (index * 7 + index / 131) as u8)
        .collect();
    fs::write(&source, &bytes).unwrap();
    for (policy_index, policy) in [
        TransferVerification::default(),
        TransferVerification::SizeAndSamples,
    ]
    .into_iter()
    .enumerate()
    {
        for phase in [
            CopyPhase::Copying,
            CopyPhase::Syncing,
            CopyPhase::Verifying,
            CopyPhase::ReadyToPublish,
        ] {
            let target = fixture.directory(&format!("{policy_index}-{phase:?}"));
            let mut job = copy(&source, &target, policy);
            if phase == CopyPhase::Copying {
                job.step(&Cancellation::default()).unwrap();
                assert_eq!(job.copied_bytes(), 262_144);
            } else {
                advance(&mut job, phase);
                if phase == CopyPhase::Verifying {
                    job.step(&Cancellation::default()).unwrap();
                    assert!(job.verified_bytes() > 0);
                }
            }
            assert_eq!(
                fs::read_dir(&target).unwrap().count(),
                1,
                "one owned unpublished stage"
            );
            let cancel = Cancellation::default();
            cancel.cancel();
            let error = if phase == CopyPhase::ReadyToPublish {
                job.publish(OsStr::new("published.bin"), &cancel)
                    .unwrap_err()
            } else {
                job.step(&cancel).unwrap_err()
            };
            assert_eq!(error.kind, io::ErrorKind::Interrupted);
            assert!(!job.can_retry());
            job.cleanup().unwrap();
            assert_eq!(fs::read(&source).unwrap(), bytes);
            // Windows can retain the delete-pending directory entry until this
            // stage handle closes. Engine::Job::finish closes it before terminal
            // progress; mirror that complete cleanup boundary, including errors.
            drop(job);
            assert_eq!(
                fs::read_dir(&target).unwrap().count(),
                0,
                "cleanup and native handle release must remove the stage"
            );
            fixture.record(&format!("PASS real cancellation policy={policy:?} gate={phase:?}: no publication, source intact, cleanup and handle release leave no stage"));
        }
    }
}

#[test]
fn real_late_publication_collision_preserves_verified_stage_and_both_originals() {
    let fixture = Fixture::new("late-collision");
    let source = fixture.root.join("source.bin");
    fs::write(&source, b"verified source bytes").unwrap();
    let target = fixture.directory("target");
    let mut job = copy(&source, &target, TransferVerification::default());
    advance(&mut job, CopyPhase::ReadyToPublish);
    fs::write(target.join("winner.bin"), b"late external winner").unwrap();
    let copied = job.copied_bytes();
    assert_eq!(
        job.publish(OsStr::new("winner.bin"), &Cancellation::default())
            .unwrap_err()
            .kind,
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(job.phase(), CopyPhase::ReadyToPublish);
    job.publish(OsStr::new("renamed.bin"), &Cancellation::default())
        .unwrap();
    assert_eq!(job.copied_bytes(), copied);
    drop(job);
    assert_eq!(fs::read(&source).unwrap(), b"verified source bytes");
    assert_eq!(
        fs::read(target.join("winner.bin")).unwrap(),
        b"late external winner"
    );
    assert_eq!(
        fs::read(target.join("renamed.bin")).unwrap(),
        b"verified source bytes"
    );
    assert_eq!(fs::read_dir(&target).unwrap().count(), 2);
    fixture.record("PASS real publication race: late winner retained; verified bytes published under new name without recopy or stage residue");
}

#[test]
fn real_windows_source_pin_excludes_writers_and_namespace_replacement() {
    let fixture = Fixture::new("source-pinning");
    let source = fixture.root.join("source.bin");
    fs::write(&source, vec![91; 524_321]).unwrap();
    let target = fixture.directory("target");
    let mut job = copy(&source, &target, TransferVerification::default());
    job.step(&Cancellation::default()).unwrap();
    assert!(fs::OpenOptions::new().write(true).open(&source).is_err());
    assert!(fs::rename(&source, fixture.root.join("substituted.bin")).is_err());
    assert!(fs::remove_file(&source).is_err());
    advance(&mut job, CopyPhase::ReadyToPublish);
    job.publish(OsStr::new("copy.bin"), &Cancellation::default())
        .unwrap();
    drop(job);
    assert_eq!(
        fs::read(&source).unwrap(),
        fs::read(target.join("copy.bin")).unwrap()
    );
    fixture.record("PASS actual concurrent write/rename/delete attempts rejected by source sharing pin; verified published content unchanged");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_sharing_violation_retry_succeeds_only_after_exclusive_handle_is_released() {
    let fixture = Fixture::new("locked-retry");
    let source = fixture.directory("source").join("locked.bin");
    fs::write(&source, b"locked file original").unwrap();
    let destination = fixture.directory("destination");
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&source)
        .unwrap();
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
    let failure = harness.failure(1).await;
    assert!(failure.can_retry);
    assert_eq!(failure.error.kind, io::Error::from_raw_os_error(32).kind());
    assert!(
        failure.error.message.ends_with("(os error 32)"),
        "{:?}",
        failure.error
    );
    harness.decide(1, failure.version, CopyChoice::Retry).await;
    let still_locked = harness.failure(1).await;
    assert!(still_locked.can_retry);
    assert!(still_locked.version > failure.version);
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
    drop(locked);
    harness
        .decide(1, still_locked.version, CopyChoice::Retry)
        .await;
    harness.completed(1).await;
    assert_eq!(
        fs::read(destination.join("locked.bin")).unwrap(),
        b"locked file original"
    );
    assert_eq!(fs::read(&source).unwrap(), b"locked file original");
    fixture.record("PASS real Windows sharing violation; Retry fails while handle retained and succeeds after explicit release");
    harness.close().await;
}

#[test]
fn real_permanent_delete_cancels_at_first_reported_node_then_resumes_owned_tree() {
    let fixture = Fixture::new("delete-cancel");
    let tree = fixture.directory("tree");
    let (files, bytes) = populate(&tree);
    let before = manifest(&tree);
    let outside = fixture.directory("outside");
    let sentinel = outside.join("hardlink-readonly.txt");
    fs::hard_link(tree.join("readonly.txt"), &sentinel).unwrap();
    let provider = LocalFileSystem::new(None);
    let item = provider.prepare(&tree, &Cancellation::default()).unwrap();
    let cancel = Cancellation::default();
    let mut removed = Vec::new();
    let error = item
        .delete(DeleteMode::Permanent, &cancel, &mut |path| {
            assert!(path.starts_with(&tree));
            removed.push(path.to_owned());
            cancel.cancel();
        })
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::Interrupted);
    assert_eq!(removed.len(), 1);
    assert!(tree.exists());
    let remaining = manifest(&tree);
    assert!(remaining.len() < before.len());
    for (path, expected) in &remaining {
        assert_eq!(before.get(path), Some(expected));
    }
    drop(item);
    let mut resumed_nodes = 0;
    provider
        .prepare(&tree, &Cancellation::default())
        .unwrap()
        .delete(
            DeleteMode::Permanent,
            &Cancellation::default(),
            &mut |path| {
                assert!(path.starts_with(&tree));
                resumed_nodes += 1;
            },
        )
        .unwrap();
    assert!(!tree.exists());
    assert_eq!(fs::read(&sentinel).unwrap(), b"read only source");
    assert!(fs::metadata(&sentinel).unwrap().permissions().readonly());
    fixture.record(&format!("PASS permanent recursive deletion: {files} files/{bytes} bytes; cancel after exactly one node; {resumed_nodes} resumed nodes; outside readonly hardlink retained"));
}

#[test]
fn real_delete_pin_rejects_replaced_source_and_precancelled_delete_preserves_bytes() {
    let fixture = Fixture::new("delete-identity");
    let source = fixture.root.join("source.bin");
    fs::write(&source, b"original").unwrap();
    let provider = LocalFileSystem::new(None);
    let item = provider.prepare(&source, &Cancellation::default()).unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    for mode in [DeleteMode::Permanent, DeleteMode::Trash] {
        assert_eq!(
            item.delete(mode, &cancel, &mut |_| panic!(
                "cancelled delete reported mutation"
            ))
            .unwrap_err()
            .kind,
            io::ErrorKind::Interrupted
        );
    }
    assert_eq!(fs::read(&source).unwrap(), b"original");
    let original = fixture.root.join("original-moved.bin");
    fs::rename(&source, &original).unwrap();
    fs::write(&source, b"namespace replacement").unwrap();
    assert!(
        item.delete(DeleteMode::Permanent, &Cancellation::default(), &mut |_| {})
            .is_err()
    );
    assert_eq!(fs::read(source).unwrap(), b"namespace replacement");
    assert_eq!(fs::read(original).unwrap(), b"original");
    fixture.record("PASS precancelled permanent/Trash preserve source; stale prepared delete refuses real namespace replacement");
}

#[test]
#[ignore = "manual QA: recycles one fresh owned folder; never empties the Recycle Bin"]
fn real_recycle_owned_fixture_without_permanent_fallback() {
    let fixture = Fixture::new("recycle");
    let tree = fixture.directory("Midnight Explorer QA recycle owned folder");
    fs::create_dir(tree.join("nested")).unwrap();
    fs::write(
        tree.join("nested/Žluťoučký 🧪.txt"),
        b"owned QA recycle contents",
    )
    .unwrap();
    let sentinel = fixture.root.join("unrelated-sentinel.txt");
    fs::write(&sentinel, b"must remain").unwrap();
    let provider = LocalFileSystem::new(None);
    let mut recycled = Vec::new();
    let result = provider
        .prepare(&tree, &Cancellation::default())
        .unwrap()
        .delete(DeleteMode::Trash, &Cancellation::default(), &mut |path| {
            recycled.push(path.to_owned())
        });
    match result {
        Ok(()) => {
            assert!(!tree.exists());
            assert_eq!(recycled, vec![tree]);
            fixture.record("PASS actual Windows dedicated recycle API accepted owned folder; no global Recycle Bin enumeration or cleanup performed");
        }
        Err(error) if error.kind == io::ErrorKind::Unsupported => {
            assert!(tree.join("nested/Žluťoučký 🧪.txt").exists());
            fixture.record(&format!("UNSUPPORTED actual recycle service: {error}; source retained, no permanent fallback"));
        }
        Err(error) => panic!("actual recycle failed: {error}"),
    }
    assert_eq!(fs::read(sentinel).unwrap(), b"must remain");
}
