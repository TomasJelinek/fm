//! Acceptance is observable through the real serialized preferences worker.
use super::*;
use midnight_core::{FileComparatorField, FileComparatorRule, FileComparatorTarget, FilterMode};
use std::path::PathBuf;

fn configured(target: FileComparatorTarget, limit: u8) -> UserPreferences {
    UserPreferences {
        file_comparators: FileComparatorPreferences {
            rules: vec![FileComparatorRule {
                enabled: true,
                mode: FilterMode::Regex,
                expression: r"\.txt$".into(),
                case_sensitive: true,
                target,
            }],
        },
        ..disk_preferences(limit)
    }
}

fn invalid_disabled() -> UserPreferences {
    let mut preferences = configured(FileComparatorTarget::BuiltInText, 12);
    preferences.file_comparators.rules.push(FileComparatorRule {
        enabled: false,
        mode: FilterMode::Regex,
        expression: "[".into(),
        case_sensitive: false,
        target: FileComparatorTarget::Automatic,
    });
    // This failure must come from semantic compilation, not raw Core validation.
    preferences.validate().unwrap();
    preferences
}

fn same_policy(actual: &FileComparatorSnapshot, expected: &FileComparatorSnapshot) {
    assert_eq!(actual.id, expected.id);
    assert!(Arc::ptr_eq(&actual.preferences, &expected.preferences));
}

fn assert_pair_selection(store: &FileComparatorPolicyStore, snapshot: &FileComparatorSnapshot) {
    let policy = store
        .pin(snapshot)
        .expect("acknowledgement must pin the current policy");
    assert_eq!(
        policy.select(&[PathBuf::from("left.txt"), PathBuf::from("right.txt")]),
        Some(0)
    );
    assert_eq!(
        policy.select(&[PathBuf::from("left.txt"), PathBuf::from("right.bin")]),
        None
    );
}

async fn saved(events: &mut mpsc::Receiver<Event>, revision: u64) -> FileComparatorSnapshot {
    let Event::PreferencesSaved {
        revision: actual,
        result,
    } = next(events).await
    else {
        panic!("expected save acknowledgement");
    };
    assert_eq!(actual, PreferencesRevision(revision));
    result.expect("valid preferences must be accepted")
}

#[tokio::test]
async fn changed_save_acknowledges_the_persisted_raw_policy_and_compiled_pair_matcher() {
    let service = Arc::new(Settings::default());
    let (controller, mut tasks, mut events) = controller(service.clone());
    let store = &controller.shared.comparators;
    let initial = store.snapshot();
    let submitted = configured(FileComparatorTarget::BuiltInText, 7);
    let caller = std::thread::current().id();
    assert!(controller.save(PreferencesRevision(31), submitted.clone()));
    let accepted = saved(&mut events, 31).await;
    assert_ne!(accepted.id, initial.id);
    assert_eq!(*accepted.preferences, submitted.file_comparators);
    same_policy(&store.snapshot(), &accepted);
    assert_pair_selection(store, &accepted);
    assert_eq!(*service.persisted.lock().unwrap(), submitted);
    assert_eq!(service.writes.lock().unwrap().len(), 1);
    assert_eq!(service.prepared_saves.load(Ordering::SeqCst), 1);
    assert!(
        service
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|thread| *thread != caller)
    );
    stop(&controller, &mut tasks).await;
}

#[tokio::test]
async fn unrelated_save_reuses_the_exact_accepted_raw_arc_and_policy_id() {
    let service = Arc::new(Settings::default());
    let (controller, mut tasks, mut events) = controller(service.clone());
    let first = configured(FileComparatorTarget::BuiltInText, 7);
    assert!(controller.save(PreferencesRevision(1), first.clone()));
    let accepted = saved(&mut events, 1).await;
    let unrelated = UserPreferences {
        theme: PreferenceTheme::Light,
        ..first
    };
    assert!(controller.save(PreferencesRevision(2), unrelated.clone()));
    let reused = saved(&mut events, 2).await;
    same_policy(&reused, &accepted);
    same_policy(&controller.shared.comparators.snapshot(), &accepted);
    assert_eq!(*service.persisted.lock().unwrap(), unrelated);
    assert_eq!(service.writes.lock().unwrap().len(), 2);
    assert_eq!(service.prepared_saves.load(Ordering::SeqCst), 2);
    stop(&controller, &mut tasks).await;
}

#[tokio::test]
async fn disabled_invalid_regex_reports_its_submitted_revision_without_write_or_publication() {
    let service = Arc::new(Settings::default());
    let (controller, mut tasks, mut events) = controller(service.clone());
    let prior = configured(FileComparatorTarget::BuiltInText, 7);
    assert!(controller.save(PreferencesRevision(1), prior.clone()));
    let accepted = saved(&mut events, 1).await;
    assert!(controller.save(PreferencesRevision(41), invalid_disabled()));
    let Event::PreferencesSaved {
        revision,
        result: Err(error),
    } = next(&mut events).await
    else {
        panic!("disabled invalid regex must fail acceptance");
    };
    assert_eq!(revision, PreferencesRevision(41));
    assert_eq!(error.error.kind, io::ErrorKind::InvalidInput);
    let diagnostic = error.comparator.expect("typed comparator diagnostic");
    assert_eq!(diagnostic.rule, Some(1));
    assert_eq!(diagnostic.field, FileComparatorField::Expression);
    assert!(!diagnostic.message.is_empty());
    same_policy(&controller.shared.comparators.snapshot(), &accepted);
    assert_eq!(controller.shared.disk_io.effective_limit(), 7);
    assert_eq!(*service.persisted.lock().unwrap(), prior);
    assert_eq!(service.writes.lock().unwrap().len(), 1);
    stop(&controller, &mut tasks).await;
}

#[tokio::test]
async fn failed_persistence_keeps_prior_compiled_policy_disk_limit_and_persisted_preferences() {
    let service = Arc::new(Settings::default());
    let (controller, mut tasks, mut events) = controller(service.clone());
    let prior = configured(FileComparatorTarget::BuiltInText, 7);
    assert!(controller.save(PreferencesRevision(1), prior.clone()));
    let accepted = saved(&mut events, 1).await;
    let replacement = configured(FileComparatorTarget::BuiltInImages, 12);
    service.fail_save.store(true, Ordering::SeqCst);
    assert!(controller.save(PreferencesRevision(2), replacement.clone()));
    let Event::PreferencesSaved {
        revision,
        result: Err(error),
    } = next(&mut events).await
    else {
        panic!("persistence failure must be acknowledged");
    };
    assert_eq!(revision, PreferencesRevision(2));
    assert_eq!(error.error.kind, io::ErrorKind::PermissionDenied);
    assert!(error.comparator.is_none());
    same_policy(&controller.shared.comparators.snapshot(), &accepted);
    assert_pair_selection(&controller.shared.comparators, &accepted);
    assert_eq!(controller.shared.disk_io.effective_limit(), 7);
    assert_eq!(*service.persisted.lock().unwrap(), prior);
    assert!(controller.save(PreferencesRevision(3), replacement.clone()));
    let retried = saved(&mut events, 3).await;
    assert_ne!(retried.id, accepted.id);
    assert_eq!(*retried.preferences, replacement.file_comparators);
    assert_eq!(*service.persisted.lock().unwrap(), replacement);
    assert_eq!(controller.shared.disk_io.effective_limit(), 12);
    stop(&controller, &mut tasks).await;
}

#[tokio::test]
async fn load_returns_matching_accepted_preferences_and_policy_metadata() {
    let submitted = configured(FileComparatorTarget::BuiltInText, 7);
    let service = Arc::new(Settings {
        persisted: Mutex::new(submitted.clone()),
        ..Settings::default()
    });
    let (controller, mut tasks, mut events) = controller(service.clone());
    assert!(controller.load());
    let Event::PreferencesLoaded(Ok(accepted)) = next(&mut events).await else {
        panic!("valid persisted comparators must load");
    };
    assert_eq!(accepted.preferences, submitted);
    assert_eq!(
        *accepted.comparators.preferences,
        submitted.file_comparators
    );
    same_policy(
        &controller.shared.comparators.snapshot(),
        &accepted.comparators,
    );
    assert_pair_selection(&controller.shared.comparators, &accepted.comparators);
    assert_eq!(controller.shared.disk_io.effective_limit(), 7);
    assert!(service.writes.lock().unwrap().is_empty());
    assert_eq!(service.acceptance_loads.load(Ordering::SeqCst), 1);
    stop(&controller, &mut tasks).await;
}

#[tokio::test]
async fn rejected_loaded_regex_preserves_the_previously_accepted_policy_and_disk_limit() {
    let service = Arc::new(Settings::default());
    let (controller, mut tasks, mut events) = controller(service.clone());
    assert!(controller.save(
        PreferencesRevision(1),
        configured(FileComparatorTarget::BuiltInText, 7)
    ));
    let accepted = saved(&mut events, 1).await;
    *service.persisted.lock().unwrap() = invalid_disabled();
    assert!(controller.load());
    let Event::PreferencesLoaded(Err(error)) = next(&mut events).await else {
        panic!("invalid loaded regex must fail acceptance");
    };
    assert_eq!(error.error.kind, io::ErrorKind::InvalidInput);
    let diagnostic = error.comparator.expect("typed comparator diagnostic");
    assert_eq!(diagnostic.rule, Some(1));
    assert_eq!(diagnostic.field, FileComparatorField::Expression);
    same_policy(&controller.shared.comparators.snapshot(), &accepted);
    assert_pair_selection(&controller.shared.comparators, &accepted);
    assert_eq!(controller.shared.disk_io.effective_limit(), 7);
    assert_eq!(service.writes.lock().unwrap().len(), 1);
    stop(&controller, &mut tasks).await;
}

#[tokio::test]
async fn publication_waits_for_successful_persistence_and_never_exposes_the_candidate_early() {
    let gate = Arc::new(Gate::default());
    let _release = ReleaseOnDrop(gate.clone());
    let prior = configured(FileComparatorTarget::BuiltInText, 7);
    let service = Arc::new(Settings {
        persisted: Mutex::new(prior.clone()),
        save_gate: Some(gate.clone()),
        ..Settings::default()
    });
    let (controller, mut tasks, mut events) = controller(service.clone());
    assert!(controller.load());
    let Event::PreferencesLoaded(Ok(loaded)) = next(&mut events).await else {
        panic!("initial load must succeed");
    };
    let accepted = loaded.comparators;
    let replacement = configured(FileComparatorTarget::BuiltInImages, 12);
    assert!(controller.save(PreferencesRevision(1), replacement.clone()));
    gate.wait().await;
    same_policy(&controller.shared.comparators.snapshot(), &accepted);
    assert_pair_selection(&controller.shared.comparators, &accepted);
    assert_eq!(controller.shared.disk_io.effective_limit(), 7);
    assert_eq!(*service.persisted.lock().unwrap(), prior);
    assert_eq!(*service.writes.lock().unwrap(), vec![replacement.clone()]);
    assert!(events.try_recv().is_err());
    gate.release();
    let published = saved(&mut events, 1).await;
    assert_ne!(published.id, accepted.id);
    assert_eq!(*published.preferences, replacement.file_comparators);
    same_policy(&controller.shared.comparators.snapshot(), &published);
    assert_eq!(*service.persisted.lock().unwrap(), replacement);
    assert_eq!(controller.shared.disk_io.effective_limit(), 12);
    stop(&controller, &mut tasks).await;
}
