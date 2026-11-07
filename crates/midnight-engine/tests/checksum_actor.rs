use midnight_core::*;
use midnight_engine::Engine;
use midnight_platform::LocalFileSystem;
use std::{fs, sync::Arc, time::Duration};
use tokio::sync::mpsc;

#[cfg(windows)]
#[path = "support/checksum_fixture_windows.rs"]
mod native_fixture;
#[cfg(target_os = "linux")]
#[path = "support/checksum_fixture_linux.rs"]
mod native_fixture;

async fn next(events: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .unwrap()
        .unwrap()
}
async fn completed(events: &mut mpsc::Receiver<Event>, generation: u64) -> Arc<ChecksumRun> {
    loop {
        if let Event::Checksum(event) = next(events).await
            && event.generation == ChecksumGeneration(generation)
            && let ChecksumUpdate::Finished(run) = event.update
        {
            return run;
        }
    }
}
async fn saved(
    events: &mut mpsc::Receiver<Event>,
    id: u64,
) -> Result<ChecksumManifestReceipt, FsError> {
    loop {
        if let Event::ChecksumManifest(event) = next(events).await
            && event.save_id == ChecksumManifestId(id)
        {
            match event.update {
                ChecksumManifestUpdate::Finished(result) => return result,
                ChecksumManifestUpdate::Rejected(error) => return Err(error),
                _ => {}
            }
        }
    }
}
fn save(job: u64, generation: u64, id: u64, destination: std::path::PathBuf) -> Command {
    Command::SaveChecksumManifest {
        save_id: ChecksumManifestId(id),
        job: ChecksumJobId(job),
        generation: ChecksumGeneration(generation),
        request: Box::new(ChecksumManifestRequest {
            algorithm: ChecksumAlgorithm::Sha256,
            destination,
            acknowledge_partial: None,
        }),
    }
}

#[tokio::test]
async fn calculation_retains_backend_results_for_verified_manifest_and_release_rejects_save() {
    let temp = native_fixture::fixture();
    let source = temp.path().join("source 文.txt");
    fs::write(&source, b"abc").unwrap();
    let service = Arc::new(LocalFileSystem::new(None));
    let (send, commands) = mpsc::channel(16);
    let (events, mut receive) = mpsc::channel(16);
    let engine = tokio::spawn(
        Engine::new(service.clone(), events)
            .with_transfers(service)
            .run(commands),
    );
    send.send(Command::StartChecksums(Box::new(ChecksumRequest::new(
        ChecksumJobId(1),
        ChecksumGeneration(1),
        vec![source.clone()],
    ))))
    .await
    .unwrap();
    let run = completed(&mut receive, 1).await;
    assert_eq!(run.rows[0].state, ChecksumRowState::Calculated);
    drop(run);
    send.send(save(1, 1, 1, temp.path().join("result.sha256")))
        .await
        .unwrap();
    assert_eq!(saved(&mut receive, 1).await.unwrap().included, 1);
    assert!(
        fs::read_to_string(temp.path().join("result.sha256"))
            .unwrap()
            .contains(" *source 文.txt\n")
    );
    send.send(Command::ReleaseChecksums {
        job: ChecksumJobId(1),
        generation: ChecksumGeneration(1),
    })
    .await
    .unwrap();
    send.send(save(1, 1, 2, temp.path().join("released.sha256")))
        .await
        .unwrap();
    assert!(saved(&mut receive, 2).await.is_err());
    assert!(!temp.path().join("released.sha256").exists());
    drop(send);
    tokio::time::timeout(Duration::from_secs(10), engine)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn cancelled_calculation_and_stale_generation_have_explicit_events() {
    let temp = native_fixture::fixture();
    let source = temp.path().join("source");
    fs::write(&source, b"abc").unwrap();
    let service = Arc::new(LocalFileSystem::new(None));
    let (send, commands) = mpsc::channel(16);
    let (events, mut receive) = mpsc::channel(16);
    let engine = tokio::spawn(
        Engine::new(service.clone(), events)
            .with_transfers(service)
            .run(commands),
    );
    send.send(Command::StartChecksums(Box::new(ChecksumRequest::new(
        ChecksumJobId(1),
        ChecksumGeneration(1),
        vec![source.clone()],
    ))))
    .await
    .unwrap();
    send.send(Command::CancelChecksums {
        job: ChecksumJobId(1),
        generation: ChecksumGeneration(1),
    })
    .await
    .unwrap();
    let first = completed(&mut receive, 1).await;
    assert_eq!(first.summary.completion, ChecksumCompletion::Cancelled);
    drop(first);
    send.send(Command::StartChecksums(Box::new(ChecksumRequest::new(
        ChecksumJobId(1),
        ChecksumGeneration(2),
        vec![source.clone()],
    ))))
    .await
    .unwrap();
    assert_eq!(
        completed(&mut receive, 2).await.summary.completion,
        ChecksumCompletion::Completed
    );
    send.send(Command::StartChecksums(Box::new(ChecksumRequest::new(
        ChecksumJobId(1),
        ChecksumGeneration(1),
        vec![source],
    ))))
    .await
    .unwrap();
    loop {
        if let Event::ChecksumRejected(event) = next(&mut receive).await {
            assert_eq!(event.generation, ChecksumGeneration(1));
            break;
        }
    }
    drop(send);
    tokio::time::timeout(Duration::from_secs(10), engine)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn unavailable_service_and_monotonic_save_ids_reject_visibly() {
    let temp = native_fixture::fixture();
    let service = Arc::new(LocalFileSystem::new(None));
    let (send, commands) = mpsc::channel(16);
    let (events, mut receive) = mpsc::channel(16);
    let engine = tokio::spawn(Engine::new(service, events).run(commands));
    send.send(Command::StartChecksums(Box::new(ChecksumRequest::new(
        ChecksumJobId(1),
        ChecksumGeneration(1),
        vec![temp.path().join("source")],
    ))))
    .await
    .unwrap();
    assert!(matches!(
        next(&mut receive).await,
        Event::ChecksumRejected(_)
    ));
    for id in [5, 5, 4] {
        send.send(save(1, 1, id, temp.path().join("never.sha256")))
            .await
            .unwrap();
        assert!(saved(&mut receive, id).await.is_err());
    }
    drop(send);
    tokio::time::timeout(Duration::from_secs(10), engine)
        .await
        .unwrap()
        .unwrap();
}
