use super::*;
use std::{
    ffi::OsString,
    os::unix::{ffi::OsStringExt, fs::symlink},
};

#[tokio::test]
async fn recursive_links_are_skipped_without_abandoning_regular_siblings() {
    let _quota_fixture = checksum::test_quota_fixture().await;
    let temp = fixture();
    let directory = temp.path().join("directory");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("regular"), b"abc").unwrap();
    fs::write(temp.path().join("outside"), b"do not hash through link").unwrap();
    symlink(temp.path().join("outside"), directory.join("link")).unwrap();
    let mut controller = checksum::Controller::new(
        checksum::Retirement::testing(),
        Arc::new(midnight_platform::LocalFileSystem::new(None)),
    );
    let mut request =
        ChecksumRequest::new(ChecksumJobId(80), ChecksumGeneration(1), vec![directory]);
    request.recursive = true;
    controller.start(request).unwrap();
    let run = loop {
        if let ChecksumUpdate::Finished(run) = controller.next().await.unwrap().update {
            break run;
        }
    };
    assert_eq!(run.summary.progress.calculated, 1);
    assert_eq!(run.summary.progress.skipped, 1);
    assert_eq!(run.summary.progress.bytes_read, 3);
    assert_eq!(
        run.summary.completion,
        ChecksumCompletion::CompletedWithErrors
    );
    controller.shutdown().await;
}

#[tokio::test]
async fn native_raw_name_can_be_hashed_but_never_lossily_exported() {
    let _quota_fixture = checksum::test_quota_fixture().await;
    let temp = fixture();
    let path = temp.path().join(OsString::from_vec(vec![b'x', 0xff]));
    fs::write(&path, b"abc").unwrap();
    let provider = Arc::new(midnight_platform::LocalFileSystem::new(None));
    let mut controller =
        checksum::Controller::new(checksum::Retirement::testing(), provider.clone());
    controller
        .start(ChecksumRequest::new(
            ChecksumJobId(81),
            ChecksumGeneration(1),
            vec![path.clone()],
        ))
        .unwrap();
    let run = loop {
        if let ChecksumUpdate::Finished(run) = controller.next().await.unwrap().update {
            break run;
        }
    };
    assert_eq!(run.rows[0].path, path);
    assert_eq!(run.rows[0].state, ChecksumRowState::Calculated);
    let request = ChecksumManifestRequest {
        algorithm: ChecksumAlgorithm::Sha256,
        destination: temp.path().join("refused.sha256"),
        acknowledge_partial: None,
    };
    assert_eq!(
        checksum::save_manifest(&run, &request, provider.as_ref(), &Cancellation::default())
            .unwrap_err()
            .kind,
        std::io::ErrorKind::InvalidData
    );
    assert!(!request.destination.exists());
    assert_eq!(fs::read(path).unwrap(), b"abc");
    controller.shutdown().await;
}
