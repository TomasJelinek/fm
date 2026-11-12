//! Native focused Rename capture uses the session's real acknowledged provider boundary.
use super::*;
use crate::tree_rename::Retirement;
use midnight_core::{
    CapturedRenameTarget, FsError, MutationService, PreparedMutation, TreeRenameRequest,
};
use std::{io, path::Path};

fn request(session: &mut Session, row: TreeRowId) -> TreeRenameRequest {
    let view = session.publication().unwrap().view;
    assert!(session.commit(&view));
    TreeRenameRequest {
        view,
        request: TreeRequestId(session.last_request.0 + 1),
        row,
        cancel: Cancellation::default(),
    }
}
#[derive(Default)]
struct Record(Mutex<Vec<PathBuf>>);
impl MutationService for Record {
    fn prepare(&self, path: &Path, _: &Cancellation) -> Result<Arc<dyn PreparedMutation>, FsError> {
        self.0.lock().unwrap().push(path.into());
        Err(FsError::from_io(
            "captured path observed",
            path,
            io::ErrorKind::Unsupported.into(),
        ))
    }
}
fn capture(
    f: &mut Fixture,
    request: &TreeRenameRequest,
    capacity: &Arc<Semaphore>,
) -> Arc<dyn CapturedRenameTarget> {
    f.session
        .capture_rename(
            request,
            capacity,
            &Retirement::testing(),
            &Cancellation::default(),
        )
        .unwrap()
}

#[test]
fn tree_rename_capture_uses_focused_descendant_even_when_another_directory_is_marked() {
    let mut f = fixture();
    let parent = f
        .session
        .projection()
        .rows
        .iter()
        .find(|row| row.entry.display_name == "z-parent")
        .unwrap()
        .id;
    super::super::action_capture_tests::expand(&mut f.session, parent);
    super::super::action_capture_tests::select(
        &mut f.session,
        midnight_core::TreeSelectionAction::Toggle(parent),
    );
    let child = f
        .session
        .projection()
        .rows
        .iter()
        .find(|row| row.depth == 1)
        .unwrap()
        .id;
    let request = request(&mut f.session, child);
    let captured = capture(&mut f, &request, &Arc::new(Semaphore::new(1)));
    assert_eq!(captured.stamp(), request.view.stamp);
    assert_eq!(captured.request(), request.request);
    assert_eq!(captured.row(), child);
    let service = Record::default();
    assert!(
        captured
            .prepare(&service, &Cancellation::default())
            .is_err()
    );
    assert_eq!(
        *service.0.lock().unwrap(),
        [f.graph.root_path.join("z-parent/child")]
    );
}

#[test]
fn tree_rename_capture_accepts_native_files_directories_and_links_without_using_captions() {
    for kind in [
        EntryKind::File,
        EntryKind::Directory,
        EntryKind::DirectoryLink,
        EntryKind::Symlink,
        EntryKind::Other,
    ] {
        let mut f = super::editor_capture_tests::one_root(kind);
        let row = f.session.projection().rows[0].id;
        let request = request(&mut f.session, row);
        let captured = capture(&mut f, &request, &Arc::new(Semaphore::new(1)));
        let service = Record::default();
        assert!(
            captured
                .prepare(&service, &Cancellation::default())
                .is_err()
        );
        assert_eq!(
            *service.0.lock().unwrap(),
            [f.graph.root_path.join("file.txt")],
            "{kind:?}"
        );
    }
}

#[test]
fn tree_rename_capture_rejects_unacknowledged_forged_parent_replayed_and_cancelled_requests() {
    let mut f = fixture();
    let view = f.session.projection().clone();
    let row = view.rows[0].id;
    let mut request = TreeRenameRequest {
        view,
        row,
        request: TreeRequestId(1),
        cancel: Cancellation::default(),
    };
    let capacity = Arc::new(Semaphore::new(1));
    assert!(
        f.session
            .capture_rename(
                &request,
                &capacity,
                &Retirement::testing(),
                &Cancellation::default()
            )
            .is_err()
    );
    assert!(f.session.commit(&request.view));
    let exact = request.view.clone();
    request.view = Arc::new((*exact).clone());
    assert!(
        f.session
            .capture_rename(
                &request,
                &capacity,
                &Retirement::testing(),
                &Cancellation::default()
            )
            .is_err()
    );
    request.view = exact;
    request.row = TreeRowId(0);
    assert!(
        f.session
            .capture_rename(
                &request,
                &capacity,
                &Retirement::testing(),
                &Cancellation::default()
            )
            .is_err()
    );
    request.row = row;
    request.cancel.cancel();
    assert!(
        f.session
            .capture_rename(
                &request,
                &capacity,
                &Retirement::testing(),
                &Cancellation::default()
            )
            .is_err()
    );
    assert_eq!(capacity.available_permits(), 1);
    request.cancel = Cancellation::default();
    let source = capture(&mut f, &request, &capacity);
    assert!(
        f.session
            .capture_rename(
                &request,
                &capacity,
                &Retirement::testing(),
                &Cancellation::default()
            )
            .is_err()
    );
    drop(source);
    wait_released(&capacity);
}

#[test]
fn tree_rename_capture_retains_provider_and_shared_capacity_until_last_owner() {
    let mut f = fixture();
    let row = f.session.projection().rows[0].id;
    let capacity = Arc::new(Semaphore::new(1));
    let first = request(&mut f.session, row);
    let source = capture(&mut f, &first, &capacity);
    let second = request(&mut f.session, row);
    assert!(matches!(
        f.session.capture_rename(
            &second,
            &capacity,
            &Retirement::testing(),
            &Cancellation::default()
        ),
        Err(ProviderError::ResourceLimited)
    ));
    let retained = source.clone();
    drop(source);
    drop(f.session);
    assert_eq!(capacity.available_permits(), 0);
    let service = Record::default();
    assert!(
        retained
            .prepare(&service, &Cancellation::default())
            .is_err()
    );
    assert_eq!(
        service.0.lock().unwrap().len(),
        1,
        "retained provider remains usable"
    );
    drop(retained);
    wait_released(&capacity);
}

fn wait_released(capacity: &Semaphore) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while capacity.available_permits() == 0 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(capacity.available_permits(), 1);
}

#[path = "tree_rename_retirement_tests.rs"]
mod retirement;
