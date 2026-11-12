//! Reject malformed capture contracts before confirmation or mutation.
use super::*;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Copy, Debug)]
enum Change {
    OutputCount,
    Count,
    Mode,
    Stamp,
    Request,
    InvalidScope,
}
#[derive(Debug)]
struct ChangingSources {
    base: Sources,
    change: Change,
    changed: AtomicBool,
}
impl ChangingSources {
    fn changed(&self) -> bool {
        self.changed.load(Ordering::Relaxed)
    }
}
impl CapturedDeleteSources for ChangingSources {
    fn stamp(&self) -> TreeViewStamp {
        let mut stamp = self.base.stamp();
        if matches!(self.change, Change::InvalidScope) {
            stamp.session = TreeSessionId(0);
        }
        if self.changed() && matches!(self.change, Change::Stamp) {
            stamp.revision.0 += 1;
        }
        stamp
    }
    fn request(&self) -> TreeRequestId {
        let mut request = self.base.request();
        if self.changed() && matches!(self.change, Change::Request) {
            request.0 += 1;
        }
        request
    }
    fn mode(&self) -> DeleteMode {
        if self.changed() && matches!(self.change, Change::Mode) {
            DeleteMode::Permanent
        } else {
            self.base.mode()
        }
    }
    fn root_count(&self) -> usize {
        self.base.root_count() + usize::from(self.changed() && matches!(self.change, Change::Count))
    }
    fn prepare(
        &self,
        service: &dyn MutationService,
        cancel: &Cancellation,
    ) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
        let mut targets = self.base.prepare(service, cancel)?;
        if matches!(self.change, Change::OutputCount) {
            targets.pop();
        }
        self.changed.store(true, Ordering::Relaxed);
        Ok(targets)
    }
}

#[tokio::test]
async fn captured_tree_delete_rejects_changed_mode_count_scope_and_incomplete_preparation() {
    for change in [
        Change::OutputCount,
        Change::Count,
        Change::Mode,
        Change::Stamp,
        Change::Request,
        Change::InvalidScope,
    ] {
        let mut h = Harness::new(MutationKind::Delete(DeleteMode::Trash));
        let request = PrepareCapturedDelete {
            id: h.request.id,
            source_directory: h.request.source.path.clone(),
            sources: Arc::new(ChangingSources {
                base: Sources {
                    paths: vec![
                        h.request.source.path.join("one"),
                        h.request.source.path.join("two"),
                    ],
                    mode: DeleteMode::Trash,
                    dropped: None,
                },
                change,
                changed: AtomicBool::new(false),
            }),
        };
        h.controller.prepare_captured_delete(request).await;
        let done = h.finished().await;
        assert_eq!(done.state, OperationState::Failed, "{change:?}: {done:?}");
        assert_eq!(
            h.calls.prepared.load(Ordering::Relaxed),
            if matches!(change, Change::InvalidScope) {
                0
            } else {
                2
            }
        );
        assert!(h.calls.deleted.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn captured_tree_delete_shares_the_flat_queue_limit_and_retires_rejected_owners() {
    let mut h = Harness::new(MutationKind::Delete(DeleteMode::Permanent));
    for id in 1..=32 {
        h.request.id = OperationId(id);
        h.controller.prepare(h.request.clone()).await;
        assert_eq!(h.preview().await.id, OperationId(id));
    }
    let (sent, received) = std::sync::mpsc::channel();
    h.controller
        .prepare_captured_delete(PrepareCapturedDelete {
            id: OperationId(33),
            source_directory: h.request.source.path.clone(),
            sources: Arc::new(Sources {
                paths: vec![h.request.source.path.join("never-prepared")],
                mode: DeleteMode::Trash,
                dropped: Some(sent),
            }),
        })
        .await;
    let done = h.finished().await;
    assert_eq!(done.id, OperationId(33));
    assert_eq!(done.state, OperationState::Failed);
    assert!(done.message.contains("32 jobs"));
    assert_eq!(h.calls.prepared.load(Ordering::Relaxed), 32);
    assert_ne!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        std::thread::current().id()
    );
    assert!(h.calls.deleted.lock().unwrap().is_empty());
    h.controller.shutdown();
    while h.tasks.join_next().await.is_some() {}
}

#[tokio::test]
async fn captured_tree_delete_rejects_empty_and_oversized_counts_before_preparation() {
    for count in [0, MAX_CAPTURED_DELETE_ROOTS + 1] {
        let mut h = Harness::new(MutationKind::Delete(DeleteMode::Permanent));
        h.controller
            .prepare_captured_delete(PrepareCapturedDelete {
                id: h.request.id,
                source_directory: h.request.source.path.clone(),
                sources: Arc::new(Sources {
                    paths: vec![h.request.source.path.join("must-not-prepare"); count],
                    mode: DeleteMode::Permanent,
                    dropped: None,
                }),
            })
            .await;
        assert_eq!(h.finished().await.state, OperationState::Failed);
        assert_eq!(h.calls.prepared.load(Ordering::Relaxed), 0);
        assert!(h.calls.deleted.lock().unwrap().is_empty());
    }
}
