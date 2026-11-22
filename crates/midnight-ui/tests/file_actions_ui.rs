#[path = "support/file_conflicts.rs"]
mod conflicts;
#[allow(dead_code)]
#[path = "support/modeless.rs"]
mod modeless;
#[path = "support/owned_commands.rs"]
mod owned_commands;
#[path = "support/owned_windows.rs"]
mod owned_windows;
#[path = "support/selection.rs"]
mod selection;
use crate::support::ui;

use i_slint_core::{
    accessibility::{AccessibilityAction, AccessibleStringProperty},
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use midnight_core::*;
use midnight_ui::{BeginOwned, Presenter, refresh};
use owned_commands::install;
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
    begin: BeginOwned,
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
        let begin = install(
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
            begin,
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
        owned_commands::run(&self.presenter, &self.begin, |presenter, send| {
            presenter.apply_with_sender(event, send);
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
    clipboard_move(true);
}

#[test]
fn a_clipboard_move_keeps_links_unless_the_user_chooses_to_copy_their_files() {
    clipboard_move(false);
}

fn clipboard_move(follow_directory_links: bool) {
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
    assert_eq!(
        h.windows.borrow().len(),
        1,
        "Move confirmation must reuse the main adapter"
    );
    assert!(h.ui.window.get_embedded_modal_open());
    assert!(!h.ui.window.get_dialog_input_blocked());
    assert_eq!(
        WindowInner::from_pub(h.ui.window.window())
            .active_popups()
            .iter()
            .filter(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
            .count(),
        1,
        "Move confirmation must be an active main-window popup"
    );
    assert!(h.commands.borrow().iter().all(|command| !matches!(
        command,
        Command::EnqueueCopy(_)
            | Command::EnqueueMove(_)
            | Command::EnqueueClipboardMove(_)
            | Command::EnqueueCopyWithOptions(_)
    )));
    let native = h.windows.borrow()[0].clone();
    let links = modeless::accessible_popup_item(
        &native,
        AccessibleRole::Combobox,
        crate::support::link_option::LABEL,
    )
    .filter(|item| item.is_visible());
    assert_eq!(
        links.is_some(),
        crate::support::link_option::shown(true),
        "a clipboard Move shows the link choice as a Copy does"
    );
    if let Some(links) = links {
        // No backend answers which default the destination has: links stay links.
        assert_eq!(
            links
                .accessible_string_property(AccessibleStringProperty::Value)
                .unwrap(),
            crate::support::link_option::KEEP_LINKS
        );
        if follow_directory_links {
            // From the destination field, past the metadata option, to the choice.
            owned_windows::child_key(&h.windows, Key::Tab);
            owned_windows::child_key(&h.windows, Key::Tab);
            owned_windows::child_key(&h.windows, Key::UpArrow);
            assert_eq!(
                links
                    .accessible_string_property(AccessibleStringProperty::Value)
                    .unwrap(),
                crate::support::link_option::COPY_FILES
            );
        }
    }
    owned_windows::child_key(&h.windows, Key::Return);
    h.draw();
    let commands = h.commands.borrow();
    let options = commands
        .iter()
        .find_map(|command| match command {
            Command::EnqueueCopyWithOptions(options) => Some(options),
            _ => None,
        })
        .expect("explicit Enter must admit the captured clipboard Move");
    assert_eq!(
        options.links,
        crate::support::link_option::sent(true, follow_directory_links)
    );
    let CopyOptionsInput::Native(request) = &options.input else {
        panic!("clipboard Move requires its native request");
    };
    assert_eq!(request.intent, TransferIntent::Move);
    assert_eq!(request.metadata, TransferMetadataPolicy::ContentOnly);
    let MetadataTransferInput::Clipboard(request) = &request.input else {
        panic!("clipboard Move must preserve its captured clipboard");
    };
    assert_eq!(request.clipboard.paths, vec![h.base.join("a.txt")]);
    assert_eq!(request.destination, h.base);
    assert!(h.ui.window.get_copy_draft_token().is_empty());
    assert!(!h.ui.window.get_embedded_modal_open());
    assert!(
        WindowInner::from_pub(h.ui.window.window())
            .active_popups()
            .iter()
            .all(|popup| matches!(popup.window_kind, WindowKind::ToolTip)),
        "Enter must retire the admitted Move confirmation popup"
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
fn drive_menu_delivered_while_the_presenter_is_borrowed_opens_on_the_next_tick() {
    let mut h = Harness::new();
    let drive = h.base.ancestors().last().unwrap().to_owned();
    h.apply(Event::VolumesUpdated(vec![Volume {
        id: "test-drive".into(),
        label: "Test drive".into(),
        root: drive.clone(),
    }]));
    let token =
        h.ui.window
            .get_left_locations()
            .iter()
            .find(|row| row.is_drive)
            .expect("the drive button")
            .identity;
    h.commands.borrow_mut().clear();
    {
        // A refresh holds this shared borrow while Slint can still deliver input.
        let _refreshing = h.presenter.borrow();
        h.ui.window
            .invoke_drive_context_menu(0, token, 10.0, 10.0, false, false);
        assert!(
            h.commands.borrow().is_empty(),
            "nothing runs inside the borrow"
        );
    }
    slint::platform::update_timers_and_animations();
    let captured = h.commands.borrow();
    let Some(Command::ShowShellMenu { files, target, .. }) = captured.last() else {
        panic!("the deferred drive menu opens on the next tick: {captured:?}")
    };
    assert_eq!(*target, ShellMenuTarget::Drive);
    assert_eq!(files.fallback.as_ref(), Some(&drive));
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
    // Escape and the native X close only the window, like Decide later; the
    // footer Cancel job of the reopened decision cancels this exact job.
    h.ui.key(Key::Escape);
    assert!(!reopened.window().is_visible());
    assert!(
        h.commands.borrow().is_empty(),
        "Escape must not send a decision"
    );
    h.ui.window
        .invoke_resolve_copy(format!("copy-job:{}", id.0).into());
    h.ui.draw();
    h.ui.click("Cancel job");
    assert!(matches!(h.decision(id, version), CopyChoice::Cancel));

    let (id, version) = h.open(21, false, false);
    let child = h.ui.child();
    child.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(
        !child.window().is_visible(),
        "native X closes only the decision window"
    );
    assert!(
        h.commands.borrow().is_empty(),
        "native X must not send a decision"
    );
    h.ui.window
        .invoke_resolve_copy(format!("copy-job:{}", id.0).into());
    h.ui.draw();
    assert_eq!(
        h.ui.window.get_copy_token(),
        format!("copy:{}:{version}", id.0)
    );
    h.ui.click("Cancel job");
    assert!(matches!(h.decision(id, version), CopyChoice::Cancel));
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
fn busy_fallback_menu_action_releases_the_captured_menu() {
    let mut h = Harness::new();
    let point = h.row(1);
    h.right(point, true);
    h.right(point, false);
    h.apply(Event::ShellMenuFinished {
        request: h.menu_request(),
        result: Ok(ShellMenuOutcome::Unavailable),
    });
    assert!(ui::has_popup(&h.ui.window));
    h.commands.borrow_mut().clear();
    // Hold the fixture's only owned-command scope, as a full reclamation batch
    // does, so the chosen action cannot run.
    let held = (h.begin)().expect("no owned action is running");
    ui::click_popup(&h.ui.window, "Copy", AccessibleRole::Button);
    drop(held);
    h.draw();
    assert!(
        h.commands.borrow().is_empty(),
        "the refused action must not run"
    );
    assert_eq!(h.ui.window.get_message_title().as_str(), "File action busy");
    assert!(
        h.ui.window.get_file_menu_token().is_empty(),
        "the refused action must still release the captured menu"
    );
    ui::key(&h.ui.window, Key::Escape);
    h.draw();
    assert!(
        !ui::has_popup(&h.ui.window),
        "Escape closes the busy notice"
    );
    h.right(point, true);
    h.right(point, false);
    assert!(
        matches!(
            h.commands.borrow().last(),
            Some(Command::ShowShellMenu { .. })
        ),
        "a later right-click must open a new menu"
    );
}

/// An opaque captured source: the UI hands it on and never resolves its targets.
#[derive(Debug)]
struct OpaqueSource(FileClipboardIntent);
impl CapturedClipboardFiles for OpaqueSource {
    fn intent(&self) -> FileClipboardIntent {
        self.0
    }
    fn resolve(&self, _: &Cancellation) -> Result<FileClipboard, FsError> {
        panic!("the UI must not resolve captured tree targets")
    }
}

/// The tree-mode counterpart of `busy_fallback_menu_action_releases_the_captured_menu`:
/// a busy action cancels the captured tree menu, whose owner then waits in its slot
/// for the next owned action to retire it, so that action can open a new menu.
/// Meanwhile the released menu blocks no other context menu.
#[test]
fn busy_tree_fallback_menu_action_releases_the_captured_menu() {
    let mut h = Harness::new();
    let drive = h.base.ancestors().last().unwrap().to_owned();
    h.apply(Event::VolumesUpdated(vec![Volume {
        id: "test-drive".into(),
        label: "Test drive".into(),
        root: drive.clone(),
    }]));
    let disclosure = ui::root_position(
        &h.ui.window,
        "Discover folders in 文件夹",
        AccessibleRole::Button,
    );
    ui::click(&h.ui.window, disclosure.x, disclosure.y);
    let root = h
        .commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::Tree(command) => match command.as_ref() {
                TreeCommand::Enable { root, .. } => Some(root.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("the disclosure must request a backend tree session");
    let folder = root
        .entries
        .iter()
        .find(|entry| entry.kind == EntryKind::Directory)
        .expect("the listing has a folder")
        .clone();
    let mut display = folder.clone();
    display.id = EntryId(7);
    let stamp = TreeViewStamp {
        pane: PaneId::Left,
        navigation: root.navigation,
        session: TreeSessionId(19),
        revision: TreeRevision(1),
    };
    let row = TreeDisplayRow {
        id: TreeRowId(7),
        entry: display,
        parent: None,
        depth: 0,
        disclosure: TreeDisclosure::Unknown,
        expanded: false,
        selectable: true,
        navigable: true,
        context_only: false,
        error: None,
        error_details: None,
        original_root_entry: Some(folder.id),
        prefix_end: 0,
        search_match: false,
    };
    let view =
        Arc::new(TreeProjection::new(root, stamp, TreeLayoutRevision(1), vec![row]).unwrap());
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    h.apply(Event::Tree(Box::new(TreeEvent::Published(
        TreePublication::new(view, SelectionRevision(0), selection).unwrap(),
    ))));
    let position =
        h.ui.window
            .get_left_rows()
            .iter()
            .position(|row| row.name == "文件夹")
            .expect("the adopted tree shows the folder");
    let point = h.row(position);
    let menu_capture = |h: &Harness| {
        h.commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Tree(command) => match command.as_ref() {
                    TreeCommand::Clipboard(request) => Some(request.clone()),
                    _ => None,
                },
                _ => None,
            })
    };
    h.commands.borrow_mut().clear();
    h.right(point, true);
    h.right(point, false);
    let capture = menu_capture(&h).expect("a tree right-click captures its occurrence");
    assert_eq!(capture.scope, TreeClipboardScope::Dragged(TreeRowId(7)));
    h.apply(Event::Tree(Box::new(TreeEvent::Clipboard {
        stamp: capture.view.stamp,
        request: capture.request,
        result: Ok(Arc::new(OpaqueSource(capture.intent))),
    })));
    let request = h
        .commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::ShowCapturedShellMenu { request, .. } => Some(*request),
            _ => None,
        })
        .expect("the captured occurrence opens the system menu");
    h.apply(Event::ShellMenuFinished {
        request,
        result: Ok(ShellMenuOutcome::Unavailable),
    });
    assert!(ui::has_popup(&h.ui.window));
    assert!(!h.ui.window.get_file_menu_token().is_empty());
    h.commands.borrow_mut().clear();
    // Hold the fixture's only owned-command scope, as a full reclamation batch
    // does, so the chosen action cannot run.
    let held = (h.begin)().expect("no owned action is running");
    ui::click_popup(&h.ui.window, "Copy", AccessibleRole::Button);
    // The batch stays full while the deferred dismissal runs, so no owned action
    // can retire the released owner yet.
    h.draw();
    assert!(
        h.commands.borrow().is_empty(),
        "the refused action must not run"
    );
    assert_eq!(h.ui.window.get_message_title().as_str(), "File action busy");
    assert!(
        h.ui.window.get_file_menu_token().is_empty(),
        "the refused action must still release the captured tree menu"
    );
    assert!(
        capture.cancel.is_cancelled(),
        "the released tree menu cancels its capture"
    );
    // The released owner waits in its slot. A drive menu is not an owned action
    // and must not be refused because of it (the busy notice blocks pointer input,
    // so the picker's callback is invoked directly).
    let token =
        h.ui.window
            .get_left_locations()
            .iter()
            .find(|row| row.is_drive)
            .expect("the drive button")
            .identity;
    h.ui.window
        .invoke_drive_context_menu(0, token, 10.0, 10.0, false, false);
    let drive_menu = match h.commands.borrow().last() {
        Some(Command::ShowShellMenu {
            request,
            target: ShellMenuTarget::Drive,
            files,
            ..
        }) => {
            assert_eq!(files.fallback.as_ref(), Some(&drive));
            *request
        }
        other => panic!("the drive picker must open its root menu: {other:?}"),
    };
    drop(held);
    h.apply(Event::ShellMenuFinished {
        request: drive_menu,
        result: Ok(ShellMenuOutcome::Dismissed),
    });
    ui::key(&h.ui.window, Key::Escape);
    h.draw();
    assert!(
        !ui::has_popup(&h.ui.window),
        "Escape closes the busy notice"
    );
    h.right(point, true);
    h.right(point, false);
    let next = menu_capture(&h).expect("the next right-click must open a new tree menu");
    assert_eq!(next.scope, TreeClipboardScope::Dragged(TreeRowId(7)));
    assert!(!next.cancel.is_cancelled());
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
    assert_eq!(
        children.len(),
        1,
        "the copy must open its own visible progress window"
    );
    let progress = &children[0];
    assert!(!Rc::ptr_eq(progress, &h.ui.native));
    // The progress window is a separate native window. MinimalSoftwareWindow does
    // not emit the OS activation handoff, unlike the former owner popup. Keep the
    // presenter borrowed while focus leaves to verify dismissal stays deferred.
    {
        let _presenter = h.presenter.borrow();
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(false));
        progress
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        assert!(!h.ui.window.get_file_menu_token().is_empty());
    }
    h.draw();
    assert!(!ui::has_popup(&h.ui.window));
    assert!(h.ui.window.get_file_menu_token().is_empty());
    assert!(progress.window().is_visible());
    assert!(
        h.commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::EnqueueCopy(_)))
    );
}
