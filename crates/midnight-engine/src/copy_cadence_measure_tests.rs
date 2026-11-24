//! Measurement, not a check: the progress heartbeat's cost through the copy actor
//! (docs/verification/2026-11-24-progress-cadence.md). Run in a release build:
//! `MIDNIGHT_MEASURE_SOURCE=<folder> MIDNIGHT_MEASURE_DESTINATION=<empty folder>
//! [MIDNIGHT_MEASURE_JOBS=2] cargo test --release -p midnight-engine --lib
//! cadence_measure::measure_progress_cadence -- --ignored --nocapture`.
use super::*;

/// Copies `MIDNIGHT_MEASURE_SOURCE` into `MIDNIGHT_MEASURE_DESTINATION` once per job
/// (`MIDNIGHT_MEASURE_JOBS`, 1 or 2, run at once into their own folders) with the
/// application's event channel (32), draining events as the UI does, and reports the time,
/// the throughput and the progress events each job published per second.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measurement; run explicitly in a release build"]
async fn measure_progress_cadence() {
    let source = PathBuf::from(std::env::var_os("MIDNIGHT_MEASURE_SOURCE").expect("source"));
    let destination =
        PathBuf::from(std::env::var_os("MIDNIGHT_MEASURE_DESTINATION").expect("destination"));
    let jobs: u64 = std::env::var("MIDNIGHT_MEASURE_JOBS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    // The application's channel (32) and its automatic disk-I/O limit (4).
    let mut h = Harness::configured_with_events(
        Some(
            crate::transfer_composition::QualifiedTransferInstallation::audited(Arc::new(
                midnight_platform::LocalFileSystem::new(None),
            )),
        ),
        4,
        32,
    );
    let started = std::time::Instant::now();
    for id in 1..=jobs {
        let target = destination.join(format!("job-{id}"));
        fs::create_dir_all(&target).unwrap();
        h.controller
            .enqueue(request(id, std::slice::from_ref(&source), &target))
            .await;
    }
    let mut events = std::collections::HashMap::<OperationId, u64>::new();
    let mut done = Vec::new();
    while done.len() < jobs as usize {
        let event = tokio::time::timeout(Duration::from_secs(1800), h.events.recv())
            .await
            .expect("an event within 30 minutes")
            .expect("events");
        match event {
            Event::Copy(CopyEvent::Progress(progress)) if progress.state.is_terminal() => {
                done.push(progress);
            }
            Event::Copy(CopyEvent::Progress(progress)) => {
                *events.entry(progress.id).or_default() += 1;
            }
            Event::Copy(CopyEvent::Failure(failure)) => panic!("{failure:?}"),
            Event::Copy(CopyEvent::Conflict(conflict)) => panic!("{conflict:?}"),
            _ => {}
        }
    }
    let seconds = started.elapsed().as_secs_f64();
    let files: u64 = done.iter().map(|progress| progress.files_completed).sum();
    let bytes: u64 = done.iter().map(|progress| progress.completed_bytes).sum();
    let published: u64 = events.values().sum();
    for progress in &done {
        assert_eq!(
            progress.state,
            CopyOperationState::Completed,
            "{:?}",
            progress.errors
        );
    }
    eprintln!(
        "MEASURE jobs={jobs} seconds={seconds:.2} files={files} bytes={bytes} mib_per_s={:.1} files_per_s={:.0} events_per_job_per_s={:.2}",
        bytes as f64 / 1_048_576.0 / seconds,
        files as f64 / seconds,
        published as f64 / jobs as f64 / seconds,
    );
    h.close().await;
}
