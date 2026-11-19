//! Exit waits for live work, then closes every tool window with the main window.
use crate::test_harness::Harness;
use midnight_core::{
    CopyClosureReceipt, CopyEvent, CopyOperationState, CopyProgress, Event, OperationId,
    TransferIntent, UserPreferences,
};
use slint::{ComponentHandle, platform::WindowAdapter};
use std::sync::Arc;

fn copy(state: CopyOperationState) -> Arc<CopyProgress> {
    Arc::new(CopyProgress {
        state,
        closure: (state == CopyOperationState::Cancelled).then_some(CopyClosureReceipt {
            unfinished_roots: 1,
            uncommitted_files: None,
            uncommitted_bytes: None,
            omitted_error_details: 0,
        }),
        ..CopyProgress::queued(
            OperationId(7),
            std::env::temp_dir().join("exit-destination"),
            1,
            TransferIntent::Copy,
        )
    })
}

#[test]
fn exit_with_live_jobs_closes_the_jobs_window_once_work_ends() {
    let mut h = Harness::new(UserPreferences::default());
    h.presenter.borrow_mut().copies.progress = vec![copy(CopyOperationState::Running)];
    h.refresh();
    h.window.invoke_exit();
    h.settle();
    assert!(h.window.window().is_visible(), "exit waits for live work");
    assert!(
        h.window.get_operations_visible(),
        "exit shows the live jobs"
    );
    let visible = |h: &Harness| {
        h.children
            .borrow()
            .iter()
            .filter(|window| window.window().is_visible())
            .count()
    };
    assert_eq!(visible(&h), 1, "the Jobs window is open while work ends");

    h.presenter
        .borrow_mut()
        .apply(Event::Copy(CopyEvent::Progress(copy(
            CopyOperationState::Cancelled,
        ))));
    h.refresh();
    assert!(
        !h.window.window().is_visible(),
        "work ended: main window hides"
    );
    assert_eq!(
        visible(&h),
        0,
        "no tool window may outlive the hidden main window and keep the app running"
    );
}
