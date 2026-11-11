use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn loaded() -> Presenter {
    let (mut p, root) = super::tree::fixture();
    let mut publication = super::tree::publication(root, 1, true, false);
    publication.selection =
        Arc::new(TreeSelectionSnapshot::from_bits(publication.view.clone(), vec![2]).unwrap());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(publication))),
        &|_| true,
    );
    p
}
fn request(p: &mut Presenter) -> TreeChecksumRequest {
    let sent = RefCell::new(Vec::new());
    p.request_checksums(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    sent.into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::Tree(command) => match *command {
                TreeCommand::Checksums(request) => Some(request),
                _ => None,
            },
            _ => None,
        })
        .expect("checksums must capture tree occurrences")
}
#[derive(Debug)]
struct Source {
    stamp: TreeViewStamp,
    request: TreeRequestId,
    drops: Arc<AtomicUsize>,
}
impl CapturedChecksumFiles for Source {
    fn stamp(&self) -> TreeViewStamp {
        self.stamp
    }
    fn request(&self) -> TreeRequestId {
        self.request
    }
    fn resolve(&self, _: &Cancellation) -> Result<Vec<ChecksumTarget>, FsError> {
        panic!("UI must never resolve checksum provider paths")
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn receipt(
    request: &TreeChecksumRequest,
    fallback: Option<PathBuf>,
    drops: Arc<AtomicUsize>,
) -> Event {
    Event::Tree(Box::new(TreeEvent::ChecksumsCaptured {
        stamp: request.view.stamp,
        request: request.request,
        result: Ok(TreeChecksumCapture {
            sources: Arc::new(Source {
                stamp: request.view.stamp,
                request: request.request,
                drops,
            }),
            directory: test_root().join("backend-authorized"),
            fallback,
        }),
    }))
}
fn reclaim(p: &mut Presenter, drops: &AtomicUsize) {
    assert_eq!(drops.load(Ordering::SeqCst), 0, "provider dropped on UI");
    let retired = p.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn tree_checksums_selected_folder_uses_origin_panel_and_recalculates_same_capture() {
    let mut p = loaded();
    let request = request(&mut p);
    assert!(request.selection.contains_position(1));
    p.active = PaneId::Right;
    let drops = Arc::new(AtomicUsize::new(0));
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(receipt(&request, None, drops.clone()), &|command| {
        sent.borrow_mut().push(command);
        true
    });
    let first = sent
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::StartCapturedChecksums(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(first.sources.stamp().pane, PaneId::Left);
    assert!(!first.recursive);
    assert!(!p.checksums.sessions[&first.job].verify_initial);
    assert_eq!(
        p.checksums.sessions[&first.job].directory,
        test_root().join("backend-authorized")
    );
    p.clear_tree(PaneId::Left, Some(NavigationId(99)));
    assert!(
        !request.cancel.is_cancelled(),
        "opened window survives navigation"
    );
    p.checksums.sessions.get_mut(&first.job).unwrap().running = false;
    sent.borrow_mut().clear();
    assert!(
        p.start_checksums(first.job, first.algorithms, true, &|command| {
            sent.borrow_mut().push(command);
            true
        })
    );
    assert!(sent.borrow().iter().any(|command| matches!(command, Command::StartCapturedChecksums(next) if next.generation.0 == first.generation.0 + 1 && next.recursive && Arc::ptr_eq(&next.sources, &first.sources))));
    sent.borrow_mut().clear();
    let id = first.job;
    drop(first);
    assert!(p.close_checksums(id, &|_| true));
    reclaim(&mut p, &drops);
}

#[test]
fn tree_checksums_retries_capture_queue_and_retains_failed_calculation_for_explicit_retry() {
    let mut p = loaded();
    p.request_checksums(PaneId::Left, &|_| false);
    assert!(p.checksums.tree.is_some());
    let sent = RefCell::new(Vec::new());
    p.flush_tree_checksums(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    let Command::Tree(command) = sent.borrow_mut().remove(0) else {
        panic!()
    };
    let TreeCommand::Checksums(request) = *command else {
        panic!()
    };
    let drops = Arc::new(AtomicUsize::new(0));
    p.apply_with_sender(receipt(&request, None, drops.clone()), &|_| false);
    let id = *p.checksums.sessions.keys().next().unwrap();
    assert!(!p.checksums.sessions[&id].running);
    assert!(p.checksums.sessions[&id].error.contains("queue is busy"));
    assert!(p.start_checksums(id, ChecksumAlgorithms::default(), false, &|_| true));
    assert!(p.close_checksums(id, &|_| true));
    reclaim(&mut p, &drops);
}

#[test]
fn tree_checksums_navigation_reprojection_and_shutdown_reject_late_receipts() {
    for cause in 0..3 {
        let mut p = loaded();
        let request = request(&mut p);
        match cause {
            0 => p.clear_tree(PaneId::Left, Some(NavigationId(99))),
            1 => {
                let publication =
                    super::tree::publication(request.view.root.clone(), 2, true, false);
                p.apply_with_sender(
                    Event::Tree(Box::new(TreeEvent::Published(publication))),
                    &|_| true,
                );
            }
            _ => p.request_operation_shutdown(&|_| true),
        }
        let drops = Arc::new(AtomicUsize::new(0));
        p.apply_with_sender(receipt(&request, None, drops.clone()), &|command| {
            assert!(!matches!(command, Command::StartCapturedChecksums(_)));
            true
        });
        assert!(request.cancel.is_cancelled());
        assert!(p.checksums.sessions.is_empty());
        reclaim(&mut p, &drops);
    }
}

#[test]
fn tree_checksums_sidecar_fallback_prefills_verification_without_starting_it() {
    let mut p = loaded();
    let request = request(&mut p);
    let manifest = test_root().join("backend-file.sha256");
    let drops = Arc::new(AtomicUsize::new(0));
    p.apply_with_sender(
        receipt(&request, Some(manifest.clone()), drops.clone()),
        &|command| {
            assert!(!matches!(
                command,
                Command::StartCapturedChecksums(_) | Command::VerifyChecksumManifest(_)
            ));
            true
        },
    );
    let id = *p.checksums.sessions.keys().next().unwrap();
    assert!(p.checksums.sessions[&id].verify_initial);
    assert_eq!(p.checksums.sessions[&id].manifest_suggested, manifest);
    p.close_checksums(id, &|_| true);
    reclaim(&mut p, &drops);
}

#[test]
fn tree_checksums_waits_for_selection_receipt_without_changing_fallback() {
    let mut p = loaded();
    p.panes[0].cursor = Some(EntryId(11));
    p.panes[0].parent_cursor = false;
    assert!(p.send_tree_selection(
        PaneId::Left,
        TreeSelectionAction::Toggle(TreeRowId(11)),
        &|_| true
    ));
    let revision = p.panes[0].tree.revision;
    let view = p.panes[0].tree_view().unwrap().clone();
    p.request_checksums(PaneId::Left, &|_| {
        panic!("capture before selection receipt")
    });
    p.panes[0].cursor = Some(EntryId(7));
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::SelectionChanged {
            view,
            revision,
            selection: selection.clone(),
        })),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    assert!(sent.borrow().iter().any(|command| matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Checksums(request) if request.selection_revision == revision && Arc::ptr_eq(&request.selection, &selection) && request.fallback == Some(TreeRowId(11))))));
}

#[test]
fn tree_checksums_stale_and_duplicate_receipts_cannot_open_windows() {
    let mut p = loaded();
    let request = request(&mut p);
    let stale_drops = Arc::new(AtomicUsize::new(0));
    let mut stale = receipt(&request, None, stale_drops.clone());
    if let Event::Tree(event) = &mut stale
        && let TreeEvent::ChecksumsCaptured { request, .. } = event.as_mut()
    {
        request.0 += 1;
    }
    p.apply_with_sender(stale, &|command| {
        assert!(!matches!(command, Command::StartCapturedChecksums(_)));
        true
    });
    assert!(p.checksums.sessions.is_empty());
    assert!(p.checksums.tree.is_some());
    reclaim(&mut p, &stale_drops);
    let first_drops = Arc::new(AtomicUsize::new(0));
    p.apply_with_sender(receipt(&request, None, first_drops.clone()), &|_| true);
    assert_eq!(p.checksums.sessions.len(), 1);
    let duplicate_drops = Arc::new(AtomicUsize::new(0));
    p.apply_with_sender(
        receipt(&request, None, duplicate_drops.clone()),
        &|command| {
            assert!(!matches!(command, Command::StartCapturedChecksums(_)));
            true
        },
    );
    assert_eq!(p.checksums.sessions.len(), 1);
    reclaim(&mut p, &duplicate_drops);
    let id = *p.checksums.sessions.keys().next().unwrap();
    p.close_checksums(id, &|_| true);
    reclaim(&mut p, &first_drops);
}
