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

#[test]
fn member_cache_is_bounded_by_bytes_and_always_keeps_the_newest_member() {
    let (_temp, mut staging) = staging();
    staging.cache_budget = 8;
    let cached_ids =
        |staging: &Staging| -> Vec<u64> { staging.cache.iter().map(|(entry, _)| *entry).collect() };
    let mut leases = Vec::new();
    for entry in 2..=4 {
        let stage = seal(&mut staging, entry, entry);
        leases.push(
            staging
                .terminal(&read(entry, entry), &ready(entry, entry, stage))
                .unwrap()
                .unwrap(),
        );
    }
    // Three 4-byte members exceed 8 bytes: the least recently used one leaves.
    assert_eq!(cached_ids(&staging), [3, 4]);
    assert_eq!(staging.cache_bytes, 8);
    // Reading member 3 makes member 4 the eviction candidate.
    assert_eq!(staging.cached(3).unwrap().read(0, 4).unwrap(), b"test");
    let stage = seal(&mut staging, 5, 5);
    staging
        .terminal(&read(5, 5), &ready(5, 5, stage))
        .unwrap()
        .unwrap();
    assert_eq!(cached_ids(&staging), [3, 5]);
    // A member larger than the whole budget still stays as the only entry.
    staging.cache_budget = 2;
    let stage = seal(&mut staging, 6, 6);
    staging
        .terminal(&read(6, 6), &ready(6, 6, stage))
        .unwrap()
        .unwrap();
    assert_eq!(cached_ids(&staging), [6]);
    assert_eq!(staging.cache_bytes, 4);
    // Recaching the same member replaces its accounting instead of adding to it.
    let lease = staging.cached(6).unwrap();
    staging.cache_member(6, lease);
    assert_eq!(staging.cache_bytes, 4);
    // Evicted entries never invalidate a read that still holds its lease.
    assert_eq!(leases[0].read(0, 4).unwrap(), b"test");
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
