//! Manual timings of transfers through the real queue and the native Linux adapter: a
//! same-volume Move of one file (a rename) and a Copy of a folder of many small files.
//! Storage load from other processes dominates the spread, so run them serially and
//! repeat them:
//! `cargo test -p midnight-engine --lib copy_operations::tests::timing -- --ignored --nocapture --test-threads=1`
//! `MIDNIGHT_TIMING_MOVES` (default 30) and `MIDNIGHT_TIMING_FILES` (default 3000) set
//! the counts.
use super::*;
use std::time::Instant;

fn setting(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// User and system CPU time of the whole process, all threads included (procfs).
fn cpu() -> Duration {
    let stat = fs::read_to_string("/proc/self/stat").unwrap();
    // After the parenthesized command name: utime and stime, fields 14 and 15.
    let fields: Vec<&str> = stat[stat.rfind(')').unwrap() + 2..].split(' ').collect();
    let ticks = fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap();
    // USER_HZ is 100 on Linux.
    Duration::from_millis(ticks * 10)
}

fn quantile(sorted: &[Duration], q: f64) -> Duration {
    sorted[((sorted.len() - 1) as f64 * q).round() as usize]
}

/// Wait for the job's terminal progress, recording when each state, phase and wait
/// reason was first observed.
async fn finish(
    h: &mut Harness,
    id: u64,
    started: Instant,
    steps: &mut Vec<(Duration, String)>,
) -> Arc<CopyProgress> {
    let deadline = started + Duration::from_secs(900);
    let mut last = String::new();
    loop {
        assert!(Instant::now() < deadline, "job {id} did not finish");
        match h.next().await {
            CopyEvent::Progress(progress) if progress.id == OperationId(id) => {
                let label = format!(
                    "{:?}/{:?}{}",
                    progress.state,
                    progress.phase,
                    progress
                        .wait_reason
                        .map_or(String::new(), |reason| format!(" waiting {reason:?}"))
                );
                if label != last {
                    steps.push((started.elapsed(), label.clone()));
                    last = label;
                }
                if progress.state.is_terminal() {
                    return progress;
                }
            }
            CopyEvent::Conflict(conflict) => panic!("unexpected conflict: {conflict:?}"),
            CopyEvent::Failure(failure) => panic!("unexpected failure: {failure:?}"),
            _ => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "manual timing of same-volume single-file Moves"]
async fn native_move_of_one_file() {
    let moves = setting("MIDNIGHT_TIMING_MOVES", 30);
    let temp = fixture();
    let left = temp.path().join("left");
    let right = temp.path().join("right");
    fs::create_dir(&left).unwrap();
    fs::create_dir(&right).unwrap();
    let mut h = Harness::native();
    let mut elapsed = Vec::new();
    let mut by_state = HashMap::<String, Duration>::new();
    let used = cpu();
    for index in 0..moves {
        let name = format!("file-{index:03}.txt");
        fs::write(left.join(&name), b"moved by a native rename").unwrap();
        let id = index as u64 + 1;
        let started = Instant::now();
        h.controller
            .enqueue_move(request(id, &[left.join(&name)], &right))
            .await;
        let mut steps = Vec::new();
        let progress = finish(&mut h, id, started, &mut steps).await;
        elapsed.push(started.elapsed());
        assert_eq!(
            progress.state,
            CopyOperationState::Completed,
            "{progress:?}"
        );
        assert_eq!(progress.native_entries_moved, 1, "{progress:?}");
        assert!(right.join(&name).exists() && !left.join(&name).exists());
        for pair in steps.windows(2) {
            *by_state.entry(pair[0].1.clone()).or_default() += pair[1].0 - pair[0].0;
        }
    }
    let used = cpu() - used;
    elapsed.sort();
    println!(
        "native Move of one file, {moves} runs: median {:?}, p90 {:?}, max {:?}, min {:?}; \
         process CPU {:?} per Move",
        quantile(&elapsed, 0.5),
        quantile(&elapsed, 0.9),
        elapsed[elapsed.len() - 1],
        elapsed[0],
        used / moves as u32
    );
    let mut states: Vec<_> = by_state.into_iter().collect();
    states.sort();
    for (state, total) in states {
        println!("  {state}: {:?} per Move", total / moves as u32);
    }
    h.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "manual timing of a Copy of many small files"]
async fn copy_of_many_small_files() {
    let files = setting("MIDNIGHT_TIMING_FILES", 3000);
    let temp = fixture();
    let tree = temp.path().join("many");
    let destination = temp.path().join("target");
    fs::create_dir(&tree).unwrap();
    fs::create_dir(&destination).unwrap();
    for index in 0..files {
        // 125-byte files, as in the acceptance fixture's many/ folder.
        fs::write(
            tree.join(format!("item-{index:05}.dat")),
            [index as u8; 125],
        )
        .unwrap();
    }
    let mut h = Harness::native();
    // Phase occupancy: the job's live status sampled every millisecond.
    let stop = Arc::new(AtomicBool::new(false));
    let sampler = {
        let registry = h.controller.registry.clone();
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut counts = HashMap::<String, u64>::new();
            while !stop.load(Ordering::Relaxed) {
                let status = registry
                    .lock()
                    .unwrap()
                    .get(&OperationId(1))
                    .map(|job| job.status.clone());
                if let Some(status) = status {
                    let progress = status.lock().unwrap();
                    *counts
                        .entry(format!("{:?}/{:?}", progress.state, progress.phase))
                        .or_default() += 1;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            counts
        })
    };
    let used = cpu();
    let started = Instant::now();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&tree), &destination))
        .await;
    let mut steps = Vec::new();
    let progress = finish(&mut h, 1, started, &mut steps).await;
    let took = started.elapsed();
    let used = cpu() - used;
    stop.store(true, Ordering::Relaxed);
    let counts = sampler.join().unwrap();
    assert_eq!(
        progress.state,
        CopyOperationState::Completed,
        "{progress:?}"
    );
    assert_eq!(progress.files_completed, files as u64, "{progress:?}");
    println!(
        "Copy of {files} files of 125 bytes: {took:?}, {:?} per file; process CPU {used:?}, \
         {:?} per file",
        took / files as u32,
        used / files as u32
    );
    let samples: u64 = counts.values().sum();
    let mut phases: Vec<_> = counts.into_iter().collect();
    phases.sort_by_key(|phase| std::cmp::Reverse(phase.1));
    for (phase, count) in phases {
        println!(
            "  {phase}: {:.1}% of samples",
            count as f64 * 100.0 / samples.max(1) as f64
        );
    }
    h.close().await;
}
