use super::*;
use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct State {
    begun: AtomicUsize,
    finished: AtomicUsize,
    abandoned: AtomicUsize,
    log: Mutex<Vec<&'static str>>,
    fail_refresh: bool,
}
struct Changes(Arc<State>);
struct Change {
    state: Arc<State>,
    finished: bool,
}
impl TransferNamespaceChanges for Changes {
    fn begin(&self, _: &Cancellation) -> Result<Box<dyn TransferNamespaceChange>, FsError> {
        self.0.begun.fetch_add(1, Ordering::SeqCst);
        self.0.log.lock().unwrap().push("begin");
        Ok(Box::new(Change {
            state: self.0.clone(),
            finished: false,
        }))
    }
}
impl TransferNamespaceChange for Change {
    fn reserve_private_child(
        &mut self,
        _: Option<Arc<dyn TransferNamespacePin>>,
        _: &Cancellation,
    ) -> Result<bool, FsError> {
        self.state.log.lock().unwrap().push("reserve");
        Ok(false)
    }
    fn finish(mut self: Box<Self>) -> Result<(), FsError> {
        self.state.log.lock().unwrap().push("finish");
        self.state.finished.fetch_add(1, Ordering::SeqCst);
        self.finished = true;
        if self.state.fail_refresh {
            Err(failure())
        } else {
            Ok(())
        }
    }
}
impl Drop for Change {
    fn drop(&mut self) {
        if !self.finished {
            self.state.abandoned.fetch_add(1, Ordering::SeqCst);
        }
    }
}
fn failure() -> FsError {
    FsError::from_io(
        "namespace fixture",
        std::path::Path::new(""),
        io::ErrorKind::Interrupted.into(),
    )
}

#[test]
fn legacy_without_context_keeps_the_native_result_and_has_no_transaction() {
    let result = with_transfer_namespace_change(&Cancellation::default(), |change| {
        assert!(change.is_none());
        Ok(17)
    });
    assert_eq!(result.operation.unwrap(), 17);
    result.refresh.unwrap();
}

#[test]
fn ordinary_success_failure_and_cancellation_all_finish_the_transaction() {
    let state = Arc::new(State::default());
    let adapter: Arc<dyn TransferNamespaceChanges> = Arc::new(Changes(state.clone()));
    with_transfer_namespace_changes(adapter, || {
        let cancel = Cancellation::default();
        let success = with_transfer_namespace_change(&cancel, |_| Ok(23));
        assert_eq!(success.operation.unwrap(), 23);
        success.refresh.unwrap();
        let failed = with_transfer_namespace_change::<()>(&cancel, |_| Err(failure()));
        failed.refresh.unwrap();
        let error = failed.operation.unwrap_err();
        assert_eq!(error.kind, io::ErrorKind::Interrupted);
        let cancellation = cancel.clone();
        let cancelled = with_transfer_namespace_change::<()>(&cancel, |_| {
            cancellation.cancel();
            Err(failure())
        });
        cancelled.refresh.unwrap();
        let error = cancelled.operation.unwrap_err();
        assert_eq!(error.kind, io::ErrorKind::Interrupted);
    });
    assert_eq!(state.begun.load(Ordering::SeqCst), 3);
    assert_eq!(state.finished.load(Ordering::SeqCst), 3);
    assert_eq!(state.abandoned.load(Ordering::SeqCst), 0);
    assert_eq!(
        *state.log.lock().unwrap(),
        ["begin", "finish", "begin", "finish", "begin", "finish"]
    );
}

#[test]
fn nested_helpers_reborrow_one_change_and_private_name_conflicts_still_finish() {
    fn native_helper(
        change: Option<&mut dyn TransferNamespaceChange>,
        cancel: &Cancellation,
    ) -> Result<bool, FsError> {
        change.unwrap().reserve_private_child(None, cancel)
    }
    let state = Arc::new(State::default());
    let adapter: Arc<dyn TransferNamespaceChanges> = Arc::new(Changes(state.clone()));
    with_transfer_namespace_changes(adapter, || {
        let cancel = Cancellation::default();
        let result =
            with_transfer_namespace_change(&cancel, |change| native_helper(change, &cancel));
        assert!(!result.operation.unwrap());
        result.refresh.unwrap();
    });
    assert_eq!(state.begun.load(Ordering::SeqCst), 1);
    assert_eq!(state.finished.load(Ordering::SeqCst), 1);
    assert_eq!(*state.log.lock().unwrap(), ["begin", "reserve", "finish"]);
}

#[test]
fn nested_tls_context_is_restored_and_unwind_leaves_transaction_uncommitted() {
    let outer = Arc::new(State::default());
    let inner = Arc::new(State::default());
    let adapter: Arc<dyn TransferNamespaceChanges> = Arc::new(Changes(outer.clone()));
    with_transfer_namespace_changes(adapter, || {
        let nested: Arc<dyn TransferNamespaceChanges> = Arc::new(Changes(inner.clone()));
        let panic = catch_unwind(AssertUnwindSafe(|| {
            with_transfer_namespace_changes(nested, || {
                let _ = with_transfer_namespace_change::<()>(&Cancellation::default(), |_| {
                    panic!("native unwind")
                });
            })
        }));
        assert!(panic.is_err());
        let result = with_transfer_namespace_change(&Cancellation::default(), |_| Ok(()));
        result.operation.unwrap();
        result.refresh.unwrap();
    });
    assert!(
        begin_transfer_namespace_change(&Cancellation::default())
            .unwrap()
            .is_none()
    );
    assert_eq!(outer.begun.load(Ordering::SeqCst), 1);
    assert_eq!(outer.finished.load(Ordering::SeqCst), 1);
    assert_eq!(inner.begun.load(Ordering::SeqCst), 1);
    assert_eq!(inner.finished.load(Ordering::SeqCst), 0);
    assert_eq!(inner.abandoned.load(Ordering::SeqCst), 1);
}

#[test]
fn refresh_failure_preserves_a_successful_native_commit_and_is_returned_separately() {
    let state = Arc::new(State {
        fail_refresh: true,
        ..State::default()
    });
    let adapter: Arc<dyn TransferNamespaceChanges> = Arc::new(Changes(state.clone()));
    with_transfer_namespace_changes(adapter.clone(), || {
        assert!(Arc::ptr_eq(
            &current_transfer_namespace_changes().unwrap(),
            &adapter
        ));
        let result = with_transfer_namespace_change(&Cancellation::default(), |_| Ok("published"));
        assert_eq!(result.operation.unwrap(), "published");
        assert_eq!(result.refresh.unwrap_err().kind, io::ErrorKind::Interrupted);
        let result = with_transfer_namespace_change::<()>(&Cancellation::default(), |_| {
            Err(FsError::from_io(
                "native refusal",
                std::path::Path::new(""),
                io::ErrorKind::PermissionDenied.into(),
            ))
        });
        assert_eq!(
            result.operation.unwrap_err().kind,
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(result.refresh.unwrap_err().kind, io::ErrorKind::Interrupted);
    });
    assert!(current_transfer_namespace_changes().is_none());
    assert_eq!(state.finished.load(Ordering::SeqCst), 2);
    assert_eq!(state.abandoned.load(Ordering::SeqCst), 0);
}

#[test]
fn refused_begin_does_not_run_the_native_operation() {
    struct Refused;
    impl TransferNamespaceChanges for Refused {
        fn begin(&self, _: &Cancellation) -> Result<Box<dyn TransferNamespaceChange>, FsError> {
            Err(failure())
        }
    }
    with_transfer_namespace_changes(Arc::new(Refused), || {
        let result = with_transfer_namespace_change::<()>(&Cancellation::default(), |_| {
            panic!("a refused mutation turn must not execute native code")
        });
        assert_eq!(
            result.operation.unwrap_err().kind,
            io::ErrorKind::Interrupted
        );
        result.refresh.unwrap();
    });
}
