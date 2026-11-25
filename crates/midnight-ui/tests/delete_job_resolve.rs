//! A Delete job that waits for an answer is marked in Jobs, and Jobs' Resolve brings
//! its question up again after Decide later, through the real windows and callbacks.
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
            permanent_fallback: None,
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

#[test]
fn trash_fallback_uses_the_destructive_confirmation_for_the_original_item() {
    use midnight_ui::OperationPresentation;
    let h = Harness::new();
    running_delete(&h, DeleteMode::Trash);
    let path = Path::new("source/unrecycled.txt");
    for (version, accept) in [(1, false), (2, true)] {
        h.apply(Event::Operation(OperationEvent::DeleteFailure(Arc::new(
            DeleteFailure {
                id: OperationId(41),
                version,
                mode: DeleteMode::Trash,
                can_retry: false,
                can_skip: false,
                error: FsError::trash_unavailable(path, "fixture has no recycle bin"),
                permanent_fallback: Some(OperationPreview {
                    id: OperationId(41),
                    kind: MutationKind::Delete(DeleteMode::Permanent),
                    files: 1,
                    folders: 0,
                    file_bytes: Some(10),
                    sample: vec![MutationItem {
                        path: path.into(),
                        kind: EntryKind::File,
                        metadata: Default::default(),
                    }],
                }),
            },
        ))));
        let form = h.main.global::<OperationPresentation>();
        assert!(form.get_active() && form.get_dangerous());
        assert_eq!(form.get_confirm_text(), "Permanently delete");
        assert!(form.get_question().contains("unrecycled.txt"));
        assert!(h.main.get_operation_warning().contains("cannot be undone"));
        assert!(h.main.get_copy_token().is_empty());
        modeless::draw(
            &h.windows.borrow()[0],
            900,
            650,
            "trash-unavailable-permanent-offer",
        );
        h.commands.borrow_mut().clear();
        if accept {
            form.invoke_confirm(form.get_token(), "".into());
        } else {
            form.invoke_request_dismiss(form.get_token());
        }
        assert!(h.commands.borrow().iter().any(|command| matches!(command,
            Command::AnswerTrashFallback { id: OperationId(41), version: v, confirm } if *v == version && *confirm == accept)));
        assert!(!form.get_active());
    }
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
