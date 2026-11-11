#[path = "../src/checksum.rs"]
#[allow(dead_code)] // This fixture does not include the tree-capture admission caller.
mod checksum;

use midnight_core::*;
use std::{fs, sync::Arc, time::Duration};

#[cfg(windows)]
#[path = "support/checksum_fixture_windows.rs"]
mod native_fixture;

#[cfg(windows)]
#[path = "support/checksum_verify_links_windows.rs"]
mod links;
#[cfg(target_os = "linux")]
#[path = "support/checksum_verify_links_linux.rs"]
mod links;
#[cfg(target_os = "linux")]
#[path = "support/checksum_fixture_linux.rs"]
mod native_fixture;

async fn verify(path: &std::path::Path) -> Arc<ChecksumRun> {
    let mut controller =
        checksum::Controller::new(Arc::new(midnight_platform::LocalFileSystem::new(None)));
    controller
        .verify(ChecksumVerifyRequest {
            job: ChecksumJobId(1),
            generation: ChecksumGeneration(1),
            manifest: path.to_owned(),
            limits: ChecksumLimits::default(),
        })
        .unwrap();
    let run = tokio::time::timeout(Duration::from_secs(10), async {
        let mut advertised = None;
        loop {
            let event = controller.next().await.unwrap();
            match event.update {
                ChecksumUpdate::Verifying(algorithm) => advertised = Some(algorithm),
                ChecksumUpdate::Batch(rows) => {
                    assert!(advertised.is_some());
                    assert!(
                        rows.iter()
                            .all(|row| row.state != ChecksumRowState::Calculated)
                    );
                }
                ChecksumUpdate::Finished(run) => break run,
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    controller.shutdown().await;
    run
}

#[tokio::test]
async fn native_manifest_verifies_all_five_published_abc_vectors_and_unicode_nested_names() {
    let _quota = checksum::test_quota_fixture().await;
    let fixture = native_fixture::fixture();
    fs::create_dir(fixture.path().join("č文🦀")).unwrap();
    fs::write(fixture.path().join("č文🦀/abc file.txt"), b"abc").unwrap();
    for (algorithm, digest) in [
        (ChecksumAlgorithm::Crc32, "352441c2"),
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
        let path = fixture
            .path()
            .join(format!("files.{}", algorithm.extension()));
        let text = if algorithm == ChecksumAlgorithm::Crc32 {
            format!("č文🦀/abc file.txt {digest}\r\n")
        } else {
            format!("{digest} *č文🦀/abc file.txt\r\n")
        };
        fs::write(&path, text).unwrap();
        let run = verify(&path).await;
        assert_eq!(
            run.summary.completion,
            ChecksumCompletion::Completed,
            "{:?}",
            run.summary.diagnostics
        );
        assert_eq!(run.summary.progress.verified, 1);
        assert_eq!(run.rows[0].state, ChecksumRowState::Verified);
        assert_eq!(run.rows[0].digest(algorithm).unwrap().hex(), digest);
    }
}

#[tokio::test]
async fn native_mismatch_and_missing_are_explicit_and_cannot_look_verified() {
    let _quota = checksum::test_quota_fixture().await;
    let fixture = native_fixture::fixture();
    fs::write(fixture.path().join("corrupt"), b"not abc").unwrap();
    let manifest = fixture.path().join("files.md5");
    fs::write(
        &manifest,
        "900150983cd24fb0d6963f7d28e17f72 *corrupt\n900150983cd24fb0d6963f7d28e17f72 *missing\n",
    )
    .unwrap();
    let run = verify(&manifest).await;
    assert_eq!(
        run.summary.completion,
        ChecksumCompletion::CompletedWithErrors
    );
    assert_eq!(run.summary.progress.verified, 0);
    assert_eq!(run.summary.progress.mismatched, 1);
    assert_eq!(run.summary.progress.missing, 1);
    assert_eq!(run.rows[0].state, ChecksumRowState::Mismatch);
    assert!(run.rows[0].digest(ChecksumAlgorithm::Md5).is_some());
    assert_eq!(run.rows[1].state, ChecksumRowState::Missing);
    assert!(run.rows[1].digests.is_empty());
}

#[tokio::test]
async fn malformed_manifest_fails_as_a_whole_and_cancelled_queue_never_reads_targets() {
    let _quota = checksum::test_quota_fixture().await;
    let fixture = native_fixture::fixture();
    let manifest = fixture.path().join("files.md5");
    fs::write(
        &manifest,
        "900150983cd24fb0d6963f7d28e17f72 *valid\ninvalid *bad\n",
    )
    .unwrap();
    let run = verify(&manifest).await;
    assert_eq!(run.summary.completion, ChecksumCompletion::Failed);
    assert!(run.rows.is_empty());
    drop(run);
    let mut controller =
        checksum::Controller::new(Arc::new(midnight_platform::LocalFileSystem::new(None)));
    controller
        .verify(ChecksumVerifyRequest {
            job: ChecksumJobId(1),
            generation: ChecksumGeneration(1),
            manifest,
            limits: ChecksumLimits::default(),
        })
        .unwrap();
    controller.cancel(ChecksumJobId(1), ChecksumGeneration(1));
    let run = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let ChecksumUpdate::Finished(run) = controller.next().await.unwrap().update {
                break run;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(run.summary.completion, ChecksumCompletion::Cancelled);
    assert!(run.rows.is_empty());
    controller.shutdown().await;
}

#[tokio::test]
async fn native_directory_links_are_rejected_in_manifest_ancestry_and_listed_paths() {
    let _quota = checksum::test_quota_fixture().await;
    let fixture = native_fixture::fixture();
    let real = fixture.path().join("real");
    let alias = fixture.path().join("alias");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("data"), b"abc").unwrap();
    fs::write(
        real.join("files.md5"),
        "900150983cd24fb0d6963f7d28e17f72 *data\n",
    )
    .unwrap();
    links::create(&alias, &real);
    let run = verify(&alias.join("files.md5")).await;
    assert_eq!(run.summary.completion, ChecksumCompletion::Failed);
    assert!(run.rows.is_empty());
    drop(run);
    let manifest = fixture.path().join("files.md5");
    fs::write(&manifest, "900150983cd24fb0d6963f7d28e17f72 *alias/data\n").unwrap();
    let run = verify(&manifest).await;
    assert_eq!(
        run.summary.completion,
        ChecksumCompletion::CompletedWithErrors
    );
    assert_eq!(run.summary.progress.verified, 0);
    assert!(run.rows[0].digests.is_empty());
    assert_eq!(fs::read(real.join("data")).unwrap(), b"abc");
}
