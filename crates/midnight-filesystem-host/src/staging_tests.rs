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
#[test]
fn an_unusable_spool_root_yields_to_the_next_private_candidate() {
    let temp = tempfile::tempdir().unwrap();
    // A file where a directory is required can never become a private root.
    let occupied = temp.path().join("occupied");
    std::fs::write(&occupied, b"not a directory").unwrap();
    let private = temp.path().join("private");
    let mut staging = Staging::open_first([occupied.clone(), private.clone()]).unwrap();
    assert!(private.is_dir());
    let op = read(1, 2);
    staging.nested(&op, &begin(1, 2)).unwrap();
    let error = Staging::open_first([occupied.clone()]).err().unwrap();
    assert!(error.to_string().contains("occupied"), "{error}");
    assert!(Staging::open_first(Vec::new()).is_err());
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
    ABOVE_RESERVE.set(above_reserve);
    QUERIES.set(0);
    (temp, staging)
}
fn pending_length(staging: &Staging) -> u64 {
    staging.pending.as_ref().unwrap().1.spool.length()
}

#[test]
fn staging_stops_before_the_spool_volume_reserve_and_writes_nothing() {
    // Three bytes above the reserve cannot take a four-byte chunk.
    let (_temp, mut staging) = staging_on_fake_volume(3);
    let op = read(1, 2);
    staging.nested(&op, &begin(1, 2)).unwrap();
    let error = staging.nested(&op, &append(1, 1)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
    assert!(error.to_string().contains("free-space reserve"), "{error}");
    assert_eq!(pending_length(&staging), 0);
    assert_eq!(QUERIES.get(), 1);
    // The volume may fill exactly up to its reserve.
    let (_temp, mut staging) = staging_on_fake_volume(4);
    staging.nested(&op, &begin(1, 2)).unwrap();
    staging.nested(&op, &append(1, 1)).unwrap();
    assert_eq!(pending_length(&staging), 4);
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
