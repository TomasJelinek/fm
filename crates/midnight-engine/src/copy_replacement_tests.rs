use super::*;

#[tokio::test]
async fn replacement_cannot_capture_another_selected_source_root() {
    let temp = fixture();
    let source = temp.path().join("same-name");
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let other_source = destination.join("same-name");
    fs::write(&source, b"first selected source").unwrap();
    fs::write(&other_source, b"second selected source").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, &[source, other_source.clone()], &destination))
        .await;
    let conflict = h.conflict(1).await;
    assert!(!conflict.can_replace && !conflict.can_merge);
    h.controller
        .decide(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::SkipAll,
        })
        .await;
    let done = h.terminal(1).await;
    assert_eq!(done.skipped, 2);
    assert_eq!(done.recovery_count, 0);
    assert_eq!(fs::read(other_source).unwrap(), b"second selected source");
    h.close().await;
}

#[test]
fn replace_all_does_not_authorize_a_late_new_destination() {
    let temp = fixture();
    let source = temp.path().join("first");
    let second = temp.path().join("second");
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    fs::write(&source, b"one").unwrap();
    fs::write(&second, b"two").unwrap();
    fs::write(destination.join("first"), b"old first").unwrap();
    let request = request(1, &[source, second], &destination);
    let status = Arc::new(Mutex::new(initial(&request.clone().into())));
    let service = midnight_platform::LocalFileSystem::new(None);
    let mut job =
        worker::Job::prepare(request, &service, Cancellation::default(), status.clone()).unwrap();
    let mut first_version = 0;
    for step in 0..100 {
        if let worker::Step::Conflict(conflict) = job.step() {
            first_version = conflict.version;
            assert!(job.accept_decision(CopyDecision {
                id: conflict.id,
                version: conflict.version,
                choice: CopyChoice::ReplaceRecoverable { all: true }
            }));
            break;
        }
        assert!(step < 99);
    }
    for step in 0..100 {
        assert!(matches!(job.step(), worker::Step::Continue));
        let progress = snapshot(&status);
        if progress.files_completed == 1
            && progress.verified_bytes == 6
            && progress.phase == CopyOperationPhase::Publishing
        {
            break;
        }
        assert!(step < 99);
    }
    fs::write(destination.join("second"), b"newly arrived").unwrap();
    let worker::Step::Conflict(conflict) = job.step() else {
        panic!("late target must be shown");
    };
    assert!(conflict.version > first_version);
    assert_eq!(conflict.target.metadata.size, Some(13));
    assert_eq!(
        fs::read(destination.join("second")).unwrap(),
        b"newly arrived"
    );
    assert!(job.accept_decision(CopyDecision {
        id: conflict.id,
        version: conflict.version,
        choice: CopyChoice::Cancel
    }));
    assert!(matches!(job.step(), worker::Step::Done));
    assert_eq!(job.finish().state, CopyOperationState::Cancelled);
}

#[test]
fn verified_stage_survives_a_late_collision_and_explicit_replacement_or_rename() {
    for replace in [false, true] {
        let temp = fixture();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir(&destination).unwrap();
        fs::write(&source, b"verified payload").unwrap();
        let request = request(1, std::slice::from_ref(&source), &destination);
        let status = Arc::new(Mutex::new(initial(&request.clone().into())));
        let service = midnight_platform::LocalFileSystem::new(None);
        let mut job =
            worker::Job::prepare(request, &service, Cancellation::default(), status.clone())
                .unwrap();
        for step in 0..100 {
            assert!(matches!(job.step(), worker::Step::Continue), "step {step}");
            if snapshot(&status).phase == CopyOperationPhase::Publishing {
                break;
            }
            assert!(step < 99, "copy did not reach publication");
        }
        assert!(!destination.join("source").exists());
        fs::write(destination.join("source"), b"late original").unwrap();
        let worker::Step::Conflict(conflict) = job.step() else {
            panic!("expected late conflict");
        };
        assert_eq!(snapshot(&status).verified_bytes, 16);
        assert!(job.accept_decision(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: if replace {
                CopyChoice::ReplaceRecoverable { all: false }
            } else {
                CopyChoice::Rename("renamed".into())
            },
        }));
        for step in 0..100 {
            match job.step() {
                worker::Step::Done => break,
                worker::Step::Continue => assert!(step < 99),
                worker::Step::Conflict(_) => panic!("unexpected second conflict"),
            }
        }
        let done = job.finish();
        assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
        assert_eq!(done.copied_bytes, 16, "never reread a consumed source");
        assert_eq!(done.verified_bytes, 16);
        assert_eq!(fs::read(source).unwrap(), b"verified payload");
        if replace {
            assert_eq!(
                fs::read(destination.join("source")).unwrap(),
                b"verified payload"
            );
            assert_eq!(fs::read(&done.recovery_paths[0]).unwrap(), b"late original");
        } else {
            assert_eq!(
                fs::read(destination.join("source")).unwrap(),
                b"late original"
            );
            assert_eq!(
                fs::read(destination.join("renamed")).unwrap(),
                b"verified payload"
            );
            assert_eq!(done.recovery_count, 0);
        }
    }
}

#[tokio::test]
async fn changed_target_requires_fresh_consent_even_for_replace_all() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    fs::write(&source, b"new source").unwrap();
    fs::write(destination.join("source"), b"old target").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, &[source], &destination))
        .await;
    let original = h.conflict(1).await;
    fs::write(
        destination.join("source"),
        b"changed while the dialog was open",
    )
    .unwrap();
    h.controller
        .decide(CopyDecision {
            id: original.id,
            version: original.version,
            choice: CopyChoice::ReplaceRecoverable { all: true },
        })
        .await;
    let fresh = h.conflict(1).await;
    assert!(fresh.version > original.version);
    assert_eq!(fresh.target.metadata.size, Some(33));
    h.controller
        .decide(CopyDecision {
            id: original.id,
            version: original.version,
            choice: CopyChoice::ReplaceRecoverable { all: true },
        })
        .await;
    h.controller
        .decide(CopyDecision {
            id: fresh.id,
            version: fresh.version,
            choice: CopyChoice::Skip,
        })
        .await;
    let done = h.terminal(1).await;
    assert_eq!(done.skipped, 1);
    assert_eq!(done.files_completed, 0);
    assert_eq!(done.recovery_count, 0);
    assert_eq!(
        fs::read(destination.join("source")).unwrap(),
        b"changed while the dialog was open"
    );
    h.close().await;
}

#[tokio::test]
async fn explicit_replacement_verifies_new_bytes_and_retains_the_original() {
    let temp = fixture();
    let source = temp.path().join("文档.txt");
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    fs::write(&source, b"verified new content").unwrap();
    fs::write(destination.join("文档.txt"), b"previous content").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    let conflict = h.conflict(1).await;
    assert!(conflict.can_replace);
    assert!(!conflict.can_merge);
    h.controller
        .decide(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::ReplaceRecoverable { all: false },
        })
        .await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.verified_bytes, 20);
    assert_eq!(done.recovery_count, 1);
    assert_eq!(
        fs::read(&done.recovery_paths[0]).unwrap(),
        b"previous content"
    );
    assert_eq!(
        fs::read(destination.join("文档.txt")).unwrap(),
        b"verified new content"
    );
    assert_eq!(fs::read(source).unwrap(), b"verified new content");
    h.close().await;
}

#[tokio::test]
async fn merge_consent_keeps_existing_children_and_does_not_authorize_file_replacement() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(destination.join("source")).unwrap();
    fs::write(source.join("new"), b"new child").unwrap();
    fs::write(source.join("collision"), b"source value").unwrap();
    fs::write(destination.join("source/keep"), b"keep child").unwrap();
    fs::write(destination.join("source/collision"), b"keep original").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    let conflict = h.conflict(1).await;
    assert!(conflict.can_merge);
    assert!(!conflict.can_replace);
    h.controller
        .decide(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::Merge { all: true },
        })
        .await;
    let file_conflict = h.conflict(1).await;
    assert!(file_conflict.can_replace);
    assert!(!file_conflict.can_merge);
    assert_eq!(
        file_conflict.target.path,
        destination.join("source/collision")
    );
    h.controller
        .decide(CopyDecision {
            id: file_conflict.id,
            version: file_conflict.version,
            choice: CopyChoice::Skip,
        })
        .await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::CompletedWithErrors);
    assert_eq!(done.skipped, 1);
    assert_eq!(
        fs::read(destination.join("source/new")).unwrap(),
        b"new child"
    );
    assert_eq!(
        fs::read(destination.join("source/keep")).unwrap(),
        b"keep child"
    );
    assert_eq!(
        fs::read(destination.join("source/collision")).unwrap(),
        b"keep original"
    );
    assert!(source.join("collision").exists());
    h.close().await;
}

#[tokio::test]
async fn replacement_all_is_limited_to_the_current_job() {
    let temp = fixture();
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let sources = [temp.path().join("first"), temp.path().join("second")];
    for source in &sources {
        fs::write(source, b"new").unwrap();
        fs::write(destination.join(source.file_name().unwrap()), b"old").unwrap();
    }
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, &sources, &destination))
        .await;
    let conflict = h.conflict(1).await;
    h.controller
        .decide(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::ReplaceRecoverable { all: true },
        })
        .await;
    let done = h.terminal(1).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.files_completed, 2);
    assert_eq!(done.recovery_count, 2);
    h.controller
        .enqueue(request(2, &sources[..1], &destination))
        .await;
    let next = h.conflict(2).await;
    h.controller
        .decide(CopyDecision {
            id: next.id,
            version: next.version,
            choice: CopyChoice::Cancel,
        })
        .await;
    assert_eq!(h.terminal(2).await.state, CopyOperationState::Cancelled);
    h.close().await;
}

#[tokio::test]
async fn source_destination_alias_cannot_authorize_replacement_or_merge() {
    let temp = fixture();
    let source = temp.path().join("same-file");
    fs::write(&source, b"untouched").unwrap();
    let mut h = Harness::native();
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), temp.path()))
        .await;
    let conflict = h.conflict(1).await;
    assert!(!conflict.can_replace && !conflict.can_merge);
    h.controller
        .decide(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::ReplaceRecoverable { all: false },
        })
        .await;
    assert_ne!(h.terminal(1).await.state, CopyOperationState::Completed);
    assert_eq!(fs::read(source).unwrap(), b"untouched");
    h.close().await;
}
