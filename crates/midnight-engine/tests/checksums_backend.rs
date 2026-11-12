//! Standalone checksum worker integration; actor and tree admission are tested separately.
#[path = "../src/retirement.rs"]
#[allow(dead_code)]
mod retirement;

#[path = "../src/checksum.rs"]
#[allow(dead_code)] // This fixture does not include the tree-capture admission caller.
mod checksum;

use midnight_core::*;
use std::{fs, sync::Arc, time::Duration};

#[cfg(windows)]
#[path = "support/checksum_fixture_windows.rs"]
mod native_fixture;
#[cfg(target_os = "linux")]
#[path = "support/checksum_fixture_linux.rs"]
mod native_fixture;
use native_fixture::fixture;

#[tokio::test]
async fn published_abc_vectors_are_calculated_together_and_manifest_is_verified() {
    let _quota_fixture = checksum::test_quota_fixture().await;
    let temp = fixture();
    let path = temp.path().join("č文🦀 abc.txt");
    fs::write(&path, b"abc").unwrap();
    let mut service = checksum::Controller::new(
        checksum::Retirement::testing(),
        Arc::new(midnight_platform::LocalFileSystem::new(None)),
    );
    let mut request =
        ChecksumRequest::new(ChecksumJobId(1), ChecksumGeneration(1), vec![path.clone()]);
    request.algorithms = ChecksumAlgorithms::all();
    service.start(request).unwrap();
    let run = loop {
        let event = tokio::time::timeout(Duration::from_secs(10), service.next())
            .await
            .unwrap()
            .unwrap();
        if let ChecksumUpdate::Finished(run) = event.update {
            break run;
        }
    };
    assert_eq!(run.summary.completion, ChecksumCompletion::Completed);
    assert_eq!(run.summary.progress.bytes_read, 3);
    assert_eq!(run.rows.len(), 1);
    let row = &run.rows[0];
    assert_eq!(row.state, ChecksumRowState::Calculated);
    for (algorithm, expected) in [
        (ChecksumAlgorithm::Md5, "900150983cd24fb0d6963f7d28e17f72"),
        (
            ChecksumAlgorithm::Sha1,
            "a9993e364706816aba3e25717850c26c9cd0d89d",
        ),
        (
            ChecksumAlgorithm::Sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            ChecksumAlgorithm::Sha512,
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
        ),
    ] {
        assert_eq!(row.digest(algorithm).unwrap().hex(), expected);
    }
    let manifest = temp.path().join("files.sha256");
    let save = ChecksumManifestRequest {
        algorithm: ChecksumAlgorithm::Sha256,
        destination: manifest.clone(),
        acknowledge_partial: None,
    };
    let receipt = checksum::save_manifest(
        &run,
        &save,
        &midnight_platform::LocalFileSystem::new(None),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(receipt.included, 1);
    assert_eq!(
        fs::read_to_string(&manifest).unwrap(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad *č文🦀 abc.txt\n"
    );
    assert!(
        checksum::save_manifest(
            &run,
            &save,
            &midnight_platform::LocalFileSystem::new(None),
            &Cancellation::default()
        )
        .is_err()
    );
    let alias = temp.path().join("source-alias.sha256");
    fs::hard_link(&path, &alias).unwrap();
    let alias_save = ChecksumManifestRequest {
        destination: alias,
        ..save.clone()
    };
    assert_eq!(
        checksum::save_manifest(
            &run,
            &alias_save,
            &midnight_platform::LocalFileSystem::new(None),
            &Cancellation::default()
        )
        .unwrap_err()
        .kind,
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(fs::read(path).unwrap(), b"abc");
    service.shutdown().await;
}

#[tokio::test]
async fn recursion_is_explicit_and_partial_manifests_require_exact_acknowledgement() {
    let _quota_fixture = checksum::test_quota_fixture().await;
    let temp = fixture();
    let directory = temp.path().join("folder");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("empty.txt"), b"").unwrap();
    let provider = Arc::new(midnight_platform::LocalFileSystem::new(None));
    let mut controller =
        checksum::Controller::new(checksum::Retirement::testing(), provider.clone());
    let request = ChecksumRequest::new(
        ChecksumJobId(2),
        ChecksumGeneration(1),
        vec![directory.clone()],
    );
    controller.start(request.clone()).unwrap();
    let skipped = loop {
        if let ChecksumUpdate::Finished(run) = controller.next().await.unwrap().update {
            break run;
        }
    };
    assert_eq!(skipped.rows.len(), 1);
    assert_eq!(skipped.rows[0].state, ChecksumRowState::Skipped);
    assert!(skipped.rows[0].digests.is_empty());
    let mut recursive = request;
    recursive.generation = ChecksumGeneration(2);
    recursive.recursive = true;
    recursive
        .targets
        .push(temp.path().join("missing.txt").into());
    recursive.targets.last_mut().unwrap().kind = Some(EntryKind::File);
    controller.start(recursive).unwrap();
    let run = loop {
        if let ChecksumUpdate::Finished(run) = controller.next().await.unwrap().update {
            break run;
        }
    };
    assert_eq!(
        run.summary.completion,
        ChecksumCompletion::CompletedWithErrors
    );
    assert_eq!(run.summary.progress.calculated, 1);
    assert_eq!(run.summary.progress.errors, 1);
    let mut save = ChecksumManifestRequest {
        algorithm: ChecksumAlgorithm::Sha256,
        destination: temp.path().join("partial.sha256"),
        acknowledge_partial: None,
    };
    assert!(
        checksum::save_manifest(&run, &save, provider.as_ref(), &Cancellation::default()).is_err()
    );
    save.acknowledge_partial = Some(ChecksumPartialAcknowledgement {
        included: 1,
        excluded: 1,
        incomplete_discovery: false,
    });
    let receipt =
        checksum::save_manifest(&run, &save, provider.as_ref(), &Cancellation::default()).unwrap();
    assert_eq!((receipt.included, receipt.excluded), (1, 1));
    assert_eq!(
        fs::read_to_string(&save.destination).unwrap(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855 *folder/empty.txt\n"
    );
    // The manifest itself can never replace one of the hashed inputs.
    save.destination = directory.join("empty.txt");
    assert!(
        checksum::save_manifest(&run, &save, provider.as_ref(), &Cancellation::default()).is_err()
    );
    assert_eq!(fs::read(directory.join("empty.txt")).unwrap(), b"");
    controller.shutdown().await;
}

#[cfg(target_os = "linux")]
#[path = "support/checksum_native_unix.rs"]
mod native_unix;

#[tokio::test]
async fn unknown_recursive_failure_requires_explicit_incomplete_discovery_acknowledgement() {
    let _quota_fixture = checksum::test_quota_fixture().await;
    let temp = fixture();
    let path = temp.path().join("good.txt");
    fs::write(&path, b"abc").unwrap();
    let provider = Arc::new(midnight_platform::LocalFileSystem::new(None));
    let mut controller =
        checksum::Controller::new(checksum::Retirement::testing(), provider.clone());
    let mut request = ChecksumRequest::new(
        ChecksumJobId(70),
        ChecksumGeneration(1),
        vec![path, temp.path().join("unknown-subtree")],
    );
    request.recursive = true;
    controller.start(request).unwrap();
    let run = loop {
        if let ChecksumUpdate::Finished(run) = controller.next().await.unwrap().update {
            break run;
        }
    };
    assert_eq!(run.summary.completion, ChecksumCompletion::Incomplete);
    let mut save = ChecksumManifestRequest {
        algorithm: ChecksumAlgorithm::Sha256,
        destination: temp.path().join("partial.sha256"),
        acknowledge_partial: Some(ChecksumPartialAcknowledgement {
            included: 1,
            excluded: 1,
            incomplete_discovery: false,
        }),
    };
    assert!(
        checksum::save_manifest(&run, &save, provider.as_ref(), &Cancellation::default()).is_err()
    );
    save.acknowledge_partial
        .as_mut()
        .unwrap()
        .incomplete_discovery = true;
    assert!(
        checksum::save_manifest(&run, &save, provider.as_ref(), &Cancellation::default()).is_ok()
    );
    controller.shutdown().await;
}

#[tokio::test]
async fn captured_metadata_change_is_not_silently_hashed_as_the_old_selection() {
    let _quota_fixture = checksum::test_quota_fixture().await;
    let temp = fixture();
    let path = temp.path().join("changed.txt");
    fs::write(&path, b"now much longer").unwrap();
    let mut controller = checksum::Controller::new(
        checksum::Retirement::testing(),
        Arc::new(midnight_platform::LocalFileSystem::new(None)),
    );
    let mut request = ChecksumRequest::new(ChecksumJobId(3), ChecksumGeneration(1), vec![path]);
    request.targets[0].size = Some(1);
    controller.start(request).unwrap();
    let run = loop {
        if let ChecksumUpdate::Finished(run) = controller.next().await.unwrap().update {
            break run;
        }
    };
    assert_eq!(run.rows[0].state, ChecksumRowState::SourceChanged);
    assert!(run.rows[0].digests.is_empty());
    controller.shutdown().await;
}

#[tokio::test]
async fn recursive_depth_node_and_memory_limits_report_incomplete_discovery() {
    let _quota_fixture = checksum::test_quota_fixture().await;
    let temp = fixture();
    let root = temp.path().join("root");
    fs::create_dir_all(root.join("child/deeper")).unwrap();
    fs::write(root.join("child/deeper/file"), b"unvisited").unwrap();
    let mut controller = checksum::Controller::new(
        checksum::Retirement::testing(),
        Arc::new(midnight_platform::LocalFileSystem::new(None)),
    );
    for (index, budget) in [
        ChecksumBudget::Depth,
        ChecksumBudget::Nodes,
        ChecksumBudget::RetainedBytes,
    ]
    .into_iter()
    .enumerate()
    {
        let mut request = ChecksumRequest::new(
            ChecksumJobId(60 + index as u64),
            ChecksumGeneration(1),
            vec![root.clone()],
        );
        request.recursive = true;
        match budget {
            ChecksumBudget::Depth => request.limits.max_depth = 1,
            ChecksumBudget::Nodes => request.limits.max_nodes = 1,
            _ => request.limits.max_retained_bytes = 1,
        }
        controller.start(request).unwrap();
        let run = loop {
            if let ChecksumUpdate::Finished(run) = controller.next().await.unwrap().update {
                break run;
            }
        };
        assert_eq!(run.summary.completion, ChecksumCompletion::Incomplete);
        assert_eq!(run.summary.progress.calculated, 0);
        assert!(run.summary.reached_budgets.contains(&budget));
    }
    controller.shutdown().await;
}
