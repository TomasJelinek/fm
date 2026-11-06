#[path = "support/selection.rs"]
mod selection;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{
    ComponentHandle,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

struct Harness {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    base: PathBuf,
}

impl Harness {
    fn new() -> Self {
        let ui = ui::TestUi::new();
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
    ui::chord(&h.ui.window, &[Key::Control], "v");
    assert!(matches!(h.commands.borrow().last(),
        Some(Command::ReadFileClipboard { destination, .. }) if destination == &h.base));
    h.commands.borrow_mut().clear();
    h.ui.window.invoke_set_command("editable text".into(), 13);
    ui::chord(&h.ui.window, &[Key::Control], "a");
    ui::chord(&h.ui.window, &[Key::Control], "c");
    ui::chord(&h.ui.window, &[Key::Control], "v");
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
            Command::CopyFiles { .. } | Command::ReadFileClipboard { .. }
        )),
        "quick search owns its text clipboard while active"
    );
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
fn copy_conflict_controls_dispatch_versioned_choices_and_escape_defers() {
    let mut test = ui::TestUi::new();
    let decisions = Rc::new(RefCell::new(Vec::new()));
    let observed = decisions.clone();
    test.window.on_decide_copy(move |token, choice, name| {
        observed
            .borrow_mut()
            .push((token.to_string(), choice, name.to_string()));
    });
    test.window.set_copy_token("copy:9:4".into());
    test.window
        .set_copy_body("Source and existing destination".into());
    test.window.set_copy_name("original.txt".into());
    for (label, choice) in [("Skip", 0), ("Skip all", 1), ("Copy as", 2)] {
        test.window.invoke_show_copy_conflict();
        test.draw();
        if choice == 2 {
            ui::click_popup(
                &test.window,
                "Copy under a different name",
                AccessibleRole::TextInput,
            );
            ui::chord(&test.window, &[Key::Control], "a");
            ui::key(&test.window, "renamed 文档.txt");
        }
        ui::click_popup(&test.window, label, AccessibleRole::Button);
        assert_eq!(
            decisions.borrow().last(),
            Some(&(
                "copy:9:4".into(),
                choice,
                if choice == 2 {
                    "renamed 文档.txt".into()
                } else {
                    String::new()
                }
            ))
        );
        test.window.invoke_close_copy_conflict();
    }
    decisions.borrow_mut().clear();
    test.window.invoke_show_copy_conflict();
    test.draw();
    ui::key(&test.window, Key::Escape);
    assert!(
        decisions.borrow().is_empty(),
        "Escape keeps the job waiting for a decision"
    );
    assert!(!ui::has_popup(&test.window));
    assert_eq!(test.window.get_copy_token(), "copy:9:4");
    test.window.invoke_show_copy_conflict();
    test.draw();
    ui::click(&test.window, 5.0, 5.0);
    assert!(!ui::has_popup(&test.window));
    assert!(decisions.borrow().is_empty());
    test.window.invoke_focus_files();
    assert_eq!(test.window.get_focused_pane(), 0);
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
    assert!(h.ui.window.get_file_menu_token().is_empty());
    assert!(
        h.commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::EnqueueCopy(_)))
    );
}
