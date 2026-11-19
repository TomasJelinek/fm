//! Installed Slint keys and real native transfers; the OS clipboard alone is in memory.
#[path = "support/window_input.rs"]
mod input;
#[path = "support/tree_move_native.rs"]
mod native;
#[path = "support/tree_filters.rs"]
mod support;
use crate::support::ui;
#[path = "support/tree_watch.rs"]
mod watch;

use midnight_core::*;
use slint::{Model, platform::Key};
use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread::{self, ThreadId},
};

#[derive(Default)]
struct ClipboardState {
    files: Option<FileClipboard>,
    writes: Vec<FileClipboard>,
    reads: usize,
    dropped: usize,
}

struct MemoryClipboard {
    ui_thread: ThreadId,
    state: Arc<Mutex<ClipboardState>>,
    trace: Arc<ClipboardActorTrace>,
}

#[derive(Default)]
struct ClipboardActorTrace {
    opens: AtomicUsize,
    copy_calls: AtomicUsize,
    copies: AtomicUsize,
}

impl MemoryClipboard {
    fn new() -> Self {
        Self {
            ui_thread: thread::current().id(),
            state: Arc::new(Mutex::new(ClipboardState::default())),
            trace: Arc::new(ClipboardActorTrace::default()),
        }
    }
}

struct ClipboardOwner {
    worker: ThreadId,
    state: Arc<Mutex<ClipboardState>>,
    trace: Arc<ClipboardActorTrace>,
}

impl ClipboardService for MemoryClipboard {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        let worker = thread::current().id();
        assert_ne!(
            worker, self.ui_thread,
            "clipboard opening must run off the UI thread"
        );
        self.trace.opens.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(ClipboardOwner {
            worker,
            state: self.state.clone(),
            trace: self.trace.clone(),
        }))
    }
}

impl ClipboardSession for ClipboardOwner {
    fn copy_text(&mut self, _: &str) -> Result<(), FsError> {
        panic!("file keyboard actions must publish native file lists, not display text")
    }

    fn copy_files(&mut self, files: &FileClipboard) -> Result<(), FsError> {
        assert_eq!(thread::current().id(), self.worker);
        self.trace.copy_calls.fetch_add(1, Ordering::Relaxed);
        files.validate()?;
        let mut state = self.state.lock().unwrap();
        state.files = Some(files.clone());
        state.writes.push(files.clone());
        drop(state);
        self.trace.copies.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn read_files(&mut self) -> Result<FileClipboard, FsError> {
        assert_eq!(thread::current().id(), self.worker);
        let mut state = self.state.lock().unwrap();
        state.reads += 1;
        Ok(state
            .files
            .clone()
            .expect("paste follows successful clipboard publication"))
    }

    fn service_pending(&mut self) -> Result<(), FsError> {
        assert_eq!(thread::current().id(), self.worker);
        Ok(())
    }
}

impl Drop for ClipboardOwner {
    fn drop(&mut self) {
        assert_eq!(
            thread::current().id(),
            self.worker,
            "clipboard owner teardown is thread affine"
        );
        self.state.lock().unwrap().dropped += 1;
    }
}

fn rows(h: &support::Harness, pane: PaneId) -> Vec<midnight_ui::FileRow> {
    if pane == PaneId::Left {
        h.ui.window.get_left_rows().iter().collect()
    } else {
        h.ui.window.get_right_rows().iter().collect()
    }
}

fn focus(h: &support::Harness, pane: PaneId, name: &str, select: bool) {
    let position = rows(h, pane)
        .iter()
        .position(|row| row.name == name)
        .unwrap();
    h.ui.window.invoke_activate(pane.index() as i32);
    h.ui.window
        .invoke_row_click(pane.index() as i32, position as i32, select, false);
    h.ui.window.invoke_focus_files();
}

fn published(h: &mut support::Harness, clipboard: &MemoryClipboard) {
    let expected = h.clipboard_publications + 1;
    if let Err(panic) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.until(|h| h.clipboard_publications == expected);
    })) {
        let state = clipboard
            .state
            .try_lock()
            .ok()
            .map(|state| (state.writes.len(), state.reads, state.dropped));
        eprintln!(
            "clipboard actor: opens={}, copy_calls={}, copies={}, state(writes,reads,dropped)={state:?}",
            clipboard.trace.opens.load(Ordering::Relaxed),
            clipboard.trace.copy_calls.load(Ordering::Relaxed),
            clipboard.trace.copies.load(Ordering::Relaxed),
        );
        std::panic::resume_unwind(panic);
    }
}

fn exercise(pane: PaneId, intent: FileClipboardIntent) {
    let fixture = native::Fixture::new();
    fs::create_dir_all(fixture.root.join("destination/existing/retained")).unwrap();
    fs::write(
        fixture.root.join("folder/large/část 文档.txt"),
        b"raw names",
    )
    .unwrap();
    let service = Arc::new(midnight_platform::LocalFileSystem::new(None));
    let watch = Arc::new(watch::ObservedWatch::new(service.clone()));
    let clipboard = Arc::new(MemoryClipboard::new());
    let mut h = support::Harness::with_native_clipboard_transfer(
        pane,
        fixture.root.clone(),
        service.clone(),
        service,
        watch.clone(),
        clipboard.clone(),
        true,
    );
    assert!(h.maintenance_hints_held());
    let source_session = h.view.as_ref().unwrap().stamp.session;
    let destination = pane.other();
    let existing = rows(&h, destination)
        .iter()
        .find(|row| row.name == "existing")
        .unwrap()
        .identity
        .clone();
    h.ui.window
        .invoke_tree_disclosure(destination.index() as i32, existing);
    h.until(|h| {
        h.views[destination.index()].as_ref().is_some_and(|view| {
            view.rows
                .iter()
                .any(|row| row.entry.display_name == "retained")
        })
    });
    h.until(|_| {
        watch.live_covers(&fixture.root.join("folder"))
            && watch.live_covers(&fixture.root.join("destination"))
            && watch.live_covers(&fixture.root.join("destination/existing"))
    });
    let destination_session = h.views[destination.index()].as_ref().unwrap().stamp.session;

    for name in ["large", "small"] {
        focus(&h, pane, name, true);
    }
    // Deliberately send the chord before draining the selection ACK: capture
    // must wait for these marks rather than copying an older selection.
    h.clipboard_checkpoint("before_copy_chord");
    ui::chord(
        &h.ui.window,
        &[Key::Control],
        if intent == FileClipboardIntent::Move {
            "x"
        } else {
            "c"
        },
    );
    h.clipboard_checkpoint("after_copy_chord");
    published(&mut h, &clipboard);
    {
        let state = clipboard.state.lock().unwrap();
        assert_eq!(
            state.writes,
            vec![FileClipboard {
                paths: vec![
                    fixture.root.join("folder/large"),
                    fixture.root.join("folder/small")
                ],
                intent,
            }]
        );
        assert_eq!(state.reads, 0);
    }
    assert!(
        h.copy_progress.is_empty(),
        "publishing Cut must not start a Move"
    );
    assert!(fixture.root.join("folder/large/payload.bin").is_file());
    assert!(fixture.root.join("folder/small/payload.bin").is_file());
    h.ui.window.invoke_activate(destination.index() as i32);
    h.ui.window.invoke_focus_files();
    ui::chord(&h.ui.window, &[Key::Control], "v");
    if intent == FileClipboardIntent::Move {
        h.until(|h| !h.ui.window.get_copy_draft_token().is_empty());
        assert!(h.ui.window.get_copy_draft_move());
        assert!(
            h.copy_progress.is_empty(),
            "Cut/Paste waits for explicit Move confirmation"
        );
        assert!(fixture.root.join("folder/large/payload.bin").is_file());
        input::key(&h.dialog(), Key::Return);
    }
    h.until(|h| {
        h.copy_progress
            .last()
            .is_some_and(|progress| progress.state.is_terminal())
    });
    h.release_maintenance_hints();
    assert!(!h.maintenance_hints_held());
    let progress = h.copy_progress.last().unwrap();
    assert_eq!(
        progress.state,
        CopyOperationState::Completed,
        "{progress:?}"
    );
    assert_eq!(
        progress.intent,
        if intent == FileClipboardIntent::Move {
            TransferIntent::Move
        } else {
            TransferIntent::Copy
        }
    );
    if intent == FileClipboardIntent::Move {
        // These sibling roots share the owned fixture's volume. Native rename
        // moves both trees without copying, checksumming or deleting each file.
        assert_eq!(progress.native_entries_moved, 2);
        assert_eq!(
            (
                progress.copied_bytes,
                progress.verified_bytes,
                progress.validation_bytes
            ),
            (0, 0, 0),
        );
    } else {
        assert_eq!(progress.native_entries_moved, 0);
        assert_eq!(
            (progress.copied_bytes, progress.verified_bytes),
            (1021, 1021)
        );
    }
    assert_eq!(progress.source_files_removed, 0);
    for (name, byte, count) in [("large", 42, 1000), ("small", 43, 12)] {
        assert_eq!(
            fs::read(
                fixture
                    .root
                    .join("destination")
                    .join(name)
                    .join("payload.bin")
            )
            .unwrap(),
            vec![byte; count]
        );
        assert_eq!(
            fixture.root.join("folder").join(name).exists(),
            intent == FileClipboardIntent::Copy
        );
        if intent == FileClipboardIntent::Copy {
            assert_eq!(
                fs::read(fixture.root.join("folder").join(name).join("payload.bin")).unwrap(),
                vec![byte; count]
            );
        }
    }
    assert_eq!(
        fs::read(fixture.root.join("destination/large/část 文档.txt")).unwrap(),
        b"raw names"
    );
    if intent == FileClipboardIntent::Copy {
        assert_eq!(
            fs::read(fixture.root.join("folder/large/část 文档.txt")).unwrap(),
            b"raw names"
        );
    }
    assert!(fixture.root.join("destination/large/nested/empty").is_dir());
    assert!(
        fixture.root.join("folder/empty").is_dir(),
        "unselected folder stays untouched"
    );
    h.until(|h| {
        rows(h, destination).iter().any(|row| row.name == "large")
            && h.rows().iter().any(|row| row.name == "small")
                == (intent == FileClipboardIntent::Copy)
    });

    // Own-write notifications may be consumed by terminal maintenance. Later
    // external writes prove both retained native tree watches still reconcile:
    // each parent needs evidence from a live subscription covering it, so one
    // pane's callback cannot stand in for the other's, nor can the engine's poll.
    let (source_parent, destination_parent) = (
        fixture.root.join("folder"),
        fixture.root.join("destination"),
    );
    let mark = watch.mark();
    fs::create_dir(source_parent.join("after-clipboard")).unwrap();
    fs::create_dir(destination_parent.join("after-clipboard")).unwrap();
    if let Err(panic) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.until(|h| {
            watch.native_evidence_for(mark, &source_parent)
                && watch.native_evidence_for(mark, &destination_parent)
                && h.rows().iter().any(|row| row.name == "after-clipboard")
                && rows(h, destination)
                    .iter()
                    .any(|row| row.name == "after-clipboard")
        });
    })) {
        eprintln!(
            "tree clipboard watch predicates: mark={mark:?}, native evidence (source, destination)={:?}, rows (source, destination)={:?}, {}",
            (
                watch.native_evidence_for(mark, &source_parent),
                watch.native_evidence_for(mark, &destination_parent)
            ),
            (
                h.rows().iter().any(|row| row.name == "after-clipboard"),
                rows(&h, destination)
                    .iter()
                    .any(|row| row.name == "after-clipboard")
            ),
            watch.describe(),
        );
        std::panic::resume_unwind(panic);
    }
    assert_eq!(h.view.as_ref().unwrap().stamp.session, source_session);
    assert_eq!(
        h.views[destination.index()].as_ref().unwrap().stamp.session,
        destination_session
    );
    assert!(
        rows(&h, destination)
            .iter()
            .any(|row| row.name == "retained")
    );
    drop(h);
    let state = clipboard.state.lock().unwrap();
    assert_eq!(state.reads, 1);
    assert_eq!(state.dropped, 1);
}

#[test]
fn copy_nested_folders_from_left_tree_and_paste_into_right_tree_root() {
    exercise(PaneId::Left, FileClipboardIntent::Copy);
}

#[test]
fn copy_nested_folders_from_right_tree_and_paste_into_left_tree_root() {
    exercise(PaneId::Right, FileClipboardIntent::Copy);
}

#[test]
fn cut_nested_folders_from_left_tree_and_confirm_native_move_into_right_tree() {
    exercise(PaneId::Left, FileClipboardIntent::Move);
}

#[test]
fn cut_nested_folders_from_right_tree_and_confirm_native_move_into_left_tree() {
    exercise(PaneId::Right, FileClipboardIntent::Move);
}

#[test]
fn unselected_parent_is_excluded_and_focused_nested_folder_is_the_fallback() {
    let fixture = native::Fixture::new();
    let service = Arc::new(midnight_platform::LocalFileSystem::new(None));
    let clipboard = Arc::new(MemoryClipboard::new());
    // The copies capture the exact tree; startup maintenance must not replace it
    // first. This test needs no hints, so they stay held.
    let mut h = support::Harness::with_native_clipboard_transfer(
        PaneId::Left,
        fixture.root.clone(),
        service.clone(),
        service.clone(),
        service,
        clipboard.clone(),
        true,
    );
    focus(&h, PaneId::Left, "..", false);
    assert!(h.selection.as_ref().unwrap().is_empty());
    ui::chord(&h.ui.window, &[Key::Control], "c");
    ui::chord(&h.ui.window, &[Key::Control], "x");
    focus(&h, PaneId::Left, "small", false);
    ui::chord(&h.ui.window, &[Key::Control], "c");
    published(&mut h, &clipboard);
    assert_eq!(
        clipboard.state.lock().unwrap().writes,
        vec![FileClipboard {
            paths: vec![fixture.root.join("folder/small")],
            intent: FileClipboardIntent::Copy,
        }]
    );
    assert!(h.selection.as_ref().unwrap().is_empty());
    assert!(h.copy_progress.is_empty());
    drop(h);
    assert_eq!(clipboard.state.lock().unwrap().dropped, 1);
}
