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

#[test]
fn copy_selected_nested_directory_requests_backend_capture() {
    let mut p = loaded();
    let sent = RefCell::new(Vec::new());
    p.copy_files(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, Command::Tree(_))),
        "Ctrl+C must capture the nested occurrence rather than reject the tree"
    );
}

#[test]
fn cut_selected_nested_directory_requests_backend_capture() {
    let mut p = loaded();
    let sent = RefCell::new(Vec::new());
    p.cut_files(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(
        sent.borrow()
            .iter()
            .any(|command| matches!(command, Command::Tree(_))),
        "Ctrl+X must capture the nested occurrence rather than reject the tree"
    );
}

#[test]
fn native_tree_paste_uses_the_panel_root() {
    let mut p = loaded();
    let destination = p.panes[0].display_path().to_owned();
    let sent = RefCell::new(Vec::new());
    p.paste_files(PaneId::Left, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    assert!(matches!(&sent.borrow()[0], Command::ReadFileClipboard {
        destination: actual, ..
    } if actual == &destination));
}

fn request(p: &mut Presenter, intent: FileClipboardIntent) -> TreeClipboardRequest {
    let sent = RefCell::new(Vec::new());
    p.request_tree_clipboard(PaneId::Left, intent, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    sent.into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::Tree(command) => match *command {
                TreeCommand::Clipboard(request) => Some(request),
                _ => None,
            },
            _ => None,
        })
        .expect("clipboard must capture exact occurrences")
}

#[derive(Debug)]
struct Source {
    intent: FileClipboardIntent,
    drops: Arc<AtomicUsize>,
}

impl CapturedClipboardFiles for Source {
    fn intent(&self) -> FileClipboardIntent {
        self.intent
    }
    fn resolve(&self, _: &Cancellation) -> Result<FileClipboard, FsError> {
        panic!("clipboard sources must not resolve on the UI thread")
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn captured(
    request: &TreeClipboardRequest,
    intent: FileClipboardIntent,
    drops: Arc<AtomicUsize>,
) -> Event {
    Event::Tree(Box::new(TreeEvent::Clipboard {
        stamp: request.view.stamp,
        request: request.request,
        result: Ok(Arc::new(Source { intent, drops })),
    }))
}

fn reclaim(p: &mut Presenter, drops: &AtomicUsize) {
    assert_eq!(
        drops.load(Ordering::SeqCst),
        0,
        "UI released provider ownership"
    );
    let retired = p.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn clipboard_capture_preserves_intent_selection_and_retires_after_completion() {
    for intent in [FileClipboardIntent::Copy, FileClipboardIntent::Move] {
        let mut p = loaded();
        let request = request(&mut p, intent);
        assert_eq!(request.intent, intent);
        assert!(request.selection.contains_position(1));
        assert!(!request.selection.contains_position(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let sent = RefCell::new(Vec::new());
        p.apply_with_sender(captured(&request, intent, drops.clone()), &|command| {
            sent.borrow_mut().push(command);
            true
        });
        let desktop_request = sent
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::CopyCapturedFiles {
                    request,
                    source,
                    cancel,
                } => {
                    assert_eq!(source.intent(), intent);
                    assert!(!cancel.is_cancelled());
                    Some(*request)
                }
                _ => None,
            })
            .expect("backend clipboard publication command");
        sent.borrow_mut().clear();
        p.apply_with_sender(
            Event::DesktopActionFinished {
                request: desktop_request,
                result: Ok(DesktopActionResult::ClipboardCopied),
            },
            &|_| true,
        );
        assert!(p.file_actions.tree_clipboard.is_none());
        assert!(
            !request.cancel.is_cancelled(),
            "completed clipboard survives navigation"
        );
        reclaim(&mut p, &drops);
    }
}

#[test]
fn clipboard_navigation_cancels_capture_and_retires_late_reply() {
    let mut p = loaded();
    let request = request(&mut p, FileClipboardIntent::Copy);
    p.clear_tree(PaneId::Left, Some(NavigationId(99)));
    assert!(request.cancel.is_cancelled());
    let drops = Arc::new(AtomicUsize::new(0));
    p.apply_with_sender(
        captured(&request, request.intent, drops.clone()),
        &|command| {
            assert!(!matches!(command, Command::CopyCapturedFiles { .. }));
            true
        },
    );
    reclaim(&mut p, &drops);
}

#[test]
fn refused_publication_retries_same_source_and_shutdown_cancels_it() {
    let mut p = loaded();
    let request = request(&mut p, FileClipboardIntent::Move);
    let drops = Arc::new(AtomicUsize::new(0));
    p.apply_with_sender(captured(&request, request.intent, drops.clone()), &|_| {
        false
    });
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let sent = RefCell::new(None);
    p.flush_tree_clipboard(&|command| {
        *sent.borrow_mut() = Some(command);
        true
    });
    assert!(
        matches!(&*sent.borrow(), Some(Command::CopyCapturedFiles { source, .. })
        if source.intent() == FileClipboardIntent::Move)
    );
    sent.borrow_mut().take();
    p.retire_all();
    assert!(request.cancel.is_cancelled());
    reclaim(&mut p, &drops);
}

#[test]
fn clipboard_rejects_changed_intent_without_native_publication() {
    let mut p = loaded();
    let request = request(&mut p, FileClipboardIntent::Copy);
    let drops = Arc::new(AtomicUsize::new(0));
    p.apply_with_sender(
        captured(&request, FileClipboardIntent::Move, drops.clone()),
        &|command| {
            assert!(!matches!(command, Command::CopyCapturedFiles { .. }));
            true
        },
    );
    assert!(p.file_actions.tree_clipboard.is_none());
    assert!(request.cancel.is_cancelled());
    reclaim(&mut p, &drops);
}

#[test]
fn clipboard_waits_for_selection_receipt_and_uses_focused_fallback() {
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
    p.copy_files(PaneId::Left, &|_| {
        panic!("clipboard must wait for selection")
    });
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
    assert!(sent.borrow().iter().any(|command| matches!(command,
        Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Clipboard(request)
            if request.selection_revision == revision && Arc::ptr_eq(&request.selection, &selection)
            && request.fallback == Some(TreeRowId(11))))));
}

#[test]
fn unselected_parent_never_enters_clipboard() {
    for intent in [FileClipboardIntent::Copy, FileClipboardIntent::Move] {
        let mut p = loaded();
        let publication = p.panes[0].tree.publication.as_mut().unwrap();
        publication.selection =
            Arc::new(TreeSelectionSnapshot::from_bits(publication.view.clone(), vec![0]).unwrap());
        p.panes[0].parent_cursor = true;
        p.panes[0].cursor = None;
        p.request_tree_clipboard(PaneId::Left, intent, &|_| panic!("parent copied"));
        assert!(p.file_actions.tree_clipboard.is_none());
    }
}
