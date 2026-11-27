//! A Delete job that waits for an answer is marked in Jobs, and Jobs' Resolve brings
//! its question up again after Decide later, through the real windows and callbacks. A
//! Trash refusal while the job runs is such an answer too, never a permanent delete.
use crate::support::modeless;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, install, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, io, path::Path, rc::Rc, sync::Arc};

struct Harness {
    main: MainWindow,
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    state: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    send: Rc<dyn Fn(Command) -> bool>,
}

impl Harness {
    fn new() -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
        let main = MainWindow::new().unwrap();
        let path = std::env::temp_dir().join("delete-resolve-fixture");
        let state = Rc::new(RefCell::new(Presenter::new(path.clone(), path)));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let saved = commands.clone();
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
            saved.borrow_mut().push(command);
            true
        });
        install(&main, state.clone(), send.clone());
        main.set_preferences_ready(true);
        main.show().unwrap();
        main.window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        modeless::draw(&windows.borrow()[0], 900, 600, "delete-resolve-owner");
        commands.borrow_mut().clear();
        Self {
            main,
            windows,
            state,
            commands,
            send,
        }
    }

    fn apply(&self, event: Event) {
        self.state
            .borrow_mut()
            .apply_with_sender(event, self.send.as_ref());
        refresh(&self.main, &self.state.borrow());
    }

    /// The visible windows besides the main one: Jobs, the Delete's own progress
    /// window and, while it is shown, the question.
    fn children(&self, expected: usize) -> Vec<Rc<MinimalSoftwareWindow>> {
        slint::platform::update_timers_and_animations();
        let children: Vec<_> = self
            .windows
            .borrow()
            .iter()
            .skip(1)
            .filter(|window| window.window().is_visible())
            .cloned()
            .collect();
        assert_eq!(children.len(), expected, "visible tool windows");
        for child in &children {
            modeless::draw(child, 700, 500, "delete-resolve-child");
        }
        children
    }

    /// The one visible tool window with a `label` button, among `expected` windows.
    fn window_with(&self, label: &str, expected: usize) -> Rc<MinimalSoftwareWindow> {
        let mut found: Vec<_> = self
            .children(expected)
            .into_iter()
            .filter(|window| {
                modeless::accessible_item(window, AccessibleRole::Button, label).is_some()
            })
            .collect();
        assert_eq!(found.len(), 1, "one window offers {label}");
        found.remove(0)
    }

    fn decisions(&self) -> Vec<(OperationId, u64, CopyChoice)> {
        self.commands
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::DecideDelete(decision) => {
                    Some((decision.id, decision.version, decision.choice.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// Whether the Jobs row of `identity` asks for attention.
    fn marked(&self, identity: &str) -> bool {
        self.main
            .get_operations()
            .iter()
            .find(|row| row.identity == identity)
            .unwrap_or_else(|| panic!("Jobs shows no row {identity}"))
            .attention
    }
}

fn running_delete(h: &Harness, mode: DeleteMode) {
    h.apply(Event::Operation(OperationEvent::Progress(Arc::new(
        OperationProgress {
            id: OperationId(41),
            kind: MutationKind::Delete(mode),
            state: OperationState::Running,
            pause_requested: false,
            paused: false,
            total: 2,
            completed: 0,
            failed: 0,
            processed_entries: 3,
            current_path: Some("source/locked".into()),
            source_directory: "source".into(),
            renamed_to: None,
            message: String::new(),
            errors: vec![],
            throughput: Arc::default(),
            discovery: OperationDiscovery::default(),
        },
    ))));
    assert!(!h.marked("41"), "a running Delete needs nothing");
}

/// A running permanent Delete, then its failure on a folder it cannot remove.
fn delete_waits(h: &Harness) {
    running_delete(h, DeleteMode::Permanent);
    h.apply(Event::Operation(OperationEvent::DeleteFailure(Arc::new(
        DeleteFailure {
            id: OperationId(41),
            version: 1,
            mode: DeleteMode::Permanent,
            error: FsError::from_io(
                "mutate entry",
                Path::new("source/locked"),
                io::Error::from(io::ErrorKind::PermissionDenied),
            ),
            can_retry: true,
            can_skip: true,
        },
    ))));
}

/// A Trash refusal after the Delete started, as Windows gives when a drive's Recycle Bin
/// was turned off after F8 checked it.
fn trash_refused(h: &Harness, version: u64) {
    h.apply(Event::Operation(OperationEvent::DeleteFailure(Arc::new(
        DeleteFailure {
            id: OperationId(41),
            version,
            mode: DeleteMode::Trash,
            can_retry: true,
            can_skip: true,
            error: FsError::trash_unavailable(
                Path::new("source/Screenshot 2023-03-31 105053.png"),
                "Windows Recycle Bin is disabled for this location",
            ),
        },
    ))));
}

/// The stuck Move to Trash of 0.6.1 asked whether to delete permanently instead, in a
/// window of its own (Permanently delete, Cancel job, Decide later). F8 now asks for a
/// permanent delete before the job when no Trash takes the items; a refusal that still
/// comes while the job runs is an ordinary failure in the shared question window:
/// Retry, Skip, Skip all errors, Cancel job and Decide later, never Permanently delete.
#[test]
fn a_trash_refusal_during_the_job_is_an_ordinary_failure_in_its_window() {
    let h = Harness::new();
    running_delete(&h, DeleteMode::Trash);
    h.window_with("Cancel operation", 1);
    trash_refused(&h, 1);
    let question = h.window_with("Retry", 2);
    for label in ["Skip", "Cancel job", "Decide later"] {
        assert!(
            modeless::accessible_item(&question, AccessibleRole::Button, label).is_some(),
            "the question offers {label}"
        );
    }
    assert!(
        modeless::accessible_item(&question, AccessibleRole::Button, "Permanently delete")
            .is_none(),
        "a Trash refusal never offers a permanent delete"
    );
    let body = h.main.get_copy_body();
    for text in [
        "Moving to Trash",
        "Windows Recycle Bin is disabled for this location",
        "The Trash did not take this item, and it was kept.",
    ] {
        assert!(body.contains(text), "{text:?} in {body:?}");
    }
    let row = h
        .main
        .get_operations()
        .iter()
        .find(|row| row.identity == "41")
        .unwrap();
    assert!(
        row.activity_label
            .starts_with("Waiting for your answer: the Delete stopped at an error.")
            && row.eta.is_empty(),
        "{:?}",
        row.activity_label
    );
    modeless::click(&question, "Retry");
    assert!(
        matches!(
            h.decisions().as_slice(),
            [(OperationId(41), 1, CopyChoice::Retry)]
        ),
        "{:?}",
        h.decisions()
    );
    h.children(1);
    assert!(!h.marked("41"), "an answered question leaves no mark");
}

#[test]
fn jobs_resolve_brings_up_a_deferred_delete_question() {
    let h = Harness::new();
    h.main.invoke_show_operations();
    delete_waits(&h);
    // The question opens by itself beside Jobs and the Delete's progress window, and
    // the job's row is marked.
    let decision = h.window_with("Decide later", 3);
    assert_eq!(h.main.get_copy_token(), "delete-error:41:1");
    assert!(h.marked("41"), "the waiting question marks the Jobs row");
    modeless::click(&decision, "Decide later");
    let jobs = h.window_with("Clear all finished", 2);
    assert!(h.decisions().is_empty(), "Decide later answers nothing");
    assert!(h.marked("41"), "a deferred question still waits");
    let progress = h.window_with("Jobs", 2);
    assert!(
        modeless::accessible_item(&progress, AccessibleRole::Button, "Resolve").is_some(),
        "the Delete's progress window offers Resolve too"
    );
    // Jobs offers Resolve for the marked row; it shows the same question again.
    modeless::click(&jobs, "Resolve");
    let decision = h.window_with("Decide later", 3);
    assert_eq!(
        h.main.get_copy_token(),
        "delete-error:41:1",
        "Resolve reopens the exact deferred Delete question"
    );
    modeless::click(&decision, "Retry");
    assert!(
        matches!(
            h.decisions().as_slice(),
            [(OperationId(41), 1, CopyChoice::Retry)]
        ),
        "the answer goes to the Delete job: {:?}",
        h.decisions()
    );
    h.children(2);
    assert!(!h.marked("41"), "an answered question leaves no mark");
}
