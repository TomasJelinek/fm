#[path = "support/file_conflicts.rs"]
mod conflicts;
#[path = "support/owned_windows.rs"]
mod owned_windows;
#[path = "support/selection.rs"]
mod selection;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{accessibility::AccessibilityAction, items::AccessibleRole};
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{Key, PointerEventButton, WindowAdapter, WindowEvent},
};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

struct Harness {
    ui: ui::TestUi,
    windows: owned_windows::Windows,
    presenter: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    base: PathBuf,
}

impl Harness {
    fn new() -> Self {
        let (ui, windows) = owned_windows::new();
        let base = std::env::temp_dir().join("UI clipboard 文档 & $(literal)");
        let presenter = Rc::new(RefCell::new(Presenter::new(
            base.clone(),
            base.join("right"),
        )));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let sent = commands.clone();
        install(
            &ui.window,
            presenter.clone(),
            Rc::new(move |command| {
                sent.borrow_mut().push(command);
                true
            }),
        );
        let mut h = Self {
            ui,
            windows,
            presenter,
            commands,
            base,
        };
        h.complete();
        h.commands.borrow_mut().clear();
        h
    }

    fn complete(&mut self) {
        let navigation = self
            .commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: PaneId::Left,
                    navigation,
                    ..
                }
                | Command::Refresh {
                    pane: PaneId::Left,
                    navigation,
                    ..
                } => Some(*navigation),
                _ => None,
            })
            .unwrap();
        let entries: Arc<[Entry]> = vec![
            Entry::new(EntryId(1), self.base.join("a.txt"), EntryKind::File),
            Entry::new(EntryId(2), self.base.join("文件夹"), EntryKind::Directory),
            Entry::new(EntryId(3), self.base.join("c.txt"), EntryKind::File),
        ]
        .into();
        self.presenter.borrow_mut().apply(Event::ListingFinished {
            pane: PaneId::Left,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
        self.draw();
    }

    fn draw(&mut self) {
        refresh(&self.ui.window, &self.presenter.borrow());
        slint::platform::update_timers_and_animations();
        self.ui.draw();
    }

    fn apply(&mut self, event: Event) {
        let sent = self.commands.clone();
        self.presenter
            .borrow_mut()
            .apply_with_sender(event, &|command| {
                sent.borrow_mut().push(command);
                true
            });
        self.draw();
    }

    fn row(&self, row: usize) -> slint::LogicalPosition {
        let columns = self.ui.window.get_left_view_columns() as usize;
        let rows = self.ui.window.get_left_view_rows() as usize;
        let (column, band) = match self.ui.window.get_left_view_mode() {
            1 => (row / rows, row % rows),
            2 => (row % columns, row / columns),
            _ => (0, row),
        };
        slint::LogicalPosition::new(
            self.ui.window.get_left_list_left()
                + (column as f32 + 0.5) * self.ui.window.get_left_list_width() / columns as f32,
            self.ui.window.get_left_list_top()
                + self.ui.window.get_left_view_row_height() * (band as f32 + 0.5),
        )
    }

    fn right(&self, position: slint::LogicalPosition, down: bool) {
        self.ui
            .window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        self.ui.window.window().dispatch_event(if down {
            WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Right,
            }
        } else {
            WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Right,
            }
        });
    }

    fn menu_request(&self) -> DesktopRequestId {
        self.commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::ShowShellMenu { request, .. } => Some(*request),
                _ => None,
            })
            .expect("right click must request a system menu")
    }
}

#[test]
fn clipboard_shortcuts_follow_actual_file_focus_and_leave_text_editors_native() {
    let mut h = Harness::new();
    ui::click(&h.ui.window, h.row(1).x, h.row(1).y);
    ui::chord(&h.ui.window, &[Key::Control], "c");
    {
        let captured = h.commands.borrow();
        let Command::CopyFiles { files, .. } = captured.last().unwrap() else {
            panic!("file Ctrl+C must publish a file clipboard payload")
        };
        assert_eq!(files.fallback.as_ref(), Some(&h.base.join("a.txt")));
    }
    h.commands.borrow_mut().clear();
    ui::chord(&h.ui.window, &[Key::Control], "x");
    assert!(matches!(
        h.commands.borrow().last(),
        Some(Command::CutFiles { .. })
    ));
    h.commands.borrow_mut().clear();
    ui::chord(&h.ui.window, &[Key::Control], "v");
    assert!(matches!(h.commands.borrow().last(),
        Some(Command::ReadFileClipboard { destination, .. }) if destination == &h.base));
    h.commands.borrow_mut().clear();
    h.ui.window.invoke_set_command("editable text".into(), 13);
    ui::chord(&h.ui.window, &[Key::Control], "a");
    ui::chord(&h.ui.window, &[Key::Control], "c");
    ui::chord(&h.ui.window, &[Key::Control], "v");
    ui::chord(&h.ui.window, &[Key::Control], "x");
    assert!(
        h.commands.borrow().is_empty(),
        "command edit owns native clipboard shortcuts"
    );
    h.ui.window.invoke_focus_files();
    ui::key(&h.ui.window, "a");
    h.draw();
    h.commands.borrow_mut().clear();
    ui::chord(&h.ui.window, &[Key::Control], "c");
    ui::chord(&h.ui.window, &[Key::Control], "v");
    assert!(
        h.commands.borrow().iter().all(|command| !matches!(
            command,
            Command::CopyFiles { .. }
                | Command::CutFiles { .. }
                | Command::ReadFileClipboard { .. }
        )),
        "quick search owns its text clipboard while active"
    );
}

#[test]
fn native_cut_paste_requires_move_confirmation_before_shared_queue_admission() {
    let mut h = Harness::new();
    ui::click(&h.ui.window, h.row(1).x, h.row(1).y);
    ui::chord(&h.ui.window, &[Key::Control], "x");
    assert!(matches!(
        h.commands.borrow().last(),
        Some(Command::CutFiles { .. })
    ));
    h.commands.borrow_mut().clear();
    ui::chord(&h.ui.window, &[Key::Control], "v");
    let request = h
        .commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::ReadFileClipboard { request, .. } => Some(*request),
            _ => None,
        })
        .unwrap();
    h.commands.borrow_mut().clear();
    h.apply(Event::ClipboardFilesRead {
        request,
        destination: h.base.clone(),
        result: Ok(FileClipboard {
            paths: vec![h.base.join("a.txt")],
            intent: FileClipboardIntent::Move,
        }),
    });
    assert!(h.ui.window.get_copy_draft_move());
    assert!(!h.ui.window.get_copy_draft_token().is_empty());
    assert!(h.commands.borrow().iter().all(|command| !matches!(
        command,
        Command::EnqueueCopy(_) | Command::EnqueueMove(_) | Command::EnqueueClipboardMove(_)
    )));
    owned_windows::child_key(&h.windows, Key::Return);
    h.draw();
    let commands = h.commands.borrow();
    let request = commands
        .iter()
        .find_map(|command| match command {
            Command::EnqueueClipboardMove(request) => Some(request),
            _ => None,
        })
        .expect("explicit Enter must admit the captured clipboard Move");
    assert_eq!(request.clipboard.paths, vec![h.base.join("a.txt")]);
    assert_eq!(request.destination, h.base);
    assert!(h.ui.window.get_copy_draft_token().is_empty());
}

#[test]
fn actual_right_click_preserves_group_and_fallback_copy_uses_captured_selection() {
    let mut h = Harness::new();
    h.ui.window.invoke_bulk_selection(0, 0);
    let a = h.base.join("a.txt");
    let folder = h.base.join("文件夹");
    selection::acknowledge(&h.presenter, &h.commands, PaneId::Left, &[&a, &folder]);
    for mode in 0..3 {
        h.ui.window.invoke_view_mode(0, mode);
        h.draw();
        h.commands.borrow_mut().clear();
        let point = h.row(2);
        h.right(point, true);
        h.draw();
        h.right(point, false);
        let request = h.menu_request();
        {
            let captured = h.commands.borrow();
            let Command::ShowShellMenu {
                files,
                target,
                position,
                ..
            } = captured.last().unwrap()
            else {
                unreachable!()
            };
            assert_eq!(files.selection.len(), 2);
            assert_eq!(*target, ShellMenuTarget::Items);
            assert_eq!(position.x, point.x.round() as i32);
            assert_eq!(position.y, point.y.round() as i32);
        }
        h.apply(Event::ShellMenuFinished {
            request,
            result: Ok(ShellMenuOutcome::Unavailable),
        });
        h.commands.borrow_mut().clear();
        ui::click_popup(&h.ui.window, "Copy", AccessibleRole::Button);
        let captured = h.commands.borrow();
        let Command::CopyFiles { files, .. } = captured.last().unwrap() else {
            unreachable!()
        };
        assert_eq!(files.selection.len(), 2);
        assert!(files.selection.contains_path(&a));
        assert!(files.selection.contains_path(&folder));
    }
}

#[test]
fn row_refresh_invalidates_a_held_right_click_and_drive_picker_targets_its_root() {
    let mut h = Harness::new();
    let point = h.row(1);
    h.right(point, true);
    h.ui.window.invoke_action(0, 3);
    h.complete();
    h.commands.borrow_mut().clear();
    h.right(point, false);
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::ShowShellMenu { .. }))
    );
    let drive = h.base.ancestors().last().unwrap().to_owned();
    h.apply(Event::VolumesUpdated(vec![Volume {
        id: "test-drive".into(),
        label: "Test drive".into(),
        root: drive.clone(),
    }]));
    h.commands.borrow_mut().clear();
    let point = ui::root_position(&h.ui.window, "Drive or volume", AccessibleRole::Combobox);
    h.right(point, true);
    h.right(point, false);
    let captured = h.commands.borrow();
    let Command::ShowShellMenu { files, target, .. } = captured.last().unwrap() else {
        panic!("the closed drive picker needs a root context menu")
    };
    assert_eq!(*target, ShellMenuTarget::Drive);
    assert_eq!(files.fallback.as_ref(), Some(&drive));
    assert!(files.selection.is_empty());
}

#[test]
fn copy_conflict_native_choices_cancel_and_later_keep_exact_job_identity() {
    let mut h = conflicts::Harness::new();
    for (index, label) in ["Skip", "Skip all", "Copy as"].into_iter().enumerate() {
        let (id, version) = h.open(index as u64 + 4, false, false);
        if index == 2 {
            h.type_name("renamed 文档.txt");
        }
        let child = h.ui.child();
        h.ui.click(label);
        match (index, h.decision(id, version)) {
            (0, CopyChoice::Skip) | (1, CopyChoice::SkipAll) => {}
            (2, CopyChoice::Rename(name)) => assert_eq!(name, "renamed 文档.txt"),
            (_, choice) => panic!("{label} emitted {choice:?}"),
        }
        assert!(!child.window().is_visible());
    }

    let (id, version) = h.open(20, false, false);
    let child = h.ui.child();
    h.ui.click_owner(5.0, 5.0);
    h.ui.draw();
    assert!(
        child.window().is_visible(),
        "outside click leaves the modeless decision open"
    );
    assert!(h.commands.borrow().is_empty());
    h.ui.click("Decide later");
    assert!(!child.window().is_visible());
    assert!(
        h.commands.borrow().is_empty(),
        "Later must not send a decision"
    );
    h.refresh();
    assert_eq!(
        h.ui.visible_count(),
        h.ui.window
            .get_operations()
            .iter()
            .filter(|row| row.active)
            .count(),
        "operation progress remains; refresh must not reopen the deferred decision"
    );
    h.ui.window
        .invoke_resolve_copy(format!("copy-job:{}", id.0).into());
    h.ui.draw();
    let reopened = h.ui.child();
    assert!(!Rc::ptr_eq(&child, &reopened));
    assert_eq!(
        h.ui.window.get_copy_token(),
        format!("copy:{}:{version}", id.0)
    );
    h.ui.key(Key::Escape);
    assert!(matches!(h.decision(id, version), CopyChoice::Cancel));
    assert!(!reopened.window().is_visible());

    let (id, version) = h.open(21, false, false);
    let child = h.ui.child();
    child.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(matches!(h.decision(id, version), CopyChoice::Cancel));
    assert!(
        !child.window().is_visible(),
        "native X cancels this exact job"
    );
}

#[test]
fn copy_conflict_exposes_separate_recovery_and_merge_choices() {
    let mut h = conflicts::Harness::new();
    for (index, (replace, label)) in [
        (true, "Replace · keep original"),
        (true, "Replace all · keep originals"),
        (false, "Merge folders"),
        (false, "Merge all folders"),
    ]
    .into_iter()
    .enumerate()
    {
        let (id, version) = h.open(index as u64 + 2, replace, !replace);
        let child = h.ui.child();
        h.ui.click(label);
        match (index, h.decision(id, version)) {
            (0, CopyChoice::ReplaceRecoverable { all: false })
            | (1, CopyChoice::ReplaceRecoverable { all: true })
            | (2, CopyChoice::Merge { all: false })
            | (3, CopyChoice::Merge { all: true }) => {}
            (_, choice) => panic!("{label} emitted {choice:?}"),
        }
        assert!(!child.window().is_visible());
    }
}

#[test]
fn fallback_menu_escape_and_outside_click_release_the_captured_menu() {
    let mut h = Harness::new();
    for escape in [true, false] {
        let point = h.row(1);
        h.right(point, true);
        h.right(point, false);
        let request = h.menu_request();
        h.apply(Event::ShellMenuFinished {
            request,
            result: Ok(ShellMenuOutcome::Unavailable),
        });
        assert!(ui::has_popup(&h.ui.window));
        h.commands.borrow_mut().clear();
        if escape {
            ui::key(&h.ui.window, Key::Escape);
        } else {
            ui::click(&h.ui.window, 5.0, 5.0);
        }
        h.draw();
        assert!(!ui::has_popup(&h.ui.window));
        assert!(h.ui.window.get_file_menu_token().is_empty());
        assert!(
            h.commands.borrow().is_empty(),
            "dismissal never invokes a file action"
        );
    }
}

#[test]
fn fallback_menu_dismissal_blocks_batched_keyboard_and_accessibility_actions() {
    let mut h = Harness::new();
    for escape in [true, false] {
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        let point = h.row(1);
        h.right(point, true);
        h.right(point, false);
        h.apply(Event::ShellMenuFinished {
            request: h.menu_request(),
            result: Ok(ShellMenuOutcome::Unavailable),
        });
        let copy = ui::popup_item(&h.ui.window, "Copy", AccessibleRole::Button);
        h.commands.borrow_mut().clear();
        if escape {
            ui::key(&h.ui.window, Key::Escape);
            ui::key(&h.ui.window, Key::Return);
            ui::key(&h.ui.window, Key::Space);
        } else {
            h.ui.window
                .window()
                .dispatch_event(WindowEvent::WindowActiveChanged(false));
        }
        // Dismissal has reached the component, but the exact-token retirement
        // timer has not run. Even a previously captured accessibility item is inert.
        assert!(!h.ui.window.get_file_menu_token().is_empty());
        copy.accessible_action(&AccessibilityAction::Default);
        assert!(h.commands.borrow().is_empty());
        h.draw();
        assert!(!ui::has_popup(&h.ui.window));
        assert!(h.ui.window.get_file_menu_token().is_empty());
        assert!(h.commands.borrow().is_empty());
    }
}

#[test]
fn copy_progress_can_take_focus_from_a_fallback_menu_without_reentrant_borrow() {
    let mut h = Harness::new();
    h.ui.window.invoke_file_clipboard(0, 1);
    let request = match h.commands.borrow().last().unwrap() {
        Command::ReadFileClipboard { request, .. } => *request,
        _ => unreachable!(),
    };
    let point = h.row(1);
    h.right(point, true);
    h.right(point, false);
    h.apply(Event::ShellMenuFinished {
        request: h.menu_request(),
        result: Ok(ShellMenuOutcome::Unavailable),
    });
    assert!(ui::has_popup(&h.ui.window));
    h.apply(Event::ClipboardFilesRead {
        request,
        destination: h.base.clone(),
        result: Ok(FileClipboard {
            paths: vec![h.base.join("source.txt")],
            intent: FileClipboardIntent::Copy,
        }),
    });
    assert!(
        !h.ui.window.get_operations_visible(),
        "copy owns separate progress"
    );
    let children: Vec<_> = h
        .windows
        .borrow()
        .iter()
        .skip(1)
        .filter(|window| window.window().is_visible())
        .cloned()
        .collect();
    assert_eq!(children.len(), 1, "Jobs must open its own visible window");
    let jobs = &children[0];
    assert!(!Rc::ptr_eq(jobs, &h.ui.native));
    // Jobs uses a separate native window. MinimalSoftwareWindow does not emit
    // the OS activation handoff, unlike the former owner popup. Keep the
    // presenter borrowed while focus leaves to verify dismissal stays deferred.
    {
        let _presenter = h.presenter.borrow();
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(false));
        jobs.window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        assert!(!h.ui.window.get_file_menu_token().is_empty());
    }
    h.draw();
    assert!(!ui::has_popup(&h.ui.window));
    assert!(h.ui.window.get_file_menu_token().is_empty());
    assert!(jobs.window().is_visible());
    assert!(
        h.commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::EnqueueCopy(_)))
    );
}
