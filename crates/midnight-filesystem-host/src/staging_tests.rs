use super::*;

fn staging() -> (tempfile::TempDir, Staging) {
    let temp = tempfile::tempdir().unwrap();
    let staging = Staging::open(&temp.path().join("private")).unwrap();
    (temp, staging)
}
fn read(request: u64, entry: u64) -> Message {
    Message::Read {
        request,
        entry,
        offset: 0,
        length: 4,
    }
}
fn begin(request: u64, entry: u64) -> Message {
    Message::StageBegin {
        request,
        purpose: StagePurpose::Member { entry },
        expected_length: Some(4),
        max_length: 4,
    }
}
fn append(request: u64, stage: u64) -> Message {
    Message::StageAppend {
        request,
        stage,
        offset: 0,
        bytes: b"test".to_vec(),
    }
}
fn ready(request: u64, entry: u64, stage: u64) -> Message {
    Message::ContentReady {
        request,
        entry,
        stage,
        offset: 0,
        length: 4,
    }
}
fn seal(staging: &mut Staging, request: u64, entry: u64) -> u64 {
    let op = read(request, entry);
    let Some(Message::StageCreated { stage, .. }) =
        staging.nested(&op, &begin(request, entry)).unwrap()
    else {
        panic!()
    };
    staging.nested(&op, &append(request, stage)).unwrap();
    staging
        .nested(
            &op,
            &Message::StageSeal {
                request,
                stage,
                length: 4,
            },
        )
        .unwrap();
    stage
}
fn kept(path: &Path) -> SpoolRoot {
    SpoolRoot {
        path: path.to_owned(),
        per_process: false,
    }
}
fn per_process(path: &Path) -> SpoolRoot {
    SpoolRoot {
        path: path.to_owned(),
        per_process: true,
    }
}
#[test]
fn an_unusable_spool_root_yields_to_the_next_private_candidate() {
    let temp = tempfile::tempdir().unwrap();
    // A file where a directory is required can never become a private root.
    let occupied = temp.path().join("occupied");
    std::fs::write(&occupied, b"not a directory").unwrap();
    let private = temp.path().join("private");
    let mut staging = Staging::open_first([kept(&occupied), kept(&private)], ample_volume).unwrap();
    assert!(private.is_dir());
    let op = read(1, 2);
    staging.nested(&op, &begin(1, 2)).unwrap();
    let error = Staging::open_first([kept(&occupied)], ample_volume)
        .err()
        .unwrap();
    assert!(error.to_string().contains("occupied"), "{error}");
    assert!(Staging::open_first(Vec::new(), ample_volume).is_err());
    drop(staging);
    assert!(private.is_dir(), "a per-user root is kept for later runs");
}

#[test]
fn one_spool_root_candidate_belongs_to_this_process_alone() {
    let roots = root::candidates();
    let suffix = format!("-archive-spool-{}", std::process::id());
    let mut processes = roots
        .iter()
        .enumerate()
        .filter(|(_, root)| root.per_process);
    let (Some((position, own)), None) = (processes.next(), processes.next()) else {
        panic!("not exactly one per-process root: {roots:?}")
    };
    let name = own.path.file_name().unwrap().to_string_lossy();
    assert!(name.ends_with(&suffix), "{roots:?}");
    // It only replaces a per-user root that this session cannot use.
    assert!(position > 0, "{roots:?}");
}

thread_local! {
    /// Free space above the reserve that `volumes_in_turn` reports, one entry per
    /// query; `None` fails that query.
    static VOLUMES: std::cell::RefCell<VecDeque<Option<u64>>> =
        const { std::cell::RefCell::new(VecDeque::new()) };
}
/// 100 GiB volumes, whose reserve is 2 GiB, answering from `VOLUMES` in turn.
fn volumes_in_turn(_: &PrivateSpoolDirectory) -> io::Result<SpoolSpace> {
    let above_reserve = VOLUMES
        .with_borrow_mut(VecDeque::pop_front)
        .flatten()
        .ok_or_else(|| io::Error::other("the fake volume query failed"))?;
    Ok(SpoolSpace {
        available: 2 * GIB + above_reserve,
        total: 100 * GIB,
    })
}

#[test]
fn a_spool_root_with_room_above_its_reserve_is_preferred_to_an_earlier_full_one() {
    let temp = tempfile::tempdir().unwrap();
    let full = temp.path().join("full");
    let unknown = temp.path().join("unknown");
    let roomy = temp.path().join("roomy-7");
    // The first volume lacks one byte of a whole check interval above its reserve and
    // the second cannot be queried: the third, with a whole interval, is used.
    VOLUMES.set(VecDeque::from([
        Some(SPACE_CHECK_INTERVAL - 1),
        None,
        Some(SPACE_CHECK_INTERVAL),
    ]));
    let candidates = [kept(&full), kept(&unknown), per_process(&roomy)];
    let staging = Staging::open_first(candidates, volumes_in_turn).unwrap();
    assert!(
        VOLUMES.with_borrow(VecDeque::is_empty),
        "one query per root"
    );
    assert!(full.is_dir() && unknown.is_dir());
    // The chosen per-process root lives as long as its staging.
    assert!(roomy.is_dir());
    drop(staging);
    assert!(!roomy.exists());
    // Without room on any volume the first usable root serves anyway: the reserve then
    // refuses only members that do not fit.
    VOLUMES.set(VecDeque::from([Some(0), Some(0), Some(4)]));
    let mut staging =
        Staging::open_first([kept(&full), per_process(&roomy)], volumes_in_turn).unwrap();
    assert!(!roomy.exists(), "the unused per-process root was removed");
    let lease = stage_member(&mut staging, 1, 2, b"test");
    assert_eq!(lease.read(0, 4).unwrap(), b"test");
}

#[test]
fn mounts_share_a_per_process_root_until_its_last_spool_closes() {
    let temp = tempfile::tempdir().unwrap();
    let occupied = temp.path().join("occupied");
    std::fs::write(&occupied, b"not a directory").unwrap();
    let path = temp.path().join("archive-spool-7");
    let roots = || [kept(&occupied), per_process(&path)];
    let mut first = Staging::open_first(roots(), ample_volume).unwrap();
    let mut second = Staging::open_first(roots(), ample_volume).unwrap();
    let lease = stage_member(&mut first, 1, 2, b"read");
    drop(first);
    // The other mount still stages members in the root.
    assert!(path.is_dir());
    stage_member(&mut second, 1, 2, b"next");
    drop(second);
    // A read that outlives its mount keeps the root until the read ends.
    assert!(path.is_dir());
    assert_eq!(lease.read(0, 4).unwrap(), b"read");
    drop(lease);
    assert!(
        !path.exists(),
        "the last spool removes the per-process root"
    );
    // A later mount of this process creates it again.
    drop(Staging::open_first(roots(), ample_volume).unwrap());
    assert!(!path.exists());
}

const GIB: u64 = 1 << 30;
thread_local! {
    /// Free space above the reserve that `fake_volume` reports, and its query count.
    static ABOVE_RESERVE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static QUERIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
/// A 100 GiB volume, whose reserve is 2 GiB, with `ABOVE_RESERVE` more bytes free.
fn fake_volume(_: &PrivateSpoolDirectory) -> io::Result<SpoolSpace> {
    QUERIES.set(QUERIES.get() + 1);
    Ok(SpoolSpace {
        available: 2 * GIB + ABOVE_RESERVE.get(),
        total: 100 * GIB,
    })
}
fn staging_on_fake_volume(above_reserve: u64) -> (tempfile::TempDir, Staging) {
    let (temp, mut staging) = staging();
    staging.space = fake_volume;
    // These tests cover the spool file's volume checks, so no stage stays in memory.
    staging.spool_files_only();
    ABOVE_RESERVE.set(above_reserve);
    QUERIES.set(0);
    (temp, staging)
}
fn pending_length(staging: &Staging) -> u64 {
    staging.pending.as_ref().unwrap().1.spool.length()
}

/// A member stage whose length the worker does not declare: checked as it is appended.
fn undeclared(request: u64, entry: u64) -> Message {
    Message::StageBegin {
        request,
        purpose: StagePurpose::Member { entry },
        expected_length: None,
        max_length: 4,
    }
}

#[test]
fn staging_stops_before_the_spool_volume_reserve_and_writes_nothing() {
    // Three bytes above the reserve cannot take a four-byte chunk of content whose
    // length was not declared up front.
    let (_temp, mut staging) = staging_on_fake_volume(3);
    let op = read(1, 2);
    staging.nested(&op, &undeclared(1, 2)).unwrap();
    let error = staging.nested(&op, &append(1, 1)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    assert!(error.to_string().contains("free-space reserve"), "{error}");
    assert_eq!(pending_length(&staging), 0);
    assert_eq!(QUERIES.get(), 1);
    // The volume may fill exactly up to its reserve.
    let (_temp, mut staging) = staging_on_fake_volume(4);
    staging.nested(&op, &undeclared(1, 2)).unwrap();
    staging.nested(&op, &append(1, 1)).unwrap();
    assert_eq!(pending_length(&staging), 4);
}

#[test]
fn a_member_that_does_not_fit_above_the_reserve_is_refused_before_anything_is_written() {
    // A declared four-byte member on a volume with three bytes above its reserve is
    // refused at its StageBegin, saying what it needs and what is free there, before a
    // spool file exists or the stage counts a byte.
    let (_temp, mut staging) = staging_on_fake_volume(3);
    let op = read(1, 2);
    let error = staging.nested(&op, &begin(1, 2)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    let message = error.to_string();
    assert!(
        message.contains("needs 4 bytes of archive spool space")
            && message.contains("only 3 bytes free above its reserve (the larger of 1 GiB"),
        "{message}"
    );
    assert!(staging.pending.is_none());
    assert_eq!(staging.root.created_spools(), 0);
    assert_eq!(staging.claims.claimed(), 0);
    assert_eq!(QUERIES.get(), 1);
    // A member that fits exactly is admitted; that one query also covers its appends,
    // and the sealed stage keeps no claim.
    let (_temp, mut staging) = staging_on_fake_volume(4);
    assert_eq!(
        stage_member(&mut staging, 1, 2, b"test")
            .read(0, 4)
            .unwrap(),
        b"test"
    );
    assert_eq!(QUERIES.get(), 1);
    assert_eq!(staging.claims.claimed(), 0);
}

#[test]
fn space_claimed_by_a_stage_in_progress_is_not_promised_to_another() {
    // Two mounts that stage in one spool root share its claims, as mounts of the
    // process's cached root do. Twelve bytes are free above the reserve.
    let (_temp, mut first) = staging_on_fake_volume(12);
    let mut second = Staging::in_directory(first.root.clone(), first.claims.clone(), fake_volume);
    second.spool_files_only();
    let eight = |request, entry| Message::StageBegin {
        request,
        purpose: StagePurpose::Member { entry },
        expected_length: Some(8),
        max_length: 8,
    };
    let op = read(1, 2);
    let Some(Message::StageCreated { stage, .. }) = first.nested(&op, &eight(1, 2)).unwrap() else {
        panic!("the first member stage was not created")
    };
    assert_eq!(first.claims.claimed(), 8);
    // Only four of the twelve bytes are not promised to the first stage.
    let other = read(1, 3);
    let error = second.nested(&other, &eight(1, 3)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    assert!(
        error
            .to_string()
            .contains("8 bytes of which other archive reads in progress still need"),
        "{error}"
    );
    // Appended bytes count in the volume's free space instead (this fake volume's free
    // space stays the same, so four written bytes leave room for eight more).
    first.nested(&op, &append(1, stage)).unwrap();
    assert_eq!(first.claims.claimed(), 4);
    second.nested(&other, &eight(1, 3)).unwrap();
    assert_eq!(first.claims.claimed(), 12);
    // An unannounced stage may use only what nobody claimed: nothing here.
    let mut third = Staging::in_directory(first.root.clone(), first.claims.clone(), fake_volume);
    third.spool_files_only();
    let unclaimed = read(1, 4);
    third.nested(&unclaimed, &undeclared(1, 4)).unwrap();
    let error = third.nested(&unclaimed, &append(1, 1)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    // Abandoned and failed stages give their claims back.
    first.abandon_member(1).unwrap();
    assert_eq!(first.claims.claimed(), 8);
    drop(second);
    assert_eq!(first.claims.claimed(), 0);
}

#[test]
fn an_admission_made_before_its_stage_begins_is_used_once_by_that_request() {
    let (_temp, mut staging) = staging_on_fake_volume(4);
    let op = read(1, 2);
    staging.admit_member(&begin(1, 2)).unwrap();
    assert_eq!((QUERIES.get(), staging.claims.claimed()), (1, 4));
    // The StageBegin takes the admission instead of querying the volume again.
    staging.nested(&op, &begin(1, 2)).unwrap();
    assert_eq!((QUERIES.get(), staging.claims.claimed()), (1, 4));
    staging.abandon_member(1).unwrap();
    assert_eq!(staging.claims.claimed(), 0);
    // An admission that its request did not use is returned before the next stage
    // claims: the four bytes are free again for request 3.
    staging.admit_member(&begin(2, 3)).unwrap();
    assert_eq!(staging.claims.claimed(), 4);
    staging.nested(&read(3, 3), &begin(3, 3)).unwrap();
    assert_eq!((QUERIES.get(), staging.claims.claimed()), (3, 4));
    staging.abandon_member(3).unwrap();
    assert_eq!(staging.claims.claimed(), 0);
    // A member that does not fit is refused by the admission itself.
    let error = staging
        .admit_member(&Message::StageBegin {
            request: 4,
            purpose: StagePurpose::Member { entry: 2 },
            expected_length: Some(5),
            max_length: 5,
        })
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    assert!(staging.admitted.is_none());
}

/// A member stage that declares `length` bytes.
fn member(request: u64, entry: u64, length: u64) -> Message {
    Message::StageBegin {
        request,
        purpose: StagePurpose::Member { entry },
        expected_length: Some(length),
        max_length: length,
    }
}
/// A volume that cannot be queried: a failure of the spool root.
fn failing_volume(_: &PrivateSpoolDirectory) -> io::Result<SpoolSpace> {
    Err(io::Error::other("the fake volume query failed"))
}

#[test]
fn a_member_refused_for_space_keeps_the_process_spool_root() {
    static CACHE: RootCache = RootCache::new();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("kept");
    // Four bytes above the reserve take no eight-byte member, declared or appended.
    ABOVE_RESERVE.set(4);
    let mut first = CACHE.open(|| vec![kept(&path)], fake_volume).unwrap();
    first.spool_files_only();
    let error = first.nested(&read(1, 2), &member(1, 2, 8)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    let error = first.admit_member(&member(2, 3, 8)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    let undeclared = Message::StageBegin {
        request: 3,
        purpose: StagePurpose::Member { entry: 4 },
        expected_length: None,
        max_length: 8,
    };
    first.nested(&read(3, 4), &undeclared).unwrap();
    let chunk = Message::StageAppend {
        request: 3,
        stage: 1,
        offset: 0,
        bytes: vec![7; 8],
    };
    let error = first.nested(&read(3, 4), &chunk).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    // The root works: the next mount stages in it without walking the candidates or
    // creating a test spool there again.
    let created = first.root.created_spools();
    let second = CACHE
        .open(
            || panic!("a refused member made the process choose its root again"),
            fake_volume,
        )
        .unwrap();
    assert!(second.root.is_same(&first.root));
    assert_eq!(second.root.created_spools(), created);
}

#[test]
fn a_root_chosen_again_or_another_root_on_the_volume_counts_the_earlier_claims() {
    static CACHE: RootCache = RootCache::new();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("kept");
    let occupied = temp.path().join("occupied");
    std::fs::write(&occupied, b"not a directory").unwrap();
    let own = temp.path().join("archive-spool-7");
    // Twelve bytes above the reserve, of which the first mount's stage claims eight.
    ABOVE_RESERVE.set(12);
    let mut first = CACHE.open(|| vec![kept(&path)], fake_volume).unwrap();
    first.spool_files_only();
    first.nested(&read(1, 2), &member(1, 2, 8)).unwrap();
    // A spool failure in the root (here its volume cannot be queried) makes the next
    // mount choose its root again: a new capability for the same directory.
    first.set_space(failing_volume);
    assert!(first.admit_member(&member(2, 3, 1)).is_err());
    let mut again = CACHE.open(|| vec![kept(&path)], fake_volume).unwrap();
    assert!(!again.root.is_same(&first.root));
    again.spool_files_only();
    // Only four bytes are not promised to the first mount's stage.
    let error = again.nested(&read(1, 2), &member(1, 2, 8)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    assert!(error.to_string().contains("8 bytes of which"), "{error}");
    again.nested(&read(1, 2), &member(1, 2, 2)).unwrap();
    // Another root on the same volume, here a per-process one, counts them too.
    again.set_space(failing_volume);
    assert!(again.admit_member(&member(2, 3, 1)).is_err());
    let mut other = CACHE
        .open(|| vec![kept(&occupied), per_process(&own)], fake_volume)
        .unwrap();
    assert!(own.is_dir());
    other.spool_files_only();
    let error = other.nested(&read(1, 2), &member(1, 2, 4)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    other.nested(&read(1, 2), &member(1, 2, 2)).unwrap();
    assert_eq!(first.claimed(), 12);
    // Ended stages give their claims back to every root on the volume.
    drop((first, again));
    assert_eq!(other.claimed(), 2);
}

#[test]
fn mounts_in_per_process_roots_count_each_others_claims() {
    static CACHE: RootCache = RootCache::new();
    let temp = tempfile::tempdir().unwrap();
    let occupied = temp.path().join("occupied");
    std::fs::write(&occupied, b"not a directory").unwrap();
    let path = temp.path().join("archive-spool-7");
    let roots = || vec![kept(&occupied), per_process(&path)];
    ABOVE_RESERVE.set(12);
    // A per-process root is never kept, so each mount chooses it on its own.
    let mut first = CACHE.open(roots, fake_volume).unwrap();
    let mut second = CACHE.open(roots, fake_volume).unwrap();
    assert!(!first.root.is_same(&second.root));
    first.spool_files_only();
    second.spool_files_only();
    first.nested(&read(1, 2), &member(1, 2, 8)).unwrap();
    let error = second.nested(&read(1, 3), &member(1, 3, 8)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    second.nested(&read(1, 3), &member(1, 3, 4)).unwrap();
    assert_eq!(first.claimed(), 12);
}

#[test]
fn released_claims_return_to_the_volume_and_the_mount_claims_no_more() {
    let (_temp, mut staging) = staging_on_fake_volume(12);
    let op = read(1, 2);
    let Some(Message::StageCreated { stage, .. }) = staging.nested(&op, &member(1, 2, 8)).unwrap()
    else {
        panic!("the member stage was not created")
    };
    staging.admit_member(&member(2, 3, 4)).unwrap();
    assert_eq!(staging.claimed(), 12);
    staging.release_claims();
    assert_eq!(staging.claimed(), 0);
    // The pending stage stays, and checks the volume before it writes again.
    let queries = QUERIES.get();
    let chunk = Message::StageAppend {
        request: 1,
        stage,
        offset: 0,
        bytes: vec![7; 8],
    };
    staging.nested(&op, &chunk).unwrap();
    assert_eq!((QUERIES.get(), pending_length(&staging)), (queries + 1, 8));
    // No member claims space again.
    assert!(staging.admit_member(&member(3, 4, 4)).is_err());
    assert_eq!(staging.claimed(), 0);
}

#[test]
fn spool_space_amounts_are_named_in_binary_units() {
    assert_eq!(space::amount(1023), "1023 bytes");
    assert_eq!(space::amount(1536), "1.5 KiB");
    assert_eq!(space::amount(20 * GIB + GIB / 2), "20.5 GiB");
    assert_eq!(space::amount(u64::MAX), "16384.0 PiB");
}

#[test]
fn the_spool_volume_is_queried_on_each_stage_and_once_per_budget_not_per_append() {
    // Twelve bytes above the reserve: one query admits three four-byte appends.
    let (_temp, mut staging) = staging_on_fake_volume(12);
    let op = read(1, 2);
    let Some(Message::StageCreated { stage, .. }) = staging
        .nested(
            &op,
            &Message::StageBegin {
                request: 1,
                purpose: StagePurpose::Member { entry: 2 },
                expected_length: None,
                max_length: 20,
            },
        )
        .unwrap()
    else {
        panic!("member stage was not created")
    };
    let mut queries = Vec::new();
    for offset in (0..20).step_by(4) {
        let chunk = Message::StageAppend {
            request: 1,
            stage,
            offset,
            bytes: b"data".to_vec(),
        };
        staging.nested(&op, &chunk).unwrap();
        queries.push(QUERIES.get());
    }
    assert_eq!(queries, [1, 1, 1, 2, 2]);
    // The next stage queries on its first append again.
    let seal = Message::StageSeal {
        request: 1,
        stage,
        length: 20,
    };
    staging.nested(&op, &seal).unwrap();
    let reply = Message::ContentReady {
        request: 1,
        entry: 2,
        stage,
        offset: 0,
        length: 20,
    };
    staging.terminal(&op, &reply).unwrap();
    stage_member(&mut staging, 2, 3, b"next");
    assert_eq!(QUERIES.get(), 3);
}

#[test]
fn ample_space_is_queried_once_per_check_interval_and_a_full_volume_admits_nothing() {
    let space = |available| SpoolSpace {
        available,
        total: 100 * GIB,
    };
    assert_eq!(space_budget(space(50 * GIB)), SPACE_CHECK_INTERVAL);
    assert_eq!(space_budget(space(2 * GIB + 5)), 5);
    assert_eq!(space_budget(space(GIB)), 0);
    // A small volume keeps at least 1 GiB.
    let small = SpoolSpace {
        available: GIB + 1,
        total: 10 * GIB,
    };
    assert_eq!(space_budget(small), 1);
}

#[test]
fn member_is_unreadable_until_sealed_and_terminal_validates_identity_and_range() {
    let (_temp, mut staging) = staging();
    let op = read(1, 2);
    staging.nested(&op, &begin(1, 2)).unwrap();
    assert!(staging.terminal(&op, &ready(1, 2, 1)).is_err());
    assert!(
        staging
            .nested(
                &op,
                &Message::SpoolRead {
                    request: 1,
                    stage: 1,
                    offset: 0,
                    length: 1
                }
            )
            .is_err()
    );
    staging.nested(&op, &append(1, 1)).unwrap();
    staging
        .nested(
            &op,
            &Message::StageSeal {
                request: 1,
                stage: 1,
                length: 4,
            },
        )
        .unwrap();
    assert!(staging.terminal(&op, &ready(1, 3, 1)).is_err());
    assert!(
        staging
            .terminal(
                &op,
                &Message::ContentReady {
                    request: 1,
                    entry: 2,
                    stage: 1,
                    offset: u64::MAX,
                    length: 4
                }
            )
            .is_err()
    );
    let lease = staging.terminal(&op, &ready(1, 2, 1)).unwrap().unwrap();
    assert_eq!(lease.read(1, 4).unwrap(), b"est");
    assert!(staging.terminal(&op, &ready(1, 2, 1)).is_err());
    assert_eq!(staging.cached(2).unwrap().read(0, 4).unwrap(), b"test");
}
#[test]
fn cross_request_wrong_direction_offsets_and_lengths_fail_closed() {
    let (_temp, mut staging) = staging();
    let op = read(1, 2);
    assert!(staging.nested(&op, &begin(2, 2)).is_err());
    assert!(staging.nested(&op, &begin(1, 3)).is_err());
    staging.nested(&op, &begin(1, 2)).unwrap();
    for message in [
        begin(1, 2),
        append(1, 99),
        append(2, 1),
        Message::StageCreated {
            request: 1,
            stage: 1,
        },
        Message::StageAppend {
            request: 1,
            stage: 1,
            offset: 1,
            bytes: vec![0],
        },
        Message::StageAppend {
            request: 1,
            stage: 1,
            offset: u64::MAX,
            bytes: vec![0],
        },
        Message::StageAppend {
            request: 1,
            stage: 1,
            offset: 0,
            bytes: vec![0; 5],
        },
        Message::StageSeal {
            request: 1,
            stage: 1,
            length: 0,
        },
    ] {
        assert!(staging.nested(&op, &message).is_err(), "{message:?}");
    }
}
#[test]
fn scratch_ranges_share_a_verified_spool_and_cannot_be_appended_after_seal() {
    let (_temp, mut staging) = staging();
    let op = Message::Open {
        request: 1,
        format: midnight_filesystem_api::Format::TarGzip,
        length: 10,
    };
    staging
        .nested(
            &op,
            &Message::StageBegin {
                request: 1,
                purpose: StagePurpose::Scratch,
                expected_length: None,
                max_length: u64::MAX,
            },
        )
        .unwrap();
    staging.nested(&op, &append(1, 1)).unwrap();
    staging
        .nested(
            &op,
            &Message::StageSeal {
                request: 1,
                stage: 1,
                length: 4,
            },
        )
        .unwrap();
    assert!(staging.nested(&op, &append(1, 1)).is_err());
    staging
        .terminal(
            &op,
            &Message::Opened {
                request: 1,
                root: 1,
            },
        )
        .unwrap();
    let reply = Message::ContentReady {
        request: 2,
        entry: 2,
        stage: 1,
        offset: 1,
        length: 2,
    };
    assert_eq!(
        staging
            .terminal(&read(2, 2), &reply)
            .unwrap()
            .unwrap()
            .read(0, 4)
            .unwrap(),
        b"es"
    );
}
#[test]
fn cache_eviction_preserves_an_active_lease_without_accumulating_stage_ids() {
    let (_temp, mut staging) = staging();
    let stage = seal(&mut staging, 1, 2);
    let first = staging
        .terminal(&read(1, 2), &ready(1, 2, stage))
        .unwrap()
        .unwrap();
    for entry in 3..25 {
        let stage = seal(&mut staging, entry, entry);
        staging
            .terminal(&read(entry, entry), &ready(entry, entry, stage))
            .unwrap();
    }
    assert!(staging.cached(2).is_none());
    assert!(staging.sealed.is_empty());
    assert_eq!(staging.cache.len(), CACHE_ENTRIES);
    assert_eq!(first.read(0, 4).unwrap(), b"test");
}

fn cached_ids(staging: &Staging) -> Vec<u64> {
    staging.cache.iter().map(|cached| cached.entry).collect()
}
/// Stage and publish one complete member with the given content.
fn stage_member(staging: &mut Staging, request: u64, entry: u64, content: &[u8]) -> Lease {
    let length = content.len() as u64;
    let op = read(request, entry);
    let Some(Message::StageCreated { stage, .. }) = staging
        .nested(
            &op,
            &Message::StageBegin {
                request,
                purpose: StagePurpose::Member { entry },
                expected_length: Some(length),
                max_length: length,
            },
        )
        .unwrap()
    else {
        panic!("member stage was not created")
    };
    for message in [
        Message::StageAppend {
            request,
            stage,
            offset: 0,
            bytes: content.to_vec(),
        },
        Message::StageSeal {
            request,
            stage,
            length,
        },
    ] {
        staging.nested(&op, &message).unwrap();
    }
    let reply = Message::ContentReady {
        request,
        entry,
        stage,
        offset: 0,
        length,
    };
    staging.terminal(&op, &reply).unwrap().unwrap()
}

#[test]
fn interleaved_readers_keep_a_member_larger_than_the_budget_cached() {
    let (_temp, mut staging) = staging();
    staging.cache_budget = 6;
    stage_member(&mut staging, 1, 2, b"12345678");
    stage_member(&mut staging, 2, 3, b"abcd");
    // A copy reading member 2 chunk by chunk alternates with a reader of member 3.
    for _ in 0..100 {
        assert_eq!(staging.cached(2).unwrap().read(4, 4).unwrap(), b"5678");
        assert_eq!(staging.cached(3).unwrap().read(0, 4).unwrap(), b"abcd");
    }
    assert_eq!(cached_ids(&staging), [2, 3]);
    assert_eq!(staging.cache_bytes, 12);
}

#[test]
fn members_beyond_the_budget_leave_once_later_reads_move_on() {
    let (_temp, mut staging) = staging();
    staging.cache_budget = 8;
    for entry in 2..=6 {
        stage_member(&mut staging, entry, entry, b"test");
    }
    // At most four recently read members stay beyond the budget; the fifth leaves.
    assert_eq!(cached_ids(&staging), [3, 4, 5, 6]);
    assert_eq!(staging.cache_bytes, 16);
    // Only member 6 is still read. Members not read within the last ACTIVE_READS
    // member reads leave, oldest first, until the budget holds again.
    for _ in 0..ACTIVE_READS {
        staging.cached(6).unwrap();
    }
    assert_eq!(cached_ids(&staging), [5, 6]);
    assert_eq!(staging.cache_bytes, 8);
    // Within the budget, inactive members stay for later hits.
    for _ in 0..ACTIVE_READS {
        staging.cached(6).unwrap();
    }
    assert_eq!(cached_ids(&staging), [5, 6]);
    assert_eq!(staging.cached(5).unwrap().read(0, 4).unwrap(), b"test");
}

#[test]
fn the_newest_member_stays_above_the_budget_and_recaching_replaces_its_accounting() {
    let (_temp, mut staging) = staging();
    staging.cache_budget = 2;
    let first = stage_member(&mut staging, 1, 2, b"test");
    assert_eq!(cached_ids(&staging), [2]);
    assert_eq!(staging.cache_bytes, 4);
    // Recaching the same member replaces its accounting instead of adding to it.
    let lease = staging.cached(2).unwrap();
    staging.cache_member(2, lease);
    assert_eq!(cached_ids(&staging), [2]);
    assert_eq!(staging.cache_bytes, 4);
    // Four newer members make member 2 inactive, so it leaves the cache.
    for entry in 3..=7 {
        stage_member(&mut staging, entry, entry, b"next");
    }
    assert_eq!(cached_ids(&staging), [4, 5, 6, 7]);
    assert_eq!(staging.cache_bytes, 16);
    // Evicted entries never invalidate a read that still holds its lease.
    assert_eq!(first.read(0, 4).unwrap(), b"test");
}

#[test]
fn scratch_ranges_add_no_member_bytes_to_the_cache_budget() {
    let (_temp, mut staging) = staging();
    let open = Message::Open {
        request: 1,
        format: midnight_filesystem_api::Format::TarGzip,
        length: 10,
    };
    staging
        .nested(
            &open,
            &Message::StageBegin {
                request: 1,
                purpose: StagePurpose::Scratch,
                expected_length: None,
                max_length: u64::MAX,
            },
        )
        .unwrap();
    staging.nested(&open, &append(1, 1)).unwrap();
    staging
        .nested(
            &open,
            &Message::StageSeal {
                request: 1,
                stage: 1,
                length: 4,
            },
        )
        .unwrap();
    staging
        .terminal(
            &open,
            &Message::Opened {
                request: 1,
                root: 1,
            },
        )
        .unwrap();
    for entry in 2..=20 {
        let reply = Message::ContentReady {
            request: entry,
            entry,
            stage: 1,
            offset: 0,
            length: 4,
        };
        staging.terminal(&read(entry, entry), &reply).unwrap();
    }
    assert_eq!(staging.cache.len(), CACHE_ENTRIES);
    assert_eq!(staging.cache_bytes, 0);
}

#[test]
fn a_sealed_member_cannot_be_orphaned_or_reused_by_a_later_request() {
    let (_temp, mut staging) = staging();
    let stage = seal(&mut staging, 1, 2);
    assert!(staging.nested(&read(1, 2), &begin(1, 2)).is_err());
    assert!(staging.terminal(&read(2, 2), &ready(2, 2, stage)).is_err());
    assert!(
        staging
            .terminal(
                &Message::List {
                    request: 2,
                    directory: 1,
                    after: 0
                },
                &Message::Entries {
                    request: 2,
                    entries: vec![],
                    more: false
                }
            )
            .is_err()
    );
}

#[test]
fn progress_requires_the_active_stage_and_strictly_increasing_decoded_work() {
    let (_temp, mut staging) = staging();
    let operation = read(1, 2);
    staging.nested(&operation, &begin(1, 2)).unwrap();
    let progress = |completed| Message::StageProgress {
        request: 1,
        stage: 1,
        completed,
    };
    assert!(staging.nested(&operation, &progress(0)).is_err());
    staging.nested(&operation, &progress(256 * 1024)).unwrap();
    assert!(staging.nested(&operation, &progress(256 * 1024)).is_err());
    assert!(staging.nested(&operation, &progress(1)).is_err());
    assert!(
        staging
            .nested(
                &read(2, 2),
                &Message::StageProgress {
                    request: 2,
                    stage: 1,
                    completed: 512 * 1024
                }
            )
            .is_err()
    );
}

#[test]
fn publishing_scratch_cannot_orphan_a_sealed_member() {
    let (_temp, mut staging) = staging();
    let open = Message::Open {
        request: 1,
        format: midnight_filesystem_api::Format::TarGzip,
        length: 10,
    };
    staging
        .nested(
            &open,
            &Message::StageBegin {
                request: 1,
                purpose: StagePurpose::Scratch,
                expected_length: None,
                max_length: u64::MAX,
            },
        )
        .unwrap();
    staging.nested(&open, &append(1, 1)).unwrap();
    staging
        .nested(
            &open,
            &Message::StageSeal {
                request: 1,
                stage: 1,
                length: 4,
            },
        )
        .unwrap();
    staging
        .terminal(
            &open,
            &Message::Opened {
                request: 1,
                root: 1,
            },
        )
        .unwrap();
    let member = seal(&mut staging, 2, 2);
    assert!(staging.terminal(&read(2, 2), &ready(2, 2, 1)).is_err());
    assert!(staging.terminal(&read(2, 2), &ready(2, 2, member)).is_ok());
}
