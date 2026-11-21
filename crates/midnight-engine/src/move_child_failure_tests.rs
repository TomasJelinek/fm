//! A child that a Move cannot prepare for Move pauses alone (stage Moving the source):
//! Skip leaves out only that child, which stays in its folder with the folders above
//! it, and the rest of the folder is moved; Retry prepares the child again. The Move
//! goes through the queued preparation (`SharedSource`), as a user's Move does.
use super::*;
use std::{collections::HashMap, ffi::OsString};

/// The native provider on another volume (`CrossDeviceService`), whose selected folder
/// refuses to prepare the named children for Move, each as many times as given.
pub(in super::super) struct Refusing {
    native: Arc<CrossDeviceService>,
    refusals: Arc<Mutex<HashMap<OsString, usize>>>,
}

impl Refusing {
    pub(in super::super) fn new(refusals: &[(&str, usize)]) -> Self {
        Self {
            native: Arc::new(CrossDeviceService(midnight_platform::LocalFileSystem::new(
                None,
            ))),
            refusals: Arc::new(Mutex::new(
                refusals
                    .iter()
                    .map(|(name, count)| (OsString::from(name), *count))
                    .collect(),
            )),
        }
    }
}

// This native-delegating fixture retains only synchronous authority and its refusals.
impl midnight_core::AuditedSynchronousTransferService for Refusing {}
impl TransferService for Refusing {
    fn prepare_transfer_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.native.prepare_transfer_source(path, cancel)
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.native.prepare_transfer_destination(path, cancel)
    }
    fn prepare_move_destination(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.native.prepare_move_destination(path, cancel)
    }
    fn prepare_move_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<PreparedTransferMove, FsError> {
        let prepared = self.native.prepare_move_source(path, cancel)?;
        Ok(PreparedTransferMove {
            source: Box::new(RefusingFolder {
                inner: prepared.source,
                refusals: self.refusals.clone(),
                held: Mutex::default(),
            }),
            capture: prepared.capture,
        })
    }
}

struct RefusingFolder {
    inner: Box<dyn TransferSource>,
    refusals: Arc<Mutex<HashMap<OsString, usize>>>,
    /// The children this folder prepared and then refused, which Retry hands back once
    /// their refusals run out, as a provider prepares a child of its own folder again.
    held: Mutex<HashMap<OsString, PreparedTransferMoveEntry>>,
}

impl RefusingFolder {
    /// The refusal of the child `name`, while it has one left.
    fn refusal(&self, name: &OsStr) -> Option<FsError> {
        let mut refusals = self.refusals.lock().unwrap();
        let left = refusals.get_mut(name).filter(|left| **left > 0)?;
        *left -= 1;
        Some(FsError::from_io(
            "move",
            &self.inner.entry().item.path.join(name),
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "this child cannot be moved",
            ),
        ))
    }
}

impl TransferSource for RefusingFolder {
    fn entry(&self) -> &TransferEntry {
        self.inner.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn discover(
        &self,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(&MutationItem),
    ) -> Result<(), FsError> {
        self.inner.discover(cancel, visit)
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.inner.read_chunk(buffer, cancel)
    }
    fn next_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        self.inner.next_child(cancel)
    }
    fn next_move_child(
        &mut self,
        cancel: &Cancellation,
    ) -> Result<Option<PreparedTransferMove>, FsError> {
        self.inner.next_move_child(cancel)
    }
    fn next_move_child_or_failure(
        &mut self,
        links: CopyLinkPolicy,
        cancel: &Cancellation,
    ) -> Result<Option<TransferMoveChild>, FsError> {
        let child = self.inner.next_move_child_or_failure(links, cancel)?;
        Ok(child.map(|child| {
            child.and_then(|entry| {
                let path = &entry.source().entry().item.path;
                let name = path.file_name().expect("a named child").to_owned();
                match self.refusal(&name) {
                    Some(error) => {
                        self.held.lock().unwrap().insert(name.clone(), entry);
                        Err(TransferChildFailure { name, error })
                    }
                    None => Ok(entry),
                }
            })
        }))
    }
    fn prepare_move_child(
        &self,
        name: &OsStr,
        _links: CopyLinkPolicy,
        _cancel: &Cancellation,
    ) -> Result<PreparedTransferMoveEntry, FsError> {
        if let Some(error) = self.refusal(name) {
            return Err(error);
        }
        Ok(self
            .held
            .lock()
            .unwrap()
            .remove(name)
            .expect("a refused child is prepared again"))
    }
}

/// A selected folder with files, a folder and `refused.txt`.
fn tree(root: &Path) -> PathBuf {
    let tree = root.join("tree");
    fs::create_dir_all(tree.join("folder")).unwrap();
    fs::write(tree.join("a.txt"), b"a").unwrap();
    fs::write(tree.join("refused.txt"), b"refused").unwrap();
    fs::write(tree.join("folder/inner.txt"), b"inner").unwrap();
    tree
}

pub(in super::super) fn move_request(source: &Path, destination: &Path) -> ClipboardMoveRequest {
    ClipboardMoveRequest {
        start_policy: midnight_core::TransferStartPolicy::Immediate,
        verification: TransferVerification::default(),
        id: OperationId(1),
        clipboard: Arc::new(FileClipboard {
            paths: vec![source.to_owned()],
            intent: FileClipboardIntent::Move,
        }),
        destination: destination.to_owned(),
    }
}

pub(in super::super) async fn failure(h: &mut Harness) -> Arc<CopyFailure> {
    loop {
        match h.next().await {
            CopyEvent::Failure(failure) => return failure,
            CopyEvent::Progress(progress) => {
                assert!(
                    !progress.state.is_terminal(),
                    "expected a failure: {progress:?}"
                );
            }
            _ => {}
        }
    }
}

/// The job's end, failing on any further question.
pub(in super::super) async fn finished(h: &mut Harness) -> Arc<CopyProgress> {
    loop {
        match h.next().await {
            CopyEvent::Failure(failure) => panic!("asked again: {failure:?}"),
            CopyEvent::Progress(progress) if progress.state.is_terminal() => return progress,
            _ => {}
        }
    }
}

pub(in super::super) async fn decide(h: &Harness, failure: &CopyFailure, choice: CopyChoice) {
    h.controller
        .decide(CopyDecision {
            id: failure.id,
            version: failure.version,
            choice,
        })
        .await;
}

fn names(folder: &Path) -> Vec<OsString> {
    let mut names: Vec<_> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn skip_leaves_out_only_the_child_that_cannot_be_prepared_and_moves_the_rest() {
    // A private parent, as Windows requires of a Move source's folder.
    let (_temp, parent) = protected_fixture();
    let tree = tree(&parent);
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    let mut h = Harness::new(Some(Arc::new(Refusing::new(&[("refused.txt", 1)]))));
    h.controller
        .enqueue_clipboard_move(move_request(&tree, &destination))
        .await;
    let failure = failure(&mut h).await;
    assert_eq!(failure.stage, CopyErrorStage::MovingSource, "{failure:?}");
    assert_eq!(failure.source, Some(tree.join("refused.txt")));
    assert_eq!(failure.destination, destination.join("tree/refused.txt"));
    assert!(failure.can_retry && failure.can_skip, "{failure:?}");
    decide(&h, &failure, CopyChoice::Skip).await;
    let done = finished(&mut h).await;
    assert_eq!(
        done.state,
        CopyOperationState::CompletedWithErrors,
        "{done:?}"
    );
    assert_eq!((done.skipped, done.failed), (1, 1), "{done:?}");
    // The skipped child stays where it was, in its folder; the rest moved.
    assert_eq!(names(&tree), ["refused.txt"]);
    assert_eq!(fs::read(tree.join("refused.txt")).unwrap(), b"refused");
    let moved = destination.join("tree");
    assert_eq!(names(&moved), ["a.txt", "folder"]);
    assert_eq!(fs::read(moved.join("folder/inner.txt")).unwrap(), b"inner");
    h.close().await;
}

#[tokio::test]
async fn retry_prepares_the_child_again_and_moves_it() {
    // A private parent, as Windows requires of a Move source's folder.
    let (_temp, parent) = protected_fixture();
    let tree = tree(&parent);
    let destination = parent.join("destination");
    fs::create_dir(&destination).unwrap();
    let mut h = Harness::new(Some(Arc::new(Refusing::new(&[("refused.txt", 2)]))));
    h.controller
        .enqueue_clipboard_move(move_request(&tree, &destination))
        .await;
    // The first Retry is refused again, and the child is asked about again.
    for _ in 0..2 {
        let failure = failure(&mut h).await;
        assert_eq!(failure.source, Some(tree.join("refused.txt")));
        assert!(failure.can_retry, "{failure:?}");
        decide(&h, &failure, CopyChoice::Retry).await;
    }
    let done = finished(&mut h).await;
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!((done.skipped, done.failed), (0, 0), "{done:?}");
    assert!(!tree.exists());
    let moved = destination.join("tree");
    assert_eq!(names(&moved), ["a.txt", "folder", "refused.txt"]);
    assert_eq!(fs::read(moved.join("refused.txt")).unwrap(), b"refused");
    h.close().await;
}
