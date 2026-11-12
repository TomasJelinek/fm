use super::*;

fn file(length: Option<u64>) -> MutationItem {
    MutationItem {
        path: "file".into(),
        kind: EntryKind::File,
        metadata: EntryMetadata {
            size: length,
            ..Default::default()
        },
    }
}
fn progress() -> CopyProgress {
    let mut p = CopyProgress::queued(OperationId(1), "target".into(), 2, TransferIntent::Copy);
    p.discovery = OperationDiscovery {
        state: DiscoveryState::Complete,
        files: 2,
        bytes: Some(400_100),
        ..Default::default()
    };
    p
}
#[test]
fn sampled_budget_sums_each_file_and_credits_only_published_or_active_work() {
    let policy = TransferVerification::SizeAndSamples;
    let mut budget = Budget::discovery();
    budget.observe(&file(Some(100)), policy);
    budget.observe(&file(Some(400_000)), policy);
    let mut p = progress();
    assert_eq!(
        budget.remaining(&p, Some(0)),
        Some(TransferRemainingWork {
            write_bytes: 400_100,
            verification_bytes: 196_708,
        })
    );
    budget.published(Some(100), policy);
    p.completed_bytes = 100;
    p.current_file_copied = 200_000;
    p.copied_bytes = 2_000_000;
    p.verified_bytes = 1_000_000;
    assert_eq!(
        budget.remaining(&p, Some(0)),
        Some(TransferRemainingWork {
            write_bytes: 200_000,
            verification_bytes: 196_608,
        })
    );
    p.current_file_copied = 400_000;
    p.current_file_verified = 65_536;
    assert_eq!(
        budget.remaining(&p, Some(0)).unwrap().verification_bytes,
        131_072
    );
}
#[test]
fn move_budget_credits_finalized_files_and_current_proof_not_retry_activity() {
    let policy = TransferVerification::SizeAndSamples;
    let mut budget = Budget::discovery();
    for length in [100, 400_000] {
        budget.observe(&file(Some(length)), policy);
    }
    let mut p = progress();
    p.intent = TransferIntent::Move;
    assert_eq!(
        budget.remaining(&p, Some(0)).unwrap().verification_bytes,
        983_540
    );
    budget.published(Some(100), policy);
    budget.removed(Some(100), policy);
    budget.published(Some(400_000), policy);
    p.completed_bytes = 400_100;
    p.validation_bytes = 9_000_000;
    assert_eq!(
        budget
            .remaining(&p, Some(393_216))
            .unwrap()
            .verification_bytes,
        393_216
    );
    assert_eq!(
        budget
            .remaining(&p, Some(786_432))
            .unwrap()
            .verification_bytes,
        0
    );
    // A repeated After pass removes its old logical credit, regardless of I/O total.
    assert_eq!(
        budget
            .remaining(&p, Some(393_216))
            .unwrap()
            .verification_bytes,
        393_216
    );
    budget.removed(Some(400_000), policy);
    assert_eq!(
        budget.remaining(&p, Some(0)),
        Some(TransferRemainingWork::default())
    );
}
#[test]
fn changed_unknown_unsupported_and_overflowing_work_stays_unknown() {
    let policy = TransferVerification::FullSha256;
    let mut budget = Budget::discovery();
    budget.observe(&file(Some(400_100)), policy);
    let mut p = progress();
    p.current_file_copied = 400_101;
    assert!(budget.remaining(&p, Some(0)).is_none());
    p.current_file_copied = 0;
    p.discovery.state = DiscoveryState::Unavailable;
    assert!(budget.remaining(&p, Some(0)).is_none());
    p.discovery.state = DiscoveryState::Complete;
    p.discovery.scope = DiscoveryScope::SelectedRoots;
    assert!(budget.remaining(&p, Some(0)).is_none());
    p.discovery.scope = DiscoveryScope::Recursive;
    budget.observe(&file(None), policy);
    assert!(budget.remaining(&p, Some(0)).is_none());
    budget = Budget::discovery();
    budget.observe(&file(Some(u64::MAX)), policy);
    p.discovery.bytes = Some(u64::MAX);
    p.intent = TransferIntent::Move;
    assert!(budget.remaining(&p, Some(0)).is_none());
    budget.observe(&file(Some(1)), policy);
    assert!(budget.remaining(&p, Some(0)).is_none());
}
