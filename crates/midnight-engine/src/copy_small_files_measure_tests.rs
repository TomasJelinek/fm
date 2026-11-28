//! Measurement, not a check: the per-file cost of a Copy of many small files through the
//! whole application path (the copy actor, admission, the namespace claim, the native
//! filesystem) with the defaults: full SHA-256 verification and preserved times and
//! attributes. Run in a release build:
//! `MIDNIGHT_MEASURE_ROOT=<folder> cargo test --release -p midnight-engine --lib
//! small_files_measure -- --ignored --nocapture --test-threads 1`.
//!
//! The fixture is created once below `MIDNIGHT_MEASURE_ROOT` (else the temporary folder):
//! `small-5000` (5,000 files of 1–16 KiB), `large-500` (500 files of 1 MiB) and
//! `one-1024` (one file of 1 GiB).
//! `MIDNIGHT_MEASURE_RUNS` (default 5) copies of each folder run one after another into
//! an emptied sibling folder of the same volume; each run prints its time, and the
//! median follows.
use super::*;

const SMALL: (&str, usize) = ("small-5000", 5_000);
const LARGE: (&str, usize) = ("large-500", 500);
/// One file of 1 GiB: the per-byte cost without the per-file one.
const ONE: (&str, usize) = ("one-1024", 1);

fn measure_root() -> PathBuf {
    std::env::var_os("MIDNIGHT_MEASURE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("midnight-copy-measure"))
}

/// Bytes of file `index`, of `len` bytes, that differ from file to file.
fn contents(index: usize, len: usize) -> Vec<u8> {
    let mut state = (index as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

/// The source folder of a workload, created once: `count` files, each 1–16 KiB for the
/// small workload and 1 MiB for the large one.
fn source(root: &Path, (name, count): (&str, usize)) -> PathBuf {
    let folder = root.join(name);
    let last = folder.join(format!("file {:05}.bin", count - 1));
    if !last.exists() {
        fs::create_dir_all(&folder).unwrap();
        for index in 0..count {
            let len = if count == SMALL.1 {
                1024 * (1 + index % 16)
            } else if count == ONE.1 {
                1024 * 1024 * 1024
            } else {
                1024 * 1024
            };
            fs::write(
                folder.join(format!("file {index:05}.bin")),
                contents(index, len),
            )
            .unwrap();
        }
    }
    folder
}

fn emptied(folder: &Path) -> PathBuf {
    if folder.exists() {
        fs::remove_dir_all(folder).unwrap();
    }
    fs::create_dir_all(folder).unwrap();
    folder.to_owned()
}

/// One Copy of `source` into `destination` with the application's defaults; returns the
/// elapsed time, the files copied and the progress events received.
async fn copy_once(source: &Path, destination: &Path) -> (Duration, u64, u64) {
    // The application's event channel (32) and automatic disk-I/O limit (4).
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
    h.controller
        .enqueue_metadata(MetadataTransferRequest {
            input: MetadataTransferInput::Paths(request(
                1,
                std::slice::from_ref(&source.to_owned()),
                destination,
            )),
            intent: TransferIntent::Copy,
            metadata: TransferMetadataPolicy::PreserveTimesAndAttributes,
        })
        .await;
    let mut events = 0u64;
    let done = loop {
        let event = tokio::time::timeout(Duration::from_secs(1800), h.events.recv())
            .await
            .expect("an event within 30 minutes")
            .expect("events");
        match event {
            Event::Copy(CopyEvent::Progress(progress)) if progress.state.is_terminal() => {
                break progress;
            }
            Event::Copy(CopyEvent::Progress(_)) => events += 1,
            Event::Copy(CopyEvent::Failure(failure)) => panic!("{failure:?}"),
            Event::Copy(CopyEvent::Conflict(conflict)) => panic!("{conflict:?}"),
            _ => {}
        }
    };
    let elapsed = started.elapsed();
    assert_eq!(
        done.state,
        CopyOperationState::Completed,
        "{:?}",
        done.errors
    );
    assert_eq!(done.metadata_not_preserved, 0);
    h.close().await;
    (elapsed, done.files_completed, events)
}

async fn measure(workload: (&str, usize)) {
    let root = measure_root();
    let source = source(&root, workload);
    let runs: usize = std::env::var("MIDNIGHT_MEASURE_RUNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5);
    let mut times = Vec::new();
    for run in 0..runs {
        let destination = emptied(&root.join(format!("{}-copy", workload.0)));
        let (elapsed, files, events) = copy_once(&source, &destination).await;
        assert_eq!(files, workload.1 as u64);
        let seconds = elapsed.as_secs_f64();
        eprintln!(
            "{} run {}: {seconds:.3} s, {:.0} files/s, {:.0} µs per file, {events} progress events",
            workload.0,
            run + 1,
            files as f64 / seconds,
            seconds * 1e6 / files as f64,
        );
        times.push(seconds);
    }
    times.sort_by(f64::total_cmp);
    let median = times[times.len() / 2];
    eprintln!(
        "{} median of {runs}: {median:.3} s, {:.0} files/s, {:.0} µs per file",
        workload.0,
        workload.1 as f64 / median,
        median * 1e6 / workload.1 as f64,
    );
    fs::remove_dir_all(root.join(format!("{}-copy", workload.0))).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measurement; run explicitly in a release build"]
async fn measure_small_files() {
    measure(SMALL).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measurement; run explicitly in a release build"]
async fn measure_large_files() {
    measure(LARGE).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measurement; run explicitly in a release build"]
async fn measure_one_large_file() {
    measure(ONE).await;
}
