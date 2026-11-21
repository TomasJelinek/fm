//! Memory-first staging (`staging_memory.rs`) and the per-process spool root.
use super::*;
use std::{
    sync::{
        Barrier,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

const KIB: usize = 1024;
const GIB: u64 = 1 << 30;

fn budget(limit: usize) -> &'static MemoryBudget {
    Box::leak(Box::new(MemoryBudget::new(limit as u64)))
}

/// Staging in a new private root that keeps stages of up to `limit` bytes in memory.
fn open_staging(limit: usize, budget: &'static MemoryBudget) -> (tempfile::TempDir, Staging) {
    let temp = tempfile::tempdir().unwrap();
    let mut staging = Staging::open(&temp.path().join("private")).unwrap();
    staging.set_memory(limit as u64, budget);
    (temp, staging)
}

fn read_op(request: u64, entry: u64) -> Message {
    Message::Read {
        request,
        entry,
        offset: 0,
        length: 1,
    }
}

/// Stage `content` as member `entry` in appends of `chunk` bytes, declaring its length
/// up front when `declared`, then seal it and return the member's lease.
fn stage(
    staging: &mut Staging,
    (request, entry): (u64, u64),
    content: &[u8],
    declared: bool,
    chunk: usize,
) -> io::Result<Lease> {
    let op = read_op(request, entry);
    let Some(Message::StageCreated { stage, .. }) = staging.nested(
        &op,
        &Message::StageBegin {
            request,
            purpose: StagePurpose::Member { entry },
            expected_length: declared.then_some(content.len() as u64),
            max_length: content.len() as u64,
        },
    )?
    else {
        panic!("the member stage was not created")
    };
    let mut offset = 0;
    for bytes in content.chunks(chunk) {
        let append = Message::StageAppend {
            request,
            stage,
            offset,
            bytes: bytes.to_vec(),
        };
        staging.nested(&op, &append)?;
        offset += bytes.len() as u64;
    }
    let seal = Message::StageSeal {
        request,
        stage,
        length: offset,
    };
    staging.nested(&op, &seal)?;
    let ready = Message::ContentReady {
        request,
        entry,
        stage,
        offset: 0,
        length: offset,
    };
    Ok(staging.terminal(&op, &ready)?.unwrap())
}

fn read_all(lease: &Lease) -> Vec<u8> {
    let mut content = Vec::new();
    while (content.len() as u64) < lease.length {
        content.extend(lease.read(content.len() as u64, MAX_READ).unwrap());
    }
    content
}

fn pattern(length: usize, seed: usize) -> Vec<u8> {
    (0..length).map(|index| (index * 31 + seed) as u8).collect()
}

fn no_volume_query(_: &PrivateSpoolDirectory) -> io::Result<SpoolSpace> {
    panic!("a stage kept in memory queried the spool volume")
}

#[test]
fn a_small_member_stays_in_memory_and_creates_no_spool_file() {
    let budget = budget(1024 * KIB);
    let (_temp, mut staging) = open_staging(MEMORY_STAGE_LIMIT as usize, budget);
    staging.space = no_volume_query;
    // Two appends, the first a whole MAX_IO chunk; a later read crosses between them.
    let content = pattern(300 * KIB, 1);
    let lease = stage(&mut staging, (1, 2), &content, true, MAX_READ as usize).unwrap();
    assert_eq!(
        staging.root.created_spools(),
        0,
        "no O_TMPFILE spool file was created for the member"
    );
    assert!(!lease.spool.on_disk());
    assert_eq!(budget.used(), content.len() as u64);
    assert_eq!(read_all(&lease), content);
    assert_eq!(
        lease.read(250 * KIB as u64, 20 * 1024).unwrap(),
        &content[250 * KIB..270 * KIB]
    );
    // A read that outlives its mount keeps the memory until the read ends.
    drop(staging);
    assert_eq!(budget.used(), content.len() as u64);
    drop(lease);
    assert_eq!(budget.used(), 0);
}

#[test]
fn a_large_member_goes_to_a_spool_file_and_a_growing_stream_spills_intact() {
    let budget = budget(1024 * KIB);
    let (_temp, mut staging) = open_staging(64 * KIB, budget);
    // A declared length above the limit gets its spool file before any byte arrives.
    let large = pattern(100 * KIB, 2);
    let lease = stage(&mut staging, (1, 2), &large, true, 16 * KIB).unwrap();
    assert_eq!(staging.root.created_spools(), 1);
    assert!(lease.spool.on_disk());
    assert_eq!(budget.used(), 0);
    assert_eq!(read_all(&lease), large);
    // Without a declared length, the first four appends stay in memory and the fifth
    // moves them to a spool file, which then takes the rest.
    let stream = pattern(100 * KIB, 3);
    let lease = stage(&mut staging, (2, 3), &stream, false, 16 * KIB).unwrap();
    assert_eq!(staging.root.created_spools(), 2);
    assert!(lease.spool.on_disk());
    assert_eq!(budget.used(), 0, "spilling returns the stage's memory");
    assert_eq!(read_all(&lease), stream);
}

thread_local! {
    /// Bytes above the reserve that `small_volume` reports.
    static ABOVE_RESERVE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
/// A 100 GiB volume, whose reserve is 2 GiB, with `ABOVE_RESERVE` more bytes free.
fn small_volume(_: &PrivateSpoolDirectory) -> io::Result<SpoolSpace> {
    Ok(SpoolSpace {
        available: 2 * GIB + ABOVE_RESERVE.get(),
        total: 100 * GIB,
    })
}

#[test]
fn a_spill_counts_the_bytes_it_moves_against_the_volume_reserve() {
    let budget = budget(1024 * KIB);
    let (_temp, mut staging) = open_staging(64 * KIB, budget);
    staging.space = small_volume;
    // The spill would write 64 KiB from memory and the 16 KiB chunk: 80 KiB > 70 KiB.
    ABOVE_RESERVE.set(70 * KIB as u64);
    let error = stage(
        &mut staging,
        (1, 2),
        &pattern(100 * KIB, 4),
        false,
        16 * KIB,
    )
    .err()
    .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    assert_eq!(staging.root.created_spools(), 0, "nothing was written");
    // With room for exactly those bytes, the same stage spills.
    let (_temp, mut staging) = open_staging(64 * KIB, budget);
    staging.space = small_volume;
    ABOVE_RESERVE.set(100 * KIB as u64);
    let stream = pattern(100 * KIB, 5);
    let lease = stage(&mut staging, (1, 2), &stream, false, 16 * KIB).unwrap();
    assert_eq!(read_all(&lease), stream);
}

#[test]
fn mounts_share_one_memory_budget_and_return_it_when_they_close() {
    let budget = budget(128 * KIB);
    let (_first_temp, mut first) = open_staging(MEMORY_STAGE_LIMIT as usize, budget);
    let (_second_temp, mut second) = open_staging(MEMORY_STAGE_LIMIT as usize, budget);
    let kept = stage(&mut first, (1, 2), &pattern(96 * KIB, 6), true, 32 * KIB).unwrap();
    assert_eq!(budget.used(), 96 * KIB as u64);
    // The other mount's first 32 KiB still fit; its next chunk does not, so it spills.
    let spilled_content = pattern(64 * KIB, 7);
    let spilled = stage(&mut second, (1, 2), &spilled_content, true, 32 * KIB).unwrap();
    assert!(spilled.spool.on_disk());
    assert_eq!(second.root.created_spools(), 1);
    assert_eq!(budget.used(), 96 * KIB as u64);
    assert_eq!(read_all(&spilled), spilled_content);
    // Closing the first mount returns its memory, and the next member fits again.
    drop(first);
    drop(kept);
    assert_eq!(budget.used(), 0);
    let fits = stage(&mut second, (2, 3), &pattern(64 * KIB, 8), true, 32 * KIB).unwrap();
    assert!(!fits.spool.on_disk());
    assert_eq!(second.root.created_spools(), 1);
}

#[test]
fn concurrent_mounts_never_hold_more_memory_than_the_budget() {
    const THREADS: usize = 8;
    const MEMBERS: u64 = 6;
    const MEMBER: usize = 40 * KIB;
    let budget = budget(256 * KIB);
    // Every lease stays alive until all threads staged all members, so together they
    // ask for far more than the budget at once.
    let barrier: &'static Barrier = Box::leak(Box::new(Barrier::new(THREADS)));
    let threads: Vec<_> = (0..THREADS)
        .map(|thread| {
            std::thread::spawn(move || {
                let (_temp, mut staging) = open_staging(MEMORY_STAGE_LIMIT as usize, budget);
                let mut leases = Vec::new();
                for member in 0..MEMBERS {
                    let content = pattern(MEMBER, thread * 10 + member as usize);
                    let lease = stage(
                        &mut staging,
                        (member + 1, member + 2),
                        &content,
                        false,
                        8 * KIB,
                    )
                    .unwrap();
                    assert!(budget.used() <= 256 * KIB as u64);
                    assert_eq!(read_all(&lease), content);
                    leases.push(lease);
                }
                barrier.wait();
                (staging.root.created_spools(), leases)
            })
        })
        .collect();
    let results: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    let leases: Vec<&Lease> = results.iter().flat_map(|(_, leases)| leases).collect();
    let in_memory = leases.iter().filter(|lease| !lease.spool.on_disk()).count();
    let spool_files: u64 = results.iter().map(|(created, _)| created).sum();
    assert_eq!(budget.used(), (in_memory * MEMBER) as u64);
    assert!(
        in_memory * MEMBER <= 256 * KIB,
        "{in_memory} members in memory"
    );
    assert_eq!(spool_files as usize, leases.len() - in_memory);
    drop(results);
    assert_eq!(budget.used(), 0);
}

fn ample(_: &PrivateSpoolDirectory) -> io::Result<SpoolSpace> {
    Ok(SpoolSpace {
        available: 1 << 40,
        total: 1 << 40,
    })
}

fn full(_: &PrivateSpoolDirectory) -> io::Result<SpoolSpace> {
    Ok(SpoolSpace {
        available: 0,
        total: 100 * GIB,
    })
}

#[test]
fn the_spool_root_is_chosen_once_per_process_and_again_after_a_failure() {
    let temp = tempfile::tempdir().unwrap();
    let cache: &'static RootCache = Box::leak(Box::new(RootCache::new()));
    let walks = AtomicUsize::new(0);
    let candidates = || {
        walks.fetch_add(1, Ordering::Relaxed);
        vec![SpoolRoot {
            path: temp.path().join("archive-spool"),
            per_process: false,
        }]
    };
    let first = cache.open(candidates, ample).unwrap();
    let second = cache.open(candidates, ample).unwrap();
    assert_eq!(walks.load(Ordering::Relaxed), 1);
    assert!(first.root.is_same(&second.root));
    assert!(
        Arc::ptr_eq(&first.claims, &second.claims),
        "mounts of one root share the space claimed in it"
    );
    assert_eq!(
        first.root.created_spools(),
        1,
        "one test spool checked the root, for both mounts"
    );
    // A spool that cannot be written in the cached root makes the next mount choose again.
    // (Content of undeclared length reaches the append; a declared length would be
    // refused at its StageBegin, which forgets the root as well.)
    let mut failing = cache.open(candidates, full).unwrap();
    failing.set_memory(0, budget(0));
    let op = read_op(1, 2);
    let begin = Message::StageBegin {
        request: 1,
        purpose: StagePurpose::Member { entry: 2 },
        expected_length: None,
        max_length: 4,
    };
    let Some(Message::StageCreated { stage, .. }) = failing.nested(&op, &begin).unwrap() else {
        panic!("the member stage was not created")
    };
    let append = Message::StageAppend {
        request: 1,
        stage,
        offset: 0,
        bytes: b"data".to_vec(),
    };
    let error = failing.nested(&op, &append).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    assert_eq!(walks.load(Ordering::Relaxed), 1);
    let third = cache.open(candidates, ample).unwrap();
    assert_eq!(walks.load(Ordering::Relaxed), 2);
    assert!(!third.root.is_same(&first.root));
    assert_eq!(third.root.created_spools(), 1);
}

#[test]
fn a_member_refused_before_staging_makes_the_next_mount_choose_its_root_again() {
    let temp = tempfile::tempdir().unwrap();
    let cache: &'static RootCache = Box::leak(Box::new(RootCache::new()));
    let walks = AtomicUsize::new(0);
    let candidates = || {
        walks.fetch_add(1, Ordering::Relaxed);
        vec![SpoolRoot {
            path: temp.path().join("archive-spool"),
            per_process: false,
        }]
    };
    let mut refusing = cache.open(candidates, full).unwrap();
    refusing.set_memory(0, budget(0));
    let begin = Message::StageBegin {
        request: 1,
        purpose: StagePurpose::Member { entry: 2 },
        expected_length: Some(4),
        max_length: 4,
    };
    let error = refusing.nested(&read_op(1, 2), &begin).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    assert_eq!(
        refusing.root.created_spools(),
        1,
        "only the root's test spool"
    );
    drop(cache.open(candidates, ample).unwrap());
    assert_eq!(walks.load(Ordering::Relaxed), 2);
}

#[test]
fn a_per_process_spool_root_is_checked_by_every_mount() {
    let temp = tempfile::tempdir().unwrap();
    let cache: &'static RootCache = Box::leak(Box::new(RootCache::new()));
    let candidates = || {
        vec![SpoolRoot {
            path: temp.path().join("archive-spool-9"),
            per_process: true,
        }]
    };
    // A startup check keeps no per-process root and removes it again at once.
    cache.prepare(candidates, ample).unwrap();
    assert!(!temp.path().join("archive-spool-9").exists());
    let first = cache.open(candidates, ample).unwrap();
    let second = cache.open(candidates, ample).unwrap();
    // Each mount checked the root; their shared guard still removes it when both close.
    assert!(!first.root.is_same(&second.root));
    drop((first, second));
    assert!(!temp.path().join("archive-spool-9").exists());
}

#[test]
fn a_root_checked_at_startup_spares_every_mount_the_check() {
    let temp = tempfile::tempdir().unwrap();
    let cache: &'static RootCache = Box::leak(Box::new(RootCache::new()));
    let path = temp.path().join("archive-spool");
    let (checking, check_started) = mpsc::channel();
    let (finish_check, finishing) = mpsc::channel::<()>();
    // A mount that starts while the startup check holds the cache waits for its choice.
    let mut mount = std::thread::scope(|scope| {
        let preparing = scope.spawn(move || {
            let candidates = move || {
                checking.send(()).unwrap();
                finishing.recv().unwrap();
                vec![SpoolRoot {
                    path,
                    per_process: false,
                }]
            };
            cache.prepare(candidates, ample)
        });
        // A check that never walks the candidates fails here instead of hanging.
        check_started
            .recv_timeout(Duration::from_secs(10))
            .expect("the startup check chose no root");
        let mounting =
            scope.spawn(|| cache.open(|| panic!("the mount checked a root"), no_volume_query));
        finish_check.send(()).unwrap();
        preparing.join().unwrap().unwrap();
        mounting.join().unwrap().unwrap()
    });
    assert_eq!(
        mount.root.created_spools(),
        1,
        "only the startup check created a test spool"
    );
    // The first mount then creates no file and queries no volume for a small member.
    mount.set_memory(MEMORY_STAGE_LIMIT, budget(1024 * KIB));
    let content = pattern(64 * KIB, 4);
    let lease = stage(&mut mount, (1, 2), &content, true, 16 * KIB).unwrap();
    assert_eq!(read_all(&lease), content);
    assert_eq!(mount.root.created_spools(), 1);
    // A kept root is not checked again.
    cache
        .prepare(|| panic!("a kept root was checked again"), no_volume_query)
        .unwrap();
    assert_eq!(mount.root.created_spools(), 1);
}
