//! Installed Slint keys and real native transfers; the OS clipboard alone is in memory.
#[path = "support/window_input.rs"]
mod input;
#[path = "support/tree_move_native.rs"]
mod native;
#[path = "support/tree_filters.rs"]
mod support;
#[path = "support/ui.rs"]
mod ui;
#[path = "support/tree_watch.rs"]
mod watch;

use midnight_core::*;
use slint::{Model, platform::Key};
use std::{
    fs,
    sync::{Arc, Mutex},
    thread::{self, ThreadId},
    time::{Duration, Instant},
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
}

impl MemoryClipboard {
    fn new() -> Self {
        Self {
            ui_thread: thread::current().id(),
            state: Arc::new(Mutex::new(ClipboardState::default())),
        }
    }
}

struct ClipboardOwner {
    worker: ThreadId,
    state: Arc<Mutex<ClipboardState>>,
}

impl ClipboardService for MemoryClipboard {
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError> {
        let worker = thread::current().id();
        assert_ne!(
            worker, self.ui_thread,
            "clipboard opening must run off the UI thread"
        );
        Ok(Box::new(ClipboardOwner {
            worker,
            state: self.state.clone(),
        }))
    }
}

impl ClipboardSession for ClipboardOwner {
    fn copy_text(&mut self, _: &str) -> Result<(), FsError> {
        panic!("file keyboard actions must publish native file lists, not display text")
    }

    fn copy_files(&mut self, files: &FileClipboard) -> Result<(), FsError> {
        assert_eq!(thread::current().id(), self.worker);
        files.validate()?;
        let mut state = self.state.lock().unwrap();
        state.files = Some(files.clone());
        state.writes.push(files.clone());
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

fn published(h: &mut support::Harness) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            Instant::now() < deadline,
            "native clipboard publication must finish"
        );
        let event = h.receive();
        let done = matches!(
            &event,
            Event::DesktopActionFinished {
                result: Ok(DesktopActionResult::ClipboardCopied),
                ..
            }
        );
        if let Event::DesktopActionFinished {
            result: Err(error), ..
        } = &event
        {
            panic!("clipboard action failed: {error:?}");
        }
        h.apply_event(event);
        if done {
            return;
        }
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
    );
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
        watch.observes(&fixture.root.join("folder"))
            && watch.observes(&fixture.root.join("destination/existing"))
    });
    let destination_session = h.views[destination.index()].as_ref().unwrap().stamp.session;

    for name in ["large", "small"] {
        focus(&h, pane, name, true);
    }
    // Deliberately send the chord before draining the selection ACK: capture
    // must wait for these marks rather than copying an older selection.
    ui::chord(
        &h.ui.window,
        &[Key::Control],
        if intent == FileClipboardIntent::Move {
            "x"
        } else {
            "c"
        },
    );
    published(&mut h);
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
    // external writes prove both retained native tree watches still reconcile.
    let callbacks = watch.callbacks();
    fs::create_dir(fixture.root.join("folder/after-clipboard")).unwrap();
    fs::create_dir(fixture.root.join("destination/after-clipboard")).unwrap();
    h.until(|h| {
        watch.callbacks() > callbacks
            && h.rows().iter().any(|row| row.name == "after-clipboard")
            && rows(h, destination)
                .iter()
                .any(|row| row.name == "after-clipboard")
    });
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
    let mut h = support::Harness::with_native_clipboard_transfer(
        PaneId::Left,
        fixture.root.clone(),
        service.clone(),
        service.clone(),
        service,
        clipboard.clone(),
    );
    focus(&h, PaneId::Left, "..", false);
    assert!(h.selection.as_ref().unwrap().is_empty());
    ui::chord(&h.ui.window, &[Key::Control], "c");
    ui::chord(&h.ui.window, &[Key::Control], "x");
    focus(&h, PaneId::Left, "small", false);
    ui::chord(&h.ui.window, &[Key::Control], "c");
    published(&mut h);
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
