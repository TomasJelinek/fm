use super::*;
use std::{cell::RefCell, ffi::OsStr, sync::Mutex};

fn fed() -> (Presenter, [PathBuf; 2]) {
    let root = std::env::temp_dir().join("fed-mutation-actions");
    let mut p = Presenter::new(root.clone(), root.join("other"));
    let id = p.create_find(PaneId::Left).unwrap();
    let paths = [root.join("a/one.txt"), root.join("b/two.txt")];
    let session = p.finds.sessions.get_mut(&id).unwrap();
    session.results = paths
        .iter()
        .enumerate()
        .map(|(row, path)| {
            Arc::new(FindResult {
                id: FindResultId(row as u64 + 1),
                entry: FindEntry {
                    path: path.clone(),
                    kind: EntryKind::File,
                    metadata: EntryMetadata {
                        size: Some(12),
                        ..Default::default()
                    },
                    identity: TransferIdentity {
                        device: 1,
                        object: [row as u8; 16],
                    },
                    revision: FindRevision([0; 6]),
                },
                duplicate_group: None,
                duplicate_group_size: 0,
                archive: None,
            })
        })
        .collect();
    p.feed_find_results(id).unwrap();
    p.feed_click(PaneId::Left, 1, true, false);
    p.feed_click(PaneId::Left, 0, false, false);
    (p, paths)
}

struct Pin(MutationItem);
impl PreparedMutation for Pin {
    fn item(&self) -> &MutationItem {
        &self.0
    }
    fn rename(&self, _: &OsStr, _: &Cancellation) -> Result<RenameCompletion, FsError> {
        panic!("preparation must not rename")
    }
    fn delete(
        &self,
        _: DeleteMode,
        _: &Cancellation,
        _: &mut dyn FnMut(&std::path::Path),
    ) -> Result<DeleteCompletion, FsError> {
        panic!("preparation must not delete")
    }
}
#[derive(Default)]
struct Service(Mutex<Vec<PathBuf>>);
impl MutationService for Service {
    fn prepare(
        &self,
        path: &std::path::Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn PreparedMutation>, FsError> {
        self.0.lock().unwrap().push(path.to_owned());
        Ok(Arc::new(Pin(MutationItem {
            path: path.to_owned(),
            kind: EntryKind::File,
            metadata: EntryMetadata {
                size: Some(12),
                ..Default::default()
            },
        })))
    }
}

#[test]
fn rename_captures_fed_focus_instead_of_marks_or_the_hidden_folder() {
    let (mut p, paths) = fed();
    let sent = RefCell::new(Vec::new());
    p.feed_rename(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    let command = sent.into_inner().pop().unwrap();
    let Command::PrepareCapturedRename(request) = command else {
        panic!("captured Rename expected")
    };
    assert_eq!(request.source_directory, paths[0].parent().unwrap());
    let service = Service::default();
    let pinned = request
        .source
        .prepare(&service, &Cancellation::default())
        .unwrap();
    assert_eq!(pinned.item().path, paths[0]);
    assert_eq!(*service.0.lock().unwrap(), [paths[0].clone()]);
    assert_eq!(p.operations.pending, Some(request.id));
}

#[test]
fn metadata_captures_marked_fed_rows_and_opens_the_ordinary_case_editor() {
    let (mut p, paths) = fed();
    let sent = RefCell::new(Vec::new());
    p.open_metadata(PaneId::Left, true, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    let command = sent.into_inner().pop().unwrap();
    let Command::PrepareCapturedMetadata(request) = command else {
        panic!("captured metadata expected")
    };
    let service = Service::default();
    let pins = request
        .sources
        .prepare(&service, &Cancellation::default())
        .unwrap();
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0].item().path, paths[1]);
    assert_eq!(*service.0.lock().unwrap(), [paths[1].clone()]);
    assert_eq!(p.metadata.id, Some(request.id));
    assert!(p.metadata.start_case && p.metadata.busy);
}

#[test]
fn rejected_native_mutation_admission_does_not_claim_a_pending_operation() {
    let (mut p, _) = fed();
    p.feed_rename(PaneId::Left, &|_| false);
    assert!(p.operations.pending.is_none());
    assert!(p.feed(PaneId::Left).unwrap().mutation.is_none());
    p.feed_metadata(PaneId::Left, false, &|_| false);
    assert!(p.operations.pending.is_none());
    assert!(p.metadata.id.is_none());
    assert!(p.feed(PaneId::Left).unwrap().mutation.is_none());
}

#[test]
fn only_completed_changes_make_fed_paths_stale() {
    let (mut p, paths) = fed();
    p.feed_rename(PaneId::Left, &|_| true);
    assert!(p.completed_feed_mutation_paths(PaneId::Left).is_none());
    let progress = Arc::make_mut(p.operations.progress.last_mut().unwrap());
    progress.state = OperationState::Completed;
    progress.completed = 1;
    progress.renamed_to = Some(paths[0].with_file_name("renamed.txt"));
    assert_eq!(
        p.completed_feed_mutation_paths(PaneId::Left),
        Some(vec![paths[0].clone()])
    );
    assert!(p.feed(PaneId::Left).unwrap().mutation.is_none());
}

#[test]
fn unconfirmed_mutation_cancellation_releases_feed_tracking() {
    let (mut p, _) = fed();
    p.feed_rename(PaneId::Left, &|_| true);
    p.feed_metadata(PaneId::Left, false, &|_| {
        panic!("must not replace an active mutation")
    });
    p.operations.progress.clear();
    p.operations.pending = None;
    assert!(p.completed_feed_mutation_paths(PaneId::Left).is_none());
    assert!(p.feed(PaneId::Left).unwrap().mutation.is_none());
    p.feed_metadata(PaneId::Left, false, &|_| true);
    assert!(p.metadata.id.is_some());
}
