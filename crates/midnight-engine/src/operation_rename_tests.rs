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
        let entries: Arc<[Entry]> = vec![Entry::new(
            EntryId(1),
            parent.join("source.txt"),
            EntryKind::File,
        )]
        .into();
        let request = PrepareOperation {
            id: OperationId(7),
            kind: MutationKind::Rename,
            source: VisibleDirectory {
                pane: PaneId::Left,
                navigation: NavigationId(1),
                path: parent.into(),
                entries: entries.clone(),
            },
            view: Arc::new(DirectoryIndex::from_entries(&entries)),
            selection: Arc::default(),
            focused: Some(EntryId(1)),
        };
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
