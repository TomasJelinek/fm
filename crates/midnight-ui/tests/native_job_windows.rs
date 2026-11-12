//! Real job callbacks keep presentation closure separate from operation decisions.
#[path = "support/window_input.rs"]
mod input;
#[allow(dead_code)]
#[path = "support/modeless.rs"]
mod modeless;

use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, install, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::{Cell, RefCell},
    io,
    path::Path,
    rc::Rc,
    sync::Arc,
};

struct Boundary {
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    refuse_window: Rc<Cell<bool>>,
    attempts: Rc<Cell<usize>>,
}
impl slint::platform::Platform for Boundary {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        self.attempts.set(self.attempts.get() + 1);
        if self.refuse_window.replace(false) {
            return Err("injected native creation failure".into());
        }
        let native = MinimalSoftwareWindow::new(
            slint::platform::software_renderer::RepaintBufferType::NewBuffer,
        );
        self.windows.borrow_mut().push(native.clone());
        Ok(native)
    }
}

struct Harness {
    main: MainWindow,
    refuse_window: Rc<Cell<bool>>,
    attempts: Rc<Cell<usize>>,
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    state: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    refuse: Rc<Cell<bool>>,
    send: Rc<dyn Fn(Command) -> bool>,
}
impl Harness {
    fn new() -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        let refuse_window = Rc::new(Cell::new(false));
        let attempts = Rc::new(Cell::new(0));
        slint::platform::set_platform(Box::new(Boundary {
            windows: windows.clone(),
            refuse_window: refuse_window.clone(),
            attempts: attempts.clone(),
        }))
        .unwrap();
        let main = MainWindow::new().unwrap();
        let path = std::env::temp_dir().join("native-jobs-owned-fixture");
        let state = Rc::new(RefCell::new(Presenter::new(path.clone(), path)));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let refuse = Rc::new(Cell::new(false));
        let gate = refuse.clone();
        let saved = commands.clone();
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
            if gate.get()
                && matches!(
                    command,
                    Command::DecideCopy(_)
                        | Command::ResolveMetadataConflict { .. }
                        | Command::CancelCopy(_)
                        | Command::CancelOperation(_)
                        | Command::SetCopyPaused { .. }
                        | Command::SetOperationPaused { .. }
                )
            {
                return false;
            }
            saved.borrow_mut().push(command);
            true
        });
        install(&main, state.clone(), send.clone());
        main.set_preferences_ready(true);
        main.show().unwrap();
        main.window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        modeless::draw(&windows.borrow()[0], 900, 600, "jobs-owner");
        commands.borrow_mut().clear();
        Self {
            main,
            refuse_window,
            attempts,
            windows,
            state,
            commands,
            refuse,
            send,
        }
    }
    fn apply(&self, event: Event) {
        self.state
            .borrow_mut()
            .apply_with_sender(event, self.send.as_ref());
        refresh(&self.main, &self.state.borrow());
    }
    fn start(&self, id: u64) {
        self.apply(Event::DragDrop(DragDropEvent::DropAccepted {
            drag: None,
            token: DropToken(id),
            pane: PaneId::Right,
            item_count: 1,
            intent: TransferIntent::Copy,
            verification: None,
            destination: Arc::new(std::env::temp_dir().join("native-jobs-destination")),
        }));
        assert!(
            self.main
                .get_operations()
                .iter()
                .any(|row| row.identity == format!("copy-job:{id}")),
            "fixture must use the actual assigned operation ID, not merely its drop token"
        );
    }
    fn complete(&self, id: u64) {
        let mut progress = CopyProgress::queued(
            OperationId(id),
            "destination".into(),
            1,
            TransferIntent::Copy,
        );
        progress.state = CopyOperationState::Completed;
        self.apply(Event::Copy(CopyEvent::Progress(Arc::new(progress))));
    }
    fn failure(&self, id: u64, version: u64) {
        self.apply(Event::Copy(CopyEvent::Failure(Arc::new(CopyFailure {
            id: OperationId(id),
            version,
            error: FsError::from_io(
                "write",
                Path::new("destination/file.txt"),
                io::Error::from(io::ErrorKind::PermissionDenied),
            ),
            source: Some("source/file.txt".into()),
            destination: "destination/file.txt".into(),
            stage: CopyErrorStage::Writing,
            can_retry: true,
            can_skip: true,
        }))));
    }
    fn children(&self, expected: usize) -> Vec<Rc<MinimalSoftwareWindow>> {
        slint::platform::update_timers_and_animations();
        let children: Vec<_> = self
            .windows
            .borrow()
            .iter()
            .skip(1)
            .filter(|w| w.window().is_visible())
            .cloned()
            .collect();
        assert_eq!(
            children.len(),
            expected,
            "Jobs and decisions must own independent visible adapters"
        );
        assert!(
            !self.main.get_dialog_input_blocked(),
            "modeless jobs must not block the owner"
        );
        assert!(
            i_slint_core::window::WindowInner::from_pub(self.main.window())
                .active_popups()
                .is_empty()
        );
        for child in &children {
            modeless::draw(child, 700, 500, "jobs-child");
        }
        children
    }
    fn choices(&self) -> Vec<(OperationId, u64, CopyChoice)> {
        self.commands
            .borrow()
            .iter()
            .filter_map(|c| match c {
                Command::DecideCopy(d) => Some((d.id, d.version, d.choice.clone())),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn native_jobs_close_never_cancels_and_manual_completion_stays_visible() {
    let h = Harness::new();
    h.main.invoke_show_operations();
    let jobs = h.children(1).remove(0);
    h.start(1);
    h.complete(1);
    assert!(
        Rc::ptr_eq(&jobs, &h.children(1)[0]),
        "refresh retains the actual Jobs occurrence"
    );
    h.commands.borrow_mut().clear();
    jobs.window().dispatch_event(WindowEvent::CloseRequested);
    h.children(0);
    assert!(
        h.commands.borrow().is_empty(),
        "native Jobs close is presentation-only"
    );
    h.main.invoke_show_operations();
    let reopened = h.children(1).remove(0);
    input::key(&reopened, Key::Escape);
    h.children(0);
    assert!(h.commands.borrow().is_empty());
}

#[test]
fn native_automatic_jobs_close_after_success_but_keep_the_history() {
    let h = Harness::new();
    h.start(1);
    h.children(1);
    h.complete(1);
    h.children(0);
    h.main.invoke_show_operations();
    let jobs = h.children(1).remove(0);
    assert!(
        modeless::accessible_item(
            &jobs,
            i_slint_core::items::AccessibleRole::Text,
            "Completed"
        )
        .is_some()
    );
}

#[test]
fn native_decision_later_keeps_jobs_and_retry_uses_the_exact_revision() {
    let h = Harness::new();
    h.start(1);
    let jobs = h.children(1).remove(0);
    h.failure(1, 9);
    let children = h.children(2);
    assert!(Rc::ptr_eq(&jobs, &children[0]));
    let decision = children[1].clone();
    modeless::click(&decision, "Decide later");
    assert!(h.choices().is_empty(), "Later must not send Cancel or Skip");
    assert!(Rc::ptr_eq(&jobs, &h.children(1)[0]));
    refresh(&h.main, &h.state.borrow());
    h.children(1);
    modeless::click(&jobs, "Resolve");
    let decision = h.children(2).pop().unwrap();
    h.refuse.set(true);
    modeless::click(&decision, "Retry");
    assert!(h.choices().is_empty());
    assert!(
        decision.window().is_visible(),
        "rejected admission preserves an actionable prompt"
    );
    h.refuse.set(false);
    modeless::click(&decision, "Retry");
    assert!(matches!(
        h.choices().as_slice(),
        [(OperationId(1), 9, CopyChoice::Retry)]
    ));
    assert!(Rc::ptr_eq(&jobs, &h.children(1)[0]));
}

#[test]
fn native_decision_escape_and_x_cancel_exact_jobs_without_closing_jobs() {
    let h = Harness::new();
    for (id, native_close) in [(1, false), (2, true)] {
        h.start(id);
        h.failure(id, id + 100);
        let decision = h.children(2).pop().unwrap();
        modeless::draw(&decision, 320, 200, "jobs-decision-compact");
        if native_close {
            decision
                .window()
                .dispatch_event(WindowEvent::CloseRequested);
        } else {
            input::key(&decision, Key::Escape);
        }
        let progress = h.children(1).remove(0);
        progress
            .window()
            .dispatch_event(WindowEvent::CloseRequested);
        h.children(0);
        assert!(
            matches!(h.choices().last(), Some((target, version, CopyChoice::Cancel)) if *target == OperationId(id) && *version == id + 100)
        );
    }
}

#[test]
fn native_decision_stale_window_cannot_cancel_a_newer_revision() {
    let h = Harness::new();
    h.start(1);
    h.failure(1, 4);
    let old = h.children(2).pop().unwrap();
    h.failure(1, 5);
    let current = h.children(2).pop().unwrap();
    assert!(
        !Rc::ptr_eq(&old, &current),
        "new revision owns a new decision occurrence"
    );
    old.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(
        h.choices().is_empty(),
        "retired child callback has no authority"
    );
    modeless::click(&current, "Skip");
    assert!(matches!(
        h.choices().as_slice(),
        [(OperationId(1), 5, CopyChoice::Skip)]
    ));
}

#[test]
fn native_metadata_decision_later_and_cancel_keep_exact_tokens() {
    let h = Harness::new();
    h.main.invoke_show_operations();
    h.apply(Event::Operation(OperationEvent::Progress(Arc::new(
        OperationProgress {
            pause_requested: false,
            paused: false,
            discovery: Default::default(),
            id: OperationId(91),
            kind: MutationKind::EditMetadata,
            state: OperationState::Running,
            total: 1,
            completed: 0,
            failed: 0,
            processed_entries: 0,
            throughput: Arc::default(),
            current_path: None,
            message: "Waiting for name conflict".into(),
            errors: Vec::new(),
            source_directory: "source".into(),
            renamed_to: None,
        },
    ))));
    let conflict = |token| {
        Event::Operation(OperationEvent::MetadataConflict(Arc::new(
            MetadataConflict {
                id: OperationId(91),
                token,
                source: "source/old.txt".into(),
                destination: "source/new.txt".into(),
            },
        )))
    };
    h.apply(conflict(8));
    let decision = h.children(2).pop().unwrap();
    modeless::click(&decision, "Decide later");
    assert!(
        !h.commands
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::ResolveMetadataConflict { .. }))
    );
    let jobs = h.children(1).remove(0);
    refresh(&h.main, &h.state.borrow());
    h.children(1);
    modeless::click(&jobs, "Resolve");
    let old = h.children(2).pop().unwrap();
    h.apply(conflict(9));
    let current = h.children(2).pop().unwrap();
    old.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(
        !h.commands
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::ResolveMetadataConflict { .. }))
    );
    current.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(h.commands.borrow().iter().any(|c| matches!(
        c,
        Command::ResolveMetadataConflict {
            id: OperationId(91),
            token: 9,
            choice: MetadataConflictPolicy::Cancel,
        }
    )));
    h.children(1);
}

#[test]
fn native_jobs_resolve_selects_the_requested_metadata_job_across_refresh() {
    let h = Harness::new();
    h.main.invoke_show_operations();
    for id in [91, 92] {
        h.apply(Event::Operation(OperationEvent::Progress(Arc::new(
            OperationProgress {
                pause_requested: false,
                paused: false,
                discovery: Default::default(),
                id: OperationId(id),
                kind: MutationKind::EditMetadata,
                state: OperationState::Running,
                total: 1,
                completed: 0,
                failed: 0,
                processed_entries: 0,
                throughput: Arc::default(),
                current_path: None,
                message: "Waiting".into(),
                errors: Vec::new(),
                source_directory: "source".into(),
                renamed_to: None,
            },
        ))));
        h.apply(Event::Operation(OperationEvent::MetadataConflict(
            Arc::new(MetadataConflict {
                id: OperationId(id),
                token: id + 10,
                source: format!("source/{id}.txt").into(),
                destination: format!("target/{id}.txt").into(),
            }),
        )));
    }
    let children = h.children(2);
    let jobs = children[0].clone();
    let old = children[1].clone();
    fn find(item: i_slint_core::item_tree::ItemRc) -> Option<i_slint_core::item_tree::ItemRc> {
        use i_slint_core::accessibility::AccessibleStringProperty;
        if item
            .accessible_string_property(AccessibleStringProperty::Value)
            .is_some_and(|v| v == "metadata-job:92")
        {
            return Some(item);
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            if let Some(found) = find(item) {
                return Some(found);
            }
        }
        None
    }
    let root = i_slint_core::item_tree::ItemRc::new_root(
        i_slint_core::window::WindowInner::from_pub(jobs.window()).component(),
    );
    let row = find(root).expect("actual row for the second metadata job");
    input::click(&jobs, &row);
    modeless::click(&jobs, "Resolve");
    assert_eq!(h.main.get_metadata_conflict_token(), "metadata:92:102");
    refresh(&h.main, &h.state.borrow());
    assert_eq!(
        h.main.get_metadata_conflict_token(),
        "metadata:92:102",
        "progress must not retarget the chosen job"
    );
    old.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(
        !h.commands
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::ResolveMetadataConflict { .. }))
    );
    let current = h.children(2).pop().unwrap();
    modeless::click(&current, "Skip");
    assert!(h.commands.borrow().iter().any(|c| matches!(
        c,
        Command::ResolveMetadataConflict {
            id: OperationId(92),
            token: 102,
            choice: MetadataConflictPolicy::Skip,
        }
    )));
}

#[test]
fn native_decision_failed_creation_preserves_job_and_retries_only_on_resolve() {
    let h = Harness::new();
    h.start(1);
    let jobs = h.children(1).remove(0);
    h.refuse_window.set(true);
    h.failure(1, 6);
    let notice = creation_notice(&h);
    assert!(!Rc::ptr_eq(&notice, &jobs));
    let attempts = h.attempts.get();
    for _ in 0..3 {
        refresh(&h.main, &h.state.borrow());
    }
    assert_eq!(
        h.attempts.get(),
        attempts,
        "progress must not repeatedly reopen a failed native host"
    );
    assert!(
        h.choices().is_empty(),
        "failed creation cannot cancel or skip the waiting job"
    );
    input::key(&notice, Key::Escape);
    h.children(1);
    h.refuse_window.set(false);
    modeless::click(&jobs, "Resolve");
    let decision = h.children(2).pop().unwrap();
    modeless::click(&decision, "Retry");
    assert!(matches!(
        h.choices().as_slice(),
        [(OperationId(1), 6, CopyChoice::Retry)]
    ));
}

fn visible_error(item: i_slint_core::item_tree::ItemRc) -> bool {
    use i_slint_core::accessibility::AccessibleStringProperty;
    if item.is_visible()
        && item.accessible_role() == i_slint_core::items::AccessibleRole::Text
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|text| text.contains("injected native creation failure"))
    {
        return true;
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if visible_error(item) {
            return true;
        }
    }
    false
}

fn creation_notice(h: &Harness) -> Rc<MinimalSoftwareWindow> {
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
    }
    let notice = h
        .windows
        .borrow()
        .iter()
        .skip(1)
        .rfind(|w| w.window().is_visible())
        .cloned()
        .expect("native creation failure must show an owned diagnostic");
    modeless::draw(&notice, 320, 200, "jobs-creation-error-compact");
    assert!(
        visible_error(i_slint_core::item_tree::ItemRc::new_root(
            i_slint_core::window::WindowInner::from_pub(notice.window()).component()
        )),
        "the failure text must be visibly rendered in the compact owned notice"
    );
    notice
}

#[test]
fn native_jobs_failed_creation_reports_visibly_and_only_explicit_open_retries() {
    let h = Harness::new();
    h.refuse_window.set(true);
    h.main.invoke_show_operations();
    let notice = creation_notice(&h);
    let attempts = h.attempts.get();
    assert!(
        h.commands.borrow().is_empty(),
        "failed Jobs creation is presentation-only"
    );
    input::key(&notice, Key::Escape);
    assert!(!h.main.get_operations_visible());
    for _ in 0..3 {
        refresh(&h.main, &h.state.borrow());
    }
    h.children(0);
    assert_eq!(
        h.attempts.get(),
        attempts,
        "ordinary refresh cannot repeatedly reopen failed Jobs"
    );
    h.main.invoke_show_operations();
    h.children(1);
    assert!(h.commands.borrow().is_empty());
}

#[test]
fn native_rename_overwrite_uses_shared_decision_and_exact_mutation_dispatch() {
    let h = Harness::new();
    h.main.invoke_show_operations();
    h.apply(Event::Operation(OperationEvent::Progress(Arc::new(
        OperationProgress {
            pause_requested: false,
            paused: false,
            discovery: Default::default(),
            id: OperationId(90),
            kind: MutationKind::Rename,
            state: OperationState::Running,
            total: 1,
            completed: 0,
            failed: 0,
            processed_entries: 0,
            throughput: Arc::default(),
            current_path: Some("source/old.txt".into()),
            message: String::new(),
            errors: vec![],
            source_directory: "source".into(),
            renamed_to: None,
        },
    ))));
    let item = |name| MutationItem {
        path: std::path::PathBuf::from(name),
        kind: EntryKind::File,
        metadata: EntryMetadata::default(),
    };
    h.apply(Event::Operation(OperationEvent::RenameConflict(Arc::new(
        CopyConflict {
            id: OperationId(90),
            version: 5,
            source: item("source/old.txt"),
            target: item("source/existing.txt"),
            destination: "source/existing.txt".into(),
            can_replace: true,
            can_merge: false,
        },
    ))));
    let decision = h.children(2).pop().unwrap();
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|c| !matches!(c, Command::DecideRename(_)))
    );
    modeless::click(&decision, "Decide later");
    let jobs = h.children(1).remove(0);
    modeless::click(&jobs, "Resolve");
    let decision = h.children(2).pop().unwrap();
    modeless::click(&decision, "Replace · keep original");
    assert!(h.commands.borrow().iter().any(|command| matches!(
        command,
        Command::DecideRename(CopyDecision {
            id: OperationId(90),
            version: 5,
            choice: CopyChoice::ReplaceRecoverable { all: false }
        })
    )));
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::DecideCopy(_)))
    );
    h.children(1);
}

#[test]
fn native_jobs_clear_actions_preserve_active_work_and_repair_selected_details() {
    let h = Harness::new();
    h.main.invoke_show_operations();
    h.start(1);
    h.complete(1);
    h.start(2);
    let mut failed = CopyProgress::queued(
        OperationId(2),
        "failed-target".into(),
        1,
        TransferIntent::Move,
    );
    failed.state = CopyOperationState::Failed;
    failed.failed = 1;
    h.apply(Event::Copy(CopyEvent::Progress(Arc::new(failed))));
    h.start(3);
    let jobs = h.children(3).remove(0);
    h.commands.borrow_mut().clear();
    modeless::click(&jobs, "Clear finished without errors");
    assert_eq!(
        h.main
            .get_operations()
            .iter()
            .map(|r| r.identity.to_string())
            .collect::<Vec<_>>(),
        ["copy-job:3", "copy-job:2"]
    );
    assert!(
        h.commands.borrow().is_empty(),
        "clearing receipts must not dispatch cancellation or filesystem work"
    );
    modeless::draw(&jobs, 700, 500, "jobs-cleared-success");
    modeless::click(&jobs, "Copy details");
    assert!(h.commands.borrow().iter().any(
        |command| matches!(command, Command::CopyText { text, .. } if text.contains("Queued"))
    ));
    h.commands.borrow_mut().clear();
    modeless::click(&jobs, "Clear all finished");
    assert_eq!(
        h.main
            .get_operations()
            .iter()
            .map(|r| r.identity.to_string())
            .collect::<Vec<_>>(),
        ["copy-job:3"]
    );
    assert!(h.commands.borrow().is_empty());
    h.complete(3);
    modeless::draw(&jobs, 700, 500, "jobs-last-completed");
    modeless::click(&jobs, "Clear all finished");
    assert_eq!(h.main.get_operations().row_count(), 0);
    modeless::draw(&jobs, 320, 200, "jobs-empty-compact");
    h.children(1);
}

#[test]
fn independent_progress_windows_route_exact_jobs_and_manual_reopen_does_not_autoclose() {
    let h = Harness::new();
    h.start(1);
    let first = h.children(1).remove(0);
    h.start(2);
    let second = h.children(2).pop().unwrap();
    h.commands.borrow_mut().clear();
    modeless::click(&first, "Cancel operation");
    h.children(2);
    assert!(h.commands.borrow().is_empty());
    modeless::click(&first, "Cancel operation");
    h.children(1);
    assert!(matches!(
        h.commands.borrow().as_slice(),
        [Command::CancelCopy(OperationId(1))]
    ));
    h.commands.borrow_mut().clear();
    input::key(&second, Key::Escape);
    h.children(1);
    assert!(h.commands.borrow().is_empty());
    modeless::click(&second, "Cancel operation");
    h.children(0);
    assert!(matches!(
        h.commands.borrow().as_slice(),
        [Command::CancelCopy(OperationId(2))]
    ));
    h.commands.borrow_mut().clear();
    refresh(&h.main, &h.state.borrow());
    h.children(0);
    h.main.invoke_open_job_progress("copy-job:2".into());
    let reopened = h.children(1).pop().unwrap();
    h.complete(2);
    assert!(
        reopened.window().is_visible(),
        "manually reopened progress stays open"
    );
    h.complete(1);
    h.children(1);
    first.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(
        h.commands.borrow().is_empty(),
        "retired callbacks have no job authority"
    );
    h.main.invoke_show_operations();
    h.children(2);
    input::key(&reopened, Key::Escape);
    h.children(1);
    assert!(h.main.get_operations_visible());
}

#[test]
fn progress_close_retains_window_if_cancellation_cannot_be_admitted() {
    let h = Harness::new();
    h.start(1);
    let progress = h.children(1).remove(0);
    h.commands.borrow_mut().clear();
    h.refuse.set(true);
    progress
        .window()
        .dispatch_event(WindowEvent::CloseRequested);
    h.children(1);
    assert!(h.commands.borrow().is_empty());
    assert!(h.main.get_operations().row_data(0).unwrap().cancellable);
    modeless::click(&progress, "Cancel operation");
    h.children(1);
    assert!(h.commands.borrow().is_empty());
    assert!(h.main.get_operations().row_data(0).unwrap().cancellable);
    modeless::accessible_item(
        &progress,
        i_slint_core::items::AccessibleRole::Button,
        "Continue",
    )
    .expect("rejected cancellation must retain the retryable question");
    h.refuse.set(false);
    modeless::click(&progress, "Cancel operation");
    h.children(0);
    assert!(matches!(
        h.commands.borrow().as_slice(),
        [Command::CancelCopy(OperationId(1))]
    ));
    let row = h.main.get_operations().row_data(0).unwrap();
    assert_eq!(row.state, "Cancelling");
    assert!(
        !row.cancellable,
        "accepted cancellation remains visible in the ledger"
    );
}

#[test]
fn pause_and_resume_are_exact_job_requests_and_wait_for_backend_acknowledgement() {
    let h = Harness::new();
    h.start(1);
    let progress = h.children(1).remove(0);
    h.start(2);
    h.commands.borrow_mut().clear();
    modeless::click(&progress, "Pause");
    assert!(matches!(
        h.commands.borrow().as_slice(),
        [Command::SetCopyPaused {
            id: OperationId(1),
            paused: true
        }]
    ));
    let rows: Vec<_> = h.main.get_operations().iter().collect();
    assert!(
        rows.iter()
            .find(|row| row.identity == "copy-job:1")
            .unwrap()
            .pause_requested
    );
    assert!(
        rows.iter().all(|row| !row.paused),
        "only a safe backend checkpoint acknowledges pause"
    );
    assert!(
        !rows
            .iter()
            .find(|row| row.identity == "copy-job:2")
            .unwrap()
            .pause_requested
    );
    h.commands.borrow_mut().clear();
    h.children(2);
    modeless::click(&progress, "Resume");
    assert!(matches!(
        h.commands.borrow().as_slice(),
        [Command::SetCopyPaused {
            id: OperationId(1),
            paused: false
        }]
    ));
    h.complete(1);
    h.commands.borrow_mut().clear();
    h.main.invoke_pause_job("copy-job:1".into(), true);
    h.main.invoke_pause_job("copy-job:99999".into(), true);
    assert!(
        h.commands.borrow().is_empty(),
        "finished or unknown identities have no pause authority"
    );
}

#[path = "native_job_windows/confirmation.rs"]
mod confirmation;
