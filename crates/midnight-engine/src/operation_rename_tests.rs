//! Real native publication behind the worker's versioned, cancellable decision boundary.
use super::*;
use std::{fs, path::PathBuf};

struct Run {
    cancel: Cancellation,
    decisions: Arc<ConflictState>,
    task: tokio::task::JoinHandle<OperationProgress>,
    events: mpsc::Receiver<Event>,
}
impl Run {
    fn start(parent: &Path, name: &str) -> Self {
        Self::localized(parent, name, None)
    }
    /// With the locale `prepare_on_worker` gives a Rename job for its prompts.
    fn localized(parent: &Path, name: &str, locale: Option<Arc<dyn LocaleService>>) -> Self {
        let request = focused_source(parent, MutationKind::Rename);
        let status = Arc::new(Mutex::new(worker::initial(&request)));
        let cancel = Cancellation::default();
        let mut job = worker::prepare(
            request,
            Some(Arc::new(midnight_platform::LocalFileSystem::new(None))),
            cancel.clone(),
            status,
            Arc::new(Semaphore::new(MAX_METADATA_TARGETS)),
        )
        .unwrap();
        job.confirm(Some(name.into())).unwrap();
        job.set_state(OperationState::Running);
        job.retain_rename_locale(locale);
        let decisions = job.rename_conflicts.clone();
        let (sender, events) = mpsc::channel(8);
        let task = tokio::task::spawn_blocking(move || {
            job.execute(Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            })
        });
        Self {
            cancel,
            decisions,
            task,
            events,
        }
    }
    async fn conflict(&mut self) -> Arc<CopyConflict> {
        let event = tokio::time::timeout(Duration::from_secs(3), self.events.recv())
            .await
            .expect("Rename must publish a decision")
            .expect("Rename event channel closed");
        let Event::Operation(OperationEvent::RenameConflict(conflict)) = event else {
            panic!("unexpected {event:?}");
        };
        conflict
    }
    fn decide(&self, conflict: &CopyConflict, choice: CopyChoice) {
        self.decisions.resolve(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice,
        });
    }
    async fn finish(&mut self) -> OperationProgress {
        tokio::time::timeout(Duration::from_secs(3), &mut self.task)
            .await
            .expect("Rename must terminate")
            .unwrap()
    }
}
impl Drop for Run {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
/// A `kind` request for the focused `source.txt` of `parent`.
fn focused_source(parent: &Path, kind: MutationKind) -> PrepareOperation {
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(1),
        parent.join("source.txt"),
        EntryKind::File,
    )]
    .into();
    PrepareOperation {
        id: OperationId(7),
        kind,
        source: VisibleDirectory {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: parent.into(),
            entries: entries.clone(),
        },
        view: Arc::new(DirectoryIndex::from_entries(&entries)),
        selection: Arc::default(),
        focused: Some(EntryId(1)),
    }
}
fn fixture() -> tempfile::TempDir {
    // WSL's mounted Windows volume cannot enforce the native 0700 mutation storage contract.
    #[cfg(target_os = "linux")]
    let root = std::env::temp_dir().join("_build/midnight-rename");
    #[cfg(not(target_os = "linux"))]
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
    fs::create_dir_all(&root).unwrap();
    let root = fs::canonicalize(root).unwrap();
    let temp = tempfile::Builder::new()
        .prefix("rename-decision-")
        .tempdir_in(root)
        .unwrap();
    fs::write(temp.path().join("source.txt"), b"selected source").unwrap();
    fs::write(temp.path().join("existing.txt"), b"consented original").unwrap();
    temp
}
fn preserved(parent: &Path, target: &[u8]) {
    assert_eq!(
        fs::read(parent.join("source.txt")).unwrap(),
        b"selected source"
    );
    assert_eq!(fs::read(parent.join("existing.txt")).unwrap(), target);
}

#[tokio::test]
async fn rename_conflict_explicit_replace_preserves_original_in_reported_recovery() {
    let temp = fixture();
    let mut run = Run::start(temp.path(), "existing.txt");
    let conflict = run.conflict().await;
    preserved(temp.path(), b"consented original");
    assert!(conflict.can_replace);
    assert!(!conflict.can_merge);
    // A stale response cannot consume the current decision or confer consent.
    run.decisions.resolve(CopyDecision {
        id: conflict.id,
        version: conflict.version - 1,
        choice: CopyChoice::Cancel,
    });
    run.decide(&conflict, CopyChoice::ReplaceRecoverable { all: false });
    let done = run.finish().await;
    assert_eq!(done.state, OperationState::Completed, "{done:?}");
    assert_eq!(done.completed, 1);
    assert_eq!(done.renamed_to, Some(temp.path().join("existing.txt")));
    assert!(!temp.path().join("source.txt").exists());
    assert_eq!(
        fs::read(temp.path().join("existing.txt")).unwrap(),
        b"selected source"
    );
    let recovery = done
        .message
        .lines()
        .skip(1)
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .expect("Jobs must expose the retained original location");
    assert_eq!(fs::read(recovery).unwrap(), b"consented original");
}

#[tokio::test]
async fn rename_conflict_skip_cancel_and_shutdown_keep_both_files() {
    for choice in [Some(CopyChoice::Skip), Some(CopyChoice::Cancel), None] {
        let temp = fixture();
        let mut run = Run::start(temp.path(), "existing.txt");
        let conflict = run.conflict().await;
        if let Some(choice) = choice {
            run.decide(&conflict, choice);
        } else {
            run.cancel.cancel();
        }
        let done = run.finish().await;
        assert_eq!(done.state, OperationState::Cancelled);
        assert_eq!(done.completed, 0);
        preserved(temp.path(), b"consented original");
    }
}

#[tokio::test]
async fn rename_conflict_alternate_name_must_be_one_basename_before_any_native_call() {
    for alternate in ["nested/renamed.txt", "..", "../outside.txt", ""] {
        let temp = fixture();
        let mut run = Run::start(temp.path(), "existing.txt");
        let conflict = run.conflict().await;
        run.decide(&conflict, CopyChoice::Rename(alternate.into()));
        let done = run.finish().await;
        assert_eq!(
            done.state,
            OperationState::Failed,
            "{alternate:?}: {done:?}"
        );
        // The engine rejects the choice itself; native name validation is not the only guard.
        assert_eq!(
            done.errors[0].kind,
            io::ErrorKind::InvalidInput,
            "{alternate:?}"
        );
        assert!(
            done.errors[0].message.contains("one native basename"),
            "{alternate:?}: {done:?}"
        );
        preserved(temp.path(), b"consented original");
    }
}

#[test]
fn rename_alternate_basename_rule_matches_confirmation() {
    for valid in ["renamed.txt", "č文 🦀.txt", ".hidden"] {
        assert!(single_basename(std::ffi::OsStr::new(valid)), "{valid:?}");
    }
    for invalid in ["", ".", "..", "a/b", "/rooted", "name\0"] {
        assert!(
            !single_basename(std::ffi::OsStr::new(invalid)),
            "{invalid:?}"
        );
    }
    #[cfg(windows)]
    for invalid in ["a\\b", "C:name", "\\\\server\\share"] {
        assert!(
            !single_basename(std::ffi::OsStr::new(invalid)),
            "{invalid:?}"
        );
    }
}

/// The native adapters report raw times; each displayed prompt formats its two
/// entries.
#[tokio::test]
async fn rename_conflict_prompts_format_only_their_displayed_entries() {
    use crate::transfer_composition::tests::CountingLocale;
    let temp = fixture();
    let locale = Arc::new(CountingLocale::default());
    let mut run = Run::localized(temp.path(), "existing.txt", Some(locale.clone()));
    let conflict = run.conflict().await;
    assert_eq!(locale.calls(), 2);
    for item in [&conflict.source, &conflict.target] {
        assert!(item.metadata.modified.is_some(), "{item:?}");
        assert_eq!(
            item.metadata.localized_modified,
            Some(CountingLocale::text())
        );
    }
    // A changed target is a fresh prompt and is formatted again.
    fs::write(temp.path().join("existing.txt"), b"changed").unwrap();
    run.decide(&conflict, CopyChoice::ReplaceRecoverable { all: false });
    let fresh = run.conflict().await;
    assert!(fresh.version > conflict.version);
    assert_eq!(
        fresh.target.metadata.localized_modified,
        Some(CountingLocale::text())
    );
    assert_eq!(locale.calls(), 4);
    run.decide(&fresh, CopyChoice::Skip);
    assert_eq!(run.finish().await.state, OperationState::Cancelled);
    preserved(temp.path(), b"changed");
    assert_eq!(locale.calls(), 4);
}

/// Only a Rename prompts with transfer entries, so preparation leaves the actor's
/// locale in that lease-owned job alone. The decision state the actor indexes by
/// job, outside any lease, never keeps the locale alive.
#[tokio::test]
async fn preparation_retains_the_prompt_locale_only_in_a_rename_job() {
    let temp = fixture();
    let locale: Arc<dyn LocaleService> =
        Arc::new(crate::transfer_composition::tests::CountingLocale::default());
    for kind in [
        MutationKind::Delete(DeleteMode::Permanent),
        MutationKind::EditMetadata,
        MutationKind::CreateFile,
        MutationKind::CreateDirectory,
        MutationKind::Rename,
    ] {
        let request = focused_source(temp.path(), kind);
        let status = Arc::new(Mutex::new(worker::initial(&request)));
        let service: Arc<dyn MutationService> =
            Arc::new(midnight_platform::LocalFileSystem::new(None));
        let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
        let context = domain.try_reserve().unwrap().own((
            Input::Flat(request),
            Some(service),
            Some(locale.clone()),
        ));
        let job = tokio::task::spawn_blocking(move || {
            prepare_on_worker(
                context,
                Cancellation::default(),
                status,
                Arc::new(Semaphore::new(MAX_METADATA_TARGETS)),
            )
        })
        .await
        .unwrap()
        .unwrap();
        let rename = kind == MutationKind::Rename;
        assert_eq!(
            Arc::strong_count(&locale),
            1 + usize::from(rename),
            "only a Rename job keeps the locale: {kind:?}"
        );
        assert_eq!(job.rename_locale.is_some(), rename, "{kind:?}");
        let decisions = job.rename_conflicts.clone();
        job.retire().await.unwrap();
        assert_eq!(
            Arc::strong_count(&locale),
            1,
            "the indexed decision state owns no locale: {kind:?}"
        );
        drop(decisions);
    }
    preserved(temp.path(), b"consented original");
}

#[tokio::test]
async fn rename_conflict_changed_destination_requires_fresh_consent() {
    let temp = fixture();
    let mut run = Run::start(temp.path(), "existing.txt");
    let conflict = run.conflict().await;
    fs::write(
        temp.path().join("existing.txt"),
        b"new destination revision",
    )
    .unwrap();
    run.decide(&conflict, CopyChoice::ReplaceRecoverable { all: true });
    let fresh = run.conflict().await;
    assert!(fresh.version > conflict.version);
    assert_eq!(fresh.target.metadata.size, Some(24));
    preserved(temp.path(), b"new destination revision");
    run.decide(&conflict, CopyChoice::ReplaceRecoverable { all: true });
    run.decide(&fresh, CopyChoice::Cancel);
    assert_eq!(run.finish().await.state, OperationState::Cancelled);
    preserved(temp.path(), b"new destination revision");
}

#[tokio::test]
async fn rename_conflict_self_alias_directory_and_case_only_keep_native_semantics() {
    for name in ["source.txt", "alias.txt", "folder", "SOURCE.txt"] {
        let temp = fixture();
        fs::hard_link(
            temp.path().join("source.txt"),
            temp.path().join("alias.txt"),
        )
        .unwrap();
        fs::create_dir(temp.path().join("folder")).unwrap();
        let mut run = Run::start(temp.path(), name);
        let done = run.finish().await;
        if matches!(name, "source.txt" | "SOURCE.txt") {
            assert_eq!(done.state, OperationState::Completed, "{name}: {done:?}");
            assert_eq!(
                fs::read(temp.path().join(name)).unwrap(),
                b"selected source"
            );
        } else {
            assert_eq!(done.state, OperationState::Failed, "{name}: {done:?}");
            preserved(temp.path(), b"consented original");
        }
        assert_eq!(
            fs::read(temp.path().join("alias.txt")).unwrap(),
            b"selected source"
        );
        assert!(temp.path().join("folder").is_dir());
    }
}

#[cfg(windows)]
#[path = "operation_rename_windows_tests.rs"]
mod windows;
