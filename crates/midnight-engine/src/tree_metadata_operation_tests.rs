//! Captured native metadata joins the same versioned preview and mutation service.
use super::*;

#[derive(Debug)]
struct Sources {
    paths: Vec<std::path::PathBuf>,
    covered: Vec<bool>,
}
impl CapturedMetadataSources for Sources {
    fn stamp(&self) -> TreeViewStamp {
        TreeViewStamp {
            pane: PaneId::Left,
            navigation: NavigationId(7),
            session: TreeSessionId(2),
            revision: TreeRevision(3),
        }
    }
    fn request(&self) -> TreeRequestId {
        TreeRequestId(4)
    }
    fn root_count(&self) -> usize {
        self.paths.len()
    }
    fn covered_roots(&self) -> &[bool] {
        &self.covered
    }
    fn prepare(
        &self,
        service: &dyn MutationService,
        cancel: &Cancellation,
    ) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
        self.paths
            .iter()
            .map(|path| service.prepare(path, cancel))
            .collect()
    }
}
fn request(
    h: &Harness,
    paths: Vec<std::path::PathBuf>,
    covered: Vec<bool>,
) -> PrepareCapturedMetadata {
    PrepareCapturedMetadata {
        id: h.request.id,
        sources: Arc::new(Sources { paths, covered }),
        source_directory: h.directory.path().join("presentation-only-directory"),
    }
}

#[tokio::test]
async fn tree_metadata_native_case_and_times_require_current_preview_and_confirmation() {
    for recursive in [false, true] {
        let mut h = Harness::new(&[("PARENT", true)]);
        fs::create_dir(h.directory.path().join("PARENT/CHILD")).unwrap();
        fs::write(
            h.directory.path().join("PARENT/CHILD/FILE.TXT"),
            b"unchanged bytes",
        )
        .unwrap();
        let request = request(
            &h,
            vec![
                h.directory.path().join("PARENT/CHILD"),
                h.directory.path().join("PARENT"),
            ],
            vec![true, false],
        );
        h.controller.prepare_captured_metadata(request).await;
        let initial = h.preview(0).await;
        assert_eq!(initial.total, 2);
        assert!(h.directory.path().join("PARENT/CHILD").is_dir());
        // Captured targets, not current pane location, remain authority after navigation.
        *h.controller.snapshots.lock().unwrap() = Default::default();
        let time = UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        h.controller
            .preview_metadata(
                h.request.id,
                1,
                MetadataPatch {
                    case: Some(CaseChange::default()),
                    modified: TimeEdit::Set(time),
                    recursive,
                    ..Default::default()
                },
            )
            .await;
        let reviewed = h.preview(1).await;
        assert_eq!(reviewed.unsupported, 0, "{reviewed:?}");
        assert!(reviewed.errors.is_empty(), "{reviewed:?}");
        h.controller
            .confirm_metadata(h.request.id, 0, MetadataConflictPolicy::Skip)
            .await;
        assert!(
            h.directory.path().join("PARENT/CHILD").is_dir(),
            "stale revision cannot execute"
        );
        // The stale confirmation releases its admission reservation asynchronously.
        // A fresh reviewed preview is the actor barrier before the next explicit Apply.
        h.controller
            .preview_metadata(h.request.id, 2, reviewed.patch.clone())
            .await;
        let current = h.preview(2).await;
        assert!(
            current
                .sample
                .iter()
                .all(|row| row.current.modified != Some(time)),
            "stale confirmation must not apply timestamps"
        );
        assert!(
            fs::read_dir(h.directory.path())
                .unwrap()
                .any(|entry| entry.unwrap().file_name() == "PARENT"),
            "stale confirmation must not change native spelling"
        );
        h.controller
            .confirm_metadata(h.request.id, 2, MetadataConflictPolicy::Skip)
            .await;
        let (result, done) = h.result().await;
        assert_eq!(done.state, OperationState::Completed, "{done:?}");
        assert_eq!(result.processed, if recursive { 3 } else { 2 });
        assert_eq!(result.changed, result.processed);
        let file = h.directory.path().join(if recursive {
            "parent/child/file.txt"
        } else {
            "parent/child/FILE.TXT"
        });
        assert_eq!(fs::read(file).unwrap(), b"unchanged bytes");
        assert_eq!(
            fs::metadata(h.directory.path().join("parent/child"))
                .unwrap()
                .modified()
                .unwrap(),
            time
        );
        assert_eq!(done.completed, done.total);
    }
}

#[tokio::test]
async fn tree_metadata_native_cancelled_preview_never_changes_selected_descendant() {
    let mut h = Harness::new(&[("PARENT", true)]);
    let path = h.directory.path().join("PARENT/CHILD");
    fs::create_dir(&path).unwrap();
    h.controller
        .prepare_captured_metadata(request(&h, vec![path.clone()], vec![false]))
        .await;
    h.preview(0).await;
    h.controller.cancel(h.request.id);
    loop {
        if let Event::Operation(OperationEvent::Progress(progress)) =
            tokio::time::timeout(Duration::from_secs(4), h.events.recv())
                .await
                .unwrap()
                .unwrap()
            && progress.state.is_terminal()
        {
            assert_eq!(progress.state, OperationState::Cancelled);
            break;
        }
    }
    assert!(path.is_dir());
}

#[tokio::test]
async fn tree_metadata_pin_budget_refuses_complete_batch_before_native_prepare() {
    let h = Harness::new(&[("PARENT", true)]);
    let request = request(
        &h,
        vec![
            h.directory.path().join("missing-one"),
            h.directory.path().join("missing-two"),
        ],
        vec![false, false],
    );
    let scope = super::super::input::MetadataScope::capture(request.sources.as_ref());
    let status = Arc::new(Mutex::new(super::super::worker::initial_for(
        request.id,
        MutationKind::EditMetadata,
        request.source_directory.clone(),
    )));
    let result = super::super::worker::prepare_captured_metadata(
        request,
        scope,
        Some(Arc::new(LocalFileSystem::new(None))),
        Cancellation::default(),
        status,
        Arc::new(Semaphore::new(1)),
    );
    let error = match result {
        Ok(_) => panic!("batch exceeded budget"),
        Err(error) => error,
    };
    assert!(
        error.message.contains("shared metadata handle budget"),
        "{error:?}"
    );
}

#[tokio::test]
async fn tree_metadata_preview_rejects_replacement_of_captured_descendant_identity() {
    let mut h = Harness::new(&[("PARENT", true)]);
    let original = h.directory.path().join("PARENT/FILE.TXT");
    fs::write(&original, b"original").unwrap();
    h.controller
        .prepare_captured_metadata(request(&h, vec![original.clone()], vec![false]))
        .await;
    h.preview(0).await;
    fs::rename(&original, original.with_file_name("retained-original")).unwrap();
    fs::write(&original, b"replacement").unwrap();
    h.controller
        .preview_metadata(
            h.request.id,
            1,
            MetadataPatch {
                modified: TimeEdit::Set(UNIX_EPOCH + Duration::from_secs(1_600_000_000)),
                ..Default::default()
            },
        )
        .await;
    let preview = h.preview(1).await;
    assert_eq!(preview.unsupported, 1);
    assert!(preview.sample[0].issue.is_some());
    assert_eq!(fs::read(original).unwrap(), b"replacement");
}
