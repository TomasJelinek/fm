//! Embedded operation forms retain the backend target and the exact UI occurrence.
use crate::support::modeless;
#[path = "support/owned_commands.rs"]
mod owned_commands;
use crate::support::ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use midnight_core::*;
use midnight_ui::{BeginOwned, MainWindow, OperationPresentation, Presenter, refresh};
use owned_commands::install;
use slint::{
    ComponentHandle, Model, SharedString,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

struct Harness {
    main: MainWindow,
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    state: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    accepting: Rc<Cell<bool>>,
    paths: [PathBuf; 2],
    begin: BeginOwned,
}

impl Harness {
    fn new() -> Self {
        Self::with_root(std::env::temp_dir().join("native-operation-targets 文档"))
    }

    /// Panes at `root/left` and `root/right`; no filesystem access happens.
    fn with_root(root: PathBuf) -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
        let main = MainWindow::new().unwrap();
        midnight_ui::install_file_keys(&main);
        let paths = [root.join("left"), root.join("right")];
        let state = Rc::new(RefCell::new(Presenter::new(
            paths[0].clone(),
            paths[1].clone(),
        )));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let captured = commands.clone();
        let accepting = Rc::new(Cell::new(true));
        let gate = accepting.clone();
        let begin = install(
            &main,
            state.clone(),
            Rc::new(move |command| {
                if !gate.get() {
                    return false;
                }
                captured.borrow_mut().push(command);
                true
            }),
        );
        for pane in [PaneId::Left, PaneId::Right] {
            let navigation = commands
                .borrow()
                .iter()
                .find_map(|command| match command {
                    Command::Navigate {
                        pane: target,
                        navigation,
                        ..
                    } if *target == pane => Some(*navigation),
                    _ => None,
                })
                .unwrap();
            let entries: Arc<[Entry]> = vec![Entry::new(
                EntryId(1),
                paths[pane.index()].join("source 文档.txt"),
                EntryKind::File,
            )]
            .into();
            state.borrow_mut().apply(Event::ListingFinished {
                pane,
                navigation,
                index: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
            });
        }
        refresh(&main, &state.borrow());
        main.set_preferences_ready(true);
        main.show().unwrap();
        main.window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        modeless::draw(&windows.borrow()[0], 1000, 700, "operation-owner");
        commands.borrow_mut().clear();
        Self {
            main,
            windows,
            state,
            commands,
            accepting,
            paths,
            begin,
        }
    }

    fn focus(&self, pane: PaneId) {
        self.main.invoke_activate(pane.index() as i32);
        let rows = if pane == PaneId::Left {
            self.main.get_left_rows()
        } else {
            self.main.get_right_rows()
        };
        let row = rows
            .iter()
            .position(|row| row.name == "source 文档.txt")
            .unwrap();
        self.main
            .invoke_row_click(pane.index() as i32, row as i32, false, false);
        self.main.invoke_focus_files();
        self.commands.borrow_mut().clear();
    }

    fn child(&self) -> Rc<MinimalSoftwareWindow> {
        slint::platform::update_timers_and_animations();
        let children: Vec<_> = self
            .windows
            .borrow()
            .iter()
            .skip(1)
            .filter(|window| window.window().is_visible())
            .cloned()
            .collect();
        assert_eq!(
            children.len(),
            0,
            "blocking operation forms must stay inside the main window adapter"
        );
        assert!(
            self.main.get_embedded_modal_open(),
            "the embedded form must gate background application input"
        );
        assert!(
            !self.main.get_dialog_input_blocked(),
            "the main input host must stay enabled"
        );
        assert!(
            WindowInner::from_pub(self.main.window())
                .active_popups()
                .len()
                == 1
        );
        let child = self.windows.borrow()[0].clone();
        modeless::draw(&child, 640, 420, "operation-embedded");
        child
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        child
    }

    fn operation(&self, pane: PaneId, action: i32) -> (OperationId, Rc<MinimalSoftwareWindow>) {
        self.focus(pane);
        self.main.invoke_mutation(pane.index() as i32, action);
        let request = self
            .commands
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::PrepareOperation(request) => Some(request.clone()),
                _ => None,
            })
            .expect("preparation must capture a backend operation before presenting the form");
        assert_eq!(request.source.pane, pane);
        assert_eq!(request.source.path, self.paths[pane.index()]);
        assert_eq!(request.focused, Some(EntryId(1)));
        let preview = OperationPreview {
            id: request.id,
            kind: request.kind,
            files: 1,
            folders: 0,
            file_bytes: Some(17),
            sample: vec![MutationItem {
                path: self.paths[pane.index()].join("source 文档.txt"),
                kind: EntryKind::File,
                metadata: EntryMetadata::default(),
            }],
        };
        owned_commands::run(&self.state, &self.begin, |state, _| {
            state.apply(Event::Operation(OperationEvent::Preview(Arc::new(preview))));
        });
        refresh(&self.main, &self.state.borrow());
        let child = self.child();
        self.commands.borrow_mut().clear();
        (request.id, child)
    }

    fn transfer(&self, pane: PaneId, moving: bool) -> Rc<MinimalSoftwareWindow> {
        self.focus(pane);
        ui::key(&self.main, if moving { Key::F6 } else { Key::F5 });
        assert_eq!(self.main.get_copy_draft_move(), moving);
        assert!(!self.main.get_copy_draft_token().is_empty());
        // Opening a draft asks only what its destination does with links.
        assert!(
            self.commands
                .borrow()
                .iter()
                .all(crate::support::link_option::inspection),
            "opening a draft must not enqueue a transfer"
        );
        self.child()
    }

    /// Answer the check of the typed destination `path` as the backend does: the
    /// folder exists. Only what the answer sends (the transfer) is kept.
    fn answer_destination_check(&self, path: &std::path::Path) {
        let check = self
            .commands
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::CheckTransferDestination(request) => Some(request.clone()),
                _ => None,
            })
            .expect("a typed destination is checked before the transfer starts");
        assert_eq!(check.path, path);
        self.commands.borrow_mut().clear();
        owned_commands::run(&self.state, &self.begin, |state, _| {
            state.apply_with_sender(
                Event::TransferDestinationChecked(Box::new(DestinationCheckResult {
                    request: check.request,
                    path: check.path.clone(),
                    result: Ok(true),
                    links: None,
                })),
                &|command| {
                    self.commands.borrow_mut().push(command);
                    true
                },
            );
        });
        refresh(&self.main, &self.state.borrow());
    }

    fn assert_closed(&self, child: &MinimalSoftwareWindow) {
        slint::platform::update_timers_and_animations();
        assert!(child.window().is_visible());
        assert!(
            WindowInner::from_pub(child.window())
                .active_popups()
                .is_empty()
        );
        assert!(self.main.window().is_visible());
        assert!(!self.main.get_embedded_modal_open());
        assert!(!self.main.get_dialog_input_blocked());
    }
}

fn key(window: &MinimalSoftwareWindow, text: impl Into<SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn replace(window: &MinimalSoftwareWindow, text: &str) {
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    key(window, "a");
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    if text.is_empty() {
        key(window, Key::Backspace);
    } else {
        key(window, text);
    }
}

fn item(window: &MinimalSoftwareWindow, label: &str, role: AccessibleRole) -> ItemRc {
    modeless::accessible_popup_item(window, role, label)
        .or_else(|| modeless::accessible_item(window, role, label))
        .unwrap_or_else(|| panic!("missing {label}"))
}

fn value(window: &MinimalSoftwareWindow, label: &str) -> SharedString {
    item(window, label, AccessibleRole::TextInput)
        .accessible_string_property(AccessibleStringProperty::Value)
        .unwrap()
}

fn assert_visible(window: &MinimalSoftwareWindow, label: &str, role: AccessibleRole) {
    let item = item(window, label, role);
    assert!(item.is_visible(), "{label} is hidden");
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    let size = window.window().size();
    assert!(rect.size.width > 1.0 && rect.size.height > 1.0);
    assert!(
        origin.x >= -0.5
            && origin.y >= -0.5
            && origin.x + rect.size.width <= size.width as f32 + 0.5
            && origin.y + rect.size.height <= size.height as f32 + 0.5,
        "{label} escapes native window {size:?}: {origin:?} {:?}",
        rect.size
    );
}

#[test]
fn embedded_operation_blocks_background_shortcuts_and_pointer_actions() {
    let h = Harness::new();
    let (id, main) = h.operation(PaneId::Left, 2);
    let active = h.main.get_active_pane();
    for shortcut in [Key::F2, Key::F5, Key::F6, Key::F7, Key::F8, Key::Delete] {
        key(&main, shortcut);
    }
    modeless::click(&main, "F5 Copy");
    assert!(h.commands.borrow().is_empty());
    assert_eq!(h.main.get_active_pane(), active);
    assert_eq!(h.main.get_operation_token(), id.0.to_string());
    assert!(h.main.global::<OperationPresentation>().get_active());
    assert_eq!(h.windows.borrow().len(), 1);
    key(&main, Key::Escape);
    assert!(matches!(h.commands.borrow().as_slice(),
        [Command::CancelOperation(actual)] if *actual == id));
    h.assert_closed(&main);
}

#[test]
fn native_operation_rename_retains_exact_target_text_and_caret_across_resize_and_refresh() {
    let h = Harness::new();
    for pane in [PaneId::Left, PaneId::Right] {
        let (id, child) = h.operation(pane, 0);
        key(&child, "renamed 文档"); // The initial basename is selected, extension preserved.
        assert_eq!(value(&child, "New name"), "renamed 文档.txt");
        key(&child, Key::End);
        key(&child, Key::LeftArrow);
        h.main.invoke_activate(1 - pane.index() as i32);
        modeless::draw(&child, 320, 200, "rename-compact");
        modeless::draw(&h.windows.borrow()[0], 1350, 850, "renamed-owner-resize");
        refresh(&h.main, &h.state.borrow());
        assert_eq!(value(&child, "New name"), "renamed 文档.txt");
        key(&child, "X");
        key(&child, Key::Return);
        assert!(matches!(h.commands.borrow().as_slice(),
            [Command::ConfirmOperation {id: actual, new_name: Some(name), ..}]
            if *actual == id && name == "renamed 文档.txXt"));
        h.assert_closed(&child);
        h.main.global::<OperationPresentation>().invoke_confirm(
            h.main.global::<OperationPresentation>().get_token(),
            "stale".into(),
        );
        assert_eq!(
            h.commands.borrow().len(),
            1,
            "a retired occurrence cannot confirm twice"
        );
    }
}

#[test]
fn native_operation_create_rejects_empty_then_accepts_unicode_and_parent_segments() {
    let h = Harness::new();
    for (pane, action, label, name) in [
        (PaneId::Left, 3, "Folder name or path", "../new 文档/inside"),
        (PaneId::Right, 4, "File name", "created 文档.txt"),
    ] {
        let (id, child) = h.operation(pane, action);
        key(&child, Key::Return);
        assert!(h.commands.borrow().is_empty(), "empty name must not submit");
        key(&child, name);
        assert_eq!(value(&child, label), name);
        key(&child, Key::Return);
        assert!(matches!(h.commands.borrow().as_slice(),
            [Command::ConfirmOperation {id: actual, new_name: Some(actual_name), ..}]
            if *actual == id && actual_name == name));
        h.assert_closed(&child);
    }
}

#[test]
fn native_operation_delete_escape_close_and_focused_cancel_never_confirm() {
    let h = Harness::new();
    for (index, action) in [1, 2, 2].into_iter().enumerate() {
        let (id, child) = h.operation(PaneId::Left, action);
        match index {
            0 => key(&child, Key::Escape),
            1 => child.window().dispatch_event(WindowEvent::CloseRequested),
            _ => {
                key(&child, Key::Tab);
                key(&child, Key::Return);
            }
        }
        assert!(
            matches!(h.commands.borrow().as_slice(), [Command::CancelOperation(actual)] if *actual == id)
        );
        h.assert_closed(&child);
    }
    let (id, child) = h.operation(PaneId::Right, 2);
    key(&child, Key::Return);
    assert!(matches!(h.commands.borrow().as_slice(),
        [Command::ConfirmOperation {id: actual, new_name: None, ..}] if *actual == id));
    h.assert_closed(&child);
}

#[test]
fn native_operation_transfer_keeps_captured_pane_and_edited_destination_on_resize_and_confirm() {
    let h = Harness::new();
    for pane in [PaneId::Left, PaneId::Right] {
        for moving in [false, true] {
            let child = h.transfer(pane, moving);
            let destination = h.paths[1 - pane.index()].join("edited 文档 target");
            replace(&child, destination.to_str().unwrap());
            h.main.invoke_activate(1 - pane.index() as i32);
            modeless::draw(&child, 320, 200, "transfer-compact");
            refresh(&h.main, &h.state.borrow());
            assert_eq!(
                value(&child, "Destination folder"),
                destination.to_str().unwrap()
            );
            key(&child, Key::Return);
            h.answer_destination_check(&destination);
            let commands = h.commands.borrow();
            let [Command::EnqueueCopyWithOptions(options)] = commands.as_slice() else {
                panic!("the frozen options must enqueue exactly once");
            };
            // No backend answers which default the destination has: links stay links.
            assert_eq!(
                options.links,
                crate::support::link_option::sent(moving, false)
            );
            let CopyOptionsInput::Native(request) = &options.input else {
                panic!("the dialog must keep its native selection");
            };
            let intent = if moving {
                TransferIntent::Move
            } else {
                TransferIntent::Copy
            };
            assert_eq!(request.intent, intent);
            let MetadataTransferInput::Selected(request) = &request.input else {
                panic!("the dialog must keep its selected sources");
            };
            assert_eq!(request.destination, destination);
            assert_eq!(
                request.files.fallback,
                Some(h.paths[pane.index()].join("source 文档.txt"))
            );
            let id = request.id;
            drop(commands);
            h.assert_closed(&child);
            // The modeless child may get Escape before the first actual progress
            // event. The fixture supplies registration and checkpoint snapshots.
            assert!(
                !h.main.get_operations_visible(),
                "progress is independent from Jobs"
            );
            let jobs: Vec<_> = h
                .windows
                .borrow()
                .iter()
                .skip(1)
                .filter(|window| window.window().is_visible())
                .cloned()
                .collect();
            assert_eq!(
                jobs.len(),
                1,
                "only progress remains after the form is accepted"
            );
            assert!(!Rc::ptr_eq(&child, &jobs[0]), "Jobs has its own adapter");
            modeless::draw(&jobs[0], 720, 500, "transfer-jobs");
            assert!(
                modeless::accessible_item(&jobs[0], AccessibleRole::Button, "Cancel operation")
                    .is_some()
            );
            h.commands.borrow_mut().clear();
            key(&jobs[0], Key::Escape);
            assert!(jobs[0].window().is_visible());
            assert!(
                h.commands.borrow().is_empty(),
                "the queued placeholder is not registered"
            );
            let apply = |progress: CopyProgress| {
                owned_commands::run(&h.state, &h.begin, |presenter, send| {
                    presenter.apply_with_sender(
                        Event::Copy(CopyEvent::Progress(Arc::new(progress))),
                        send,
                    );
                });
                refresh(&h.main, &h.state.borrow());
                // A running job also asks for the panels' free space again
                // (presenter/tests/free_space.rs); this test follows the hold.
                h.commands
                    .borrow_mut()
                    .retain(|command| !matches!(command, Command::RefreshSpace { .. }));
            };
            let mut progress = CopyProgress::queued(id, destination, 1, intent);
            progress.state = CopyOperationState::Running;
            apply(progress.clone());
            let token = match h.commands.borrow().as_slice() {
                [
                    Command::CopyCancelConfirmation {
                        id: actual,
                        token,
                        action: CopyCancelConfirmationAction::Begin,
                    },
                ] if *actual == id => *token,
                other => panic!("registration must start the exact hold: {other:?}"),
            };
            h.commands.borrow_mut().clear();
            progress.pause_requested = true;
            progress.paused = true;
            progress.cancel_confirmation_ready = Some(token);
            apply(progress.clone());
            modeless::draw(&jobs[0], 720, 500, "transfer-cancel-question");
            assert_visible(&jobs[0], "Continue", AccessibleRole::Button);
            modeless::click(&jobs[0], "Cancel operation");
            assert!(matches!(h.commands.borrow().as_slice(),
                [Command::CopyCancelConfirmation { id: actual, token: confirmed, action: CopyCancelConfirmationAction::Confirm }]
                if *actual == id && *confirmed == token));
            progress.state = CopyOperationState::Cancelled;
            progress.pause_requested = false;
            progress.paused = false;
            progress.cancel_confirmation_ready = None;
            progress.closure = Some(CopyClosureReceipt {
                unfinished_roots: 1,
                uncommitted_files: None,
                uncommitted_bytes: None,
                omitted_error_details: 0,
            });
            apply(progress);
            assert!(jobs[0].window().is_visible());
            modeless::draw(&jobs[0], 720, 500, "transfer-cancelled");
            key(&jobs[0], Key::Escape);
            assert!(!jobs[0].window().is_visible());
            assert!(!h.main.get_operations_visible());
            assert!(
                WindowInner::from_pub(h.main.window())
                    .active_popups()
                    .is_empty()
            );
        }
    }
}

#[test]
fn native_operation_rejected_confirmation_keeps_same_child_and_edited_name_for_retry() {
    let h = Harness::new();
    let (id, child) = h.operation(PaneId::Left, 0);
    replace(&child, "retry 文档.txt");
    h.accepting.set(false);
    key(&child, Key::Return);
    assert!(h.commands.borrow().is_empty());
    assert!(child.window().is_visible());
    assert_eq!(h.windows.borrow().len(), 1);
    refresh(&h.main, &h.state.borrow());
    assert_eq!(value(&child, "New name"), "retry 文档.txt");
    h.accepting.set(true);
    key(&child, Key::Return);
    assert!(matches!(h.commands.borrow().as_slice(),
        [Command::ConfirmOperation {id: actual, new_name: Some(name), ..}]
        if *actual == id && name == "retry 文档.txt"));
    h.assert_closed(&child);
}

#[test]
fn native_operation_controls_remain_reachable_at_320_by_200_across_appearance_preferences() {
    let h = Harness::new();
    for theme in [PreferenceTheme::Light, PreferenceTheme::Dark] {
        for density in [UiDensity::Desktop, UiDensity::Touch] {
            for font in [11, 24] {
                let mut preferences = UserPreferences {
                    theme,
                    density,
                    desktop_font_size: font,
                    touch_font_size: font,
                    ..Default::default()
                };
                preferences.tabs[0].paths = vec![h.paths[0].clone()];
                preferences.tabs[1].paths = vec![h.paths[1].clone()];
                h.state
                    .borrow_mut()
                    .apply(Event::PreferencesLoaded(Ok(Box::new(
                        midnight_core::AcceptedPreferences {
                            preferences,
                            comparators: midnight_core::FileComparatorSnapshot::default(),
                        },
                    ))));
                refresh(&h.main, &h.state.borrow());
                let (_, child) = h.operation(PaneId::Left, 0);
                modeless::draw(
                    &child,
                    320,
                    200,
                    &format!("native-rename-{theme:?}-{density:?}-{font}"),
                );
                for (label, role) in [
                    ("New name", AccessibleRole::TextInput),
                    ("Cancel", AccessibleRole::Button),
                    ("Rename", AccessibleRole::Button),
                ] {
                    assert_visible(&child, label, role);
                }
                modeless::click(&child, "Cancel");
                h.assert_closed(&child);
                let child = h.transfer(PaneId::Right, true);
                modeless::draw(
                    &child,
                    320,
                    200,
                    &format!("native-move-{theme:?}-{density:?}-{font}"),
                );
                for (label, role) in [
                    ("Destination folder", AccessibleRole::TextInput),
                    ("Cancel", AccessibleRole::Button),
                    ("Move", AccessibleRole::Button),
                ] {
                    assert_visible(&child, label, role);
                }
                modeless::click(&child, "Cancel");
                h.assert_closed(&child);
            }
        }
    }
}

fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (
        origin.x,
        origin.y,
        origin.x + rect.width(),
        origin.y + rect.height(),
    )
}

/// The Copy/Move draft's first line names the captured item and ends right above its
/// destination field, which stays a single line at every density and font. (The layout
/// of every state at every size is checked in copy_dialog_polish.)
#[test]
fn embedded_copy_draft_names_its_item_right_above_a_single_line_destination() {
    let h = Harness::new();
    let child = h.transfer(PaneId::Left, false);
    let line = h.main.global::<OperationPresentation>().get_body();
    assert!(
        line.starts_with("Copy \u{201c}")
            && line.contains("source 文档.txt")
            && line.ends_with(" to:"),
        "the draft names its captured item: {line}"
    );
    for (density, font) in [(0, 13), (1, 15), (0, 22)] {
        h.main.invoke_apply_fonts("".into(), font, font);
        h.main.invoke_apply_preferences(3, density);
        modeless::draw(
            &child,
            720,
            520,
            &format!("embedded-copy-line-{density}-{font}"),
        );
        let first = item(&child, &line, AccessibleRole::Text);
        let editor = item(&child, "Destination folder", AccessibleRole::TextInput);
        let (.., line_bottom) = bounds(&first);
        let (_, editor_top, ..) = bounds(&editor);
        assert!(first.is_visible() && editor.is_visible());
        assert!(
            line_bottom <= editor_top + 0.5 && editor_top - line_bottom < 10.0,
            "the first line must sit right above the destination: line ends {line_bottom}, field starts {editor_top}, font={font}, density={density}"
        );
        assert!(
            editor.geometry().height() < font as f32 * 3.0,
            "the destination stays single-line: font={font}, density={density}"
        );
    }
    h.main.invoke_apply_fonts("".into(), 13, 15);
    h.main.invoke_apply_preferences(3, 0);
    modeless::draw(&child, 320, 200, "embedded-copy-line-compact");
    for (label, role) in [
        ("Destination folder", AccessibleRole::TextInput),
        ("Cancel", AccessibleRole::Button),
        ("Copy", AccessibleRole::Button),
    ] {
        assert_visible(&child, label, role);
    }
    modeless::click(&child, "Cancel");
    h.assert_closed(&child);
}
#[test]
fn native_operation_rejected_close_retains_modal_child_and_never_turns_into_confirmation() {
    let h = Harness::new();
    let (id, child) = h.operation(PaneId::Left, 2);
    h.accepting.set(false);
    child.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(child.window().is_visible());
    assert!(h.main.get_embedded_modal_open());
    assert!(!h.main.get_dialog_input_blocked());
    assert_eq!(h.windows.borrow().len(), 1);
    assert!(h.commands.borrow().is_empty());
    assert_eq!(
        item(&child, "Permanently delete", AccessibleRole::Button)
            .accessible_string_property(AccessibleStringProperty::Enabled)
            .unwrap(),
        "false",
        "after requesting cancellation, Return must never confirm destructive work"
    );
    h.accepting.set(true);
    key(&child, Key::Return);
    assert!(
        h.commands.borrow().is_empty(),
        "Return neither confirms nor silently retries the refused cancellation"
    );
    assert!(
        h.main.global::<OperationPresentation>().get_active(),
        "the refused close keeps the form until an explicit retry"
    );
    // A second explicit close retries the same cancellation identity.
    child.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(
        matches!(h.commands.borrow().as_slice(), [Command::CancelOperation(actual)] if *actual == id)
    );
    h.assert_closed(&child);
}

#[test]
fn native_operation_copy_cancel_restores_invoking_command_editor() {
    let h = Harness::new();
    h.focus(PaneId::Left);
    for native_close in [false, true] {
        h.main.invoke_set_command("draft".into(), 2);
        let inner = WindowInner::from_pub(h.main.window());
        let invoking = inner.focus_item.borrow().upgrade().unwrap();
        // Invoke the semantic F5 route without a file-pane key changing focus.
        h.main.invoke_file_clipboard(0, 2);
        let child = h.child();
        if native_close {
            child.window().dispatch_event(WindowEvent::CloseRequested);
        } else {
            key(&child, Key::Escape);
        }
        h.assert_closed(&child);
        assert!(
            inner
                .focus_item
                .borrow()
                .upgrade()
                .is_some_and(|item| item == invoking),
            "closing the native form replaced its invoking text editor with file-pane focus"
        );
        ui::key(&h.main, "x");
        assert_eq!(value(&h.windows.borrow()[0], "Command line"), "drxaft");
        // Opening the draft asked only what its destination does with links.
        assert!(
            h.commands
                .borrow()
                .iter()
                .all(crate::support::link_option::inspection),
            "cancellation enqueued a transfer"
        );
    }
}

fn render_pixels(window: &MinimalSoftwareWindow, width: u32, height: u32) -> Vec<slint::Rgb8Pixel> {
    window.set_size(slint::PhysicalSize::new(width, height));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
    slint::platform::update_timers_and_animations();
    let mut pixels = vec![slint::Rgb8Pixel::default(); width as usize * height as usize];
    window.window().request_redraw();
    assert!(window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width as usize);
    }));
    pixels
}

#[test]
fn embedded_operation_paints_the_native_icon_and_distinct_fallback_artwork() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let main = MainWindow::new().unwrap();
    main.invoke_apply_fonts("".into(), 11, 15);
    main.invoke_apply_preferences(0, 0);
    main.show().unwrap();
    // The shipped embedded form, driven through its presentation like the adapter.
    let form = main.global::<OperationPresentation>();
    form.set_title_text("Action".into());
    form.set_confirm_text("Proceed".into());
    form.set_body("Selected file 文档.txt\nCaptured source summary".into());
    form.set_token("icon".into());
    form.invoke_begin();
    let child = windows.borrow()[0].clone();
    let marker = slint::Rgb8Pixel {
        r: 241,
        g: 17,
        b: 199,
    };
    let mut icon = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(16, 16);
    icon.make_mut_slice().fill(marker);
    let icon = slint::Image::from_rgb8(icon);
    for (width, height) in [(320, 200), (540, 320)] {
        form.set_kind(0);
        form.set_native_icon(icon.clone());
        let native = render_pixels(&child, width, height);
        let painted: Vec<_> = native
            .iter()
            .enumerate()
            .filter_map(|(index, pixel)| {
                (pixel.r == marker.r && pixel.g == marker.g && pixel.b == marker.b).then_some(index)
            })
            .collect();
        assert!(
            painted.len() >= 64,
            "native action artwork must paint inside the {width}×{height} client; found {} marker pixels",
            painted.len()
        );
        let left = painted
            .iter()
            .map(|index| index % width as usize)
            .min()
            .unwrap();
        let right = painted
            .iter()
            .map(|index| index % width as usize)
            .max()
            .unwrap();
        let top = painted
            .iter()
            .map(|index| index / width as usize)
            .min()
            .unwrap();
        let bottom = painted
            .iter()
            .map(|index| index / width as usize)
            .max()
            .unwrap();
        assert!(right > left + 5 && bottom > top + 5);
        assert!(right < width as usize && bottom < height as usize);

        form.set_native_icon(slint::Image::default());
        let rename = render_pixels(&child, width, height);
        form.set_kind(3);
        let directory = render_pixels(&child, width, height);
        let changed = (top..=bottom)
            .flat_map(|y| (left..=right).map(move |x| y * width as usize + x))
            .filter(|&index| rename[index] != directory[index])
            .count();
        assert!(
            changed >= 8,
            "real Rename and NewFolder fallback artwork must paint in the native icon's client region; only {changed} pixels differ"
        );
        modeless::bitmap::snapshot(
            &native,
            width as usize,
            height as usize,
            &format!("native-operation-icon-{width}x{height}"),
        );
    }
}

#[test]
fn trash_and_permanent_delete_share_modeless_progress_and_close_means_cancel() {
    let h = Harness::new();
    for action in [1, 2] {
        let (id, confirmation) = h.operation(PaneId::Left, action);
        assert_eq!(
            h.main.get_operations().row_count(),
            (action - 1) as usize,
            "an unconfirmed delete must not appear in Jobs"
        );
        key(&confirmation, Key::Return);
        h.assert_closed(&confirmation);
        assert!(matches!(h.commands.borrow().as_slice(),
            [Command::ConfirmOperation { id: actual, new_name: None, replace_existing: false }] if *actual == id));
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
            "each confirmed delete gets its own progress window"
        );
        let progress = &children[0];
        assert!(!h.main.get_embedded_modal_open());
        assert!(!h.main.get_dialog_input_blocked());
        assert!(
            !h.main.get_operations_visible(),
            "the Jobs ledger is independent"
        );
        modeless::draw(progress, 640, 420, "delete-progress");
        assert_visible(progress, "Deletion speed", AccessibleRole::Groupbox);
        assert_visible(
            progress,
            "Overall progress",
            AccessibleRole::ProgressIndicator,
        );
        h.commands.borrow_mut().clear();
        modeless::click(progress, "Pause");
        assert!(matches!(h.commands.borrow().as_slice(),
            [Command::SetOperationPaused { id: actual, paused: true }] if *actual == id));
        h.commands.borrow_mut().clear();
        h.accepting.set(false);
        progress
            .window()
            .dispatch_event(WindowEvent::CloseRequested);
        assert!(
            progress.window().is_visible(),
            "a refused cancel must not hide live work"
        );
        assert!(h.commands.borrow().is_empty());
        modeless::draw(progress, 640, 420, "delete-cancel-question");
        assert_visible(progress, "Continue", AccessibleRole::Button);
        modeless::click(progress, "Cancel operation");
        assert!(
            progress.window().is_visible(),
            "refused confirmation stays retryable"
        );
        assert!(h.commands.borrow().is_empty());
        h.accepting.set(true);
        modeless::click(progress, "Cancel operation");
        // As a Copy's, the window stays to show how the Delete ends.
        assert!(progress.window().is_visible());
        assert!(matches!(h.commands.borrow().as_slice(),
            [Command::CancelOperation(actual)] if *actual == id));
        assert!(
            h.main
                .get_operations()
                .iter()
                .any(|row| row.identity == id.0.to_string() && row.state == "Cancelling")
        );
        key(progress, Key::Escape);
        assert!(
            !progress.window().is_visible(),
            "a cancelling Delete's window closes without another question"
        );
    }
}
