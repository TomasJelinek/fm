//! Native operation forms retain the backend target and the exact UI occurrence.
#[path = "support/modeless.rs"]
mod modeless;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, install, refresh};
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
}

impl Harness {
    fn new() -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
        let main = MainWindow::new().unwrap();
        midnight_ui::install_file_keys(&main);
        let root = std::env::temp_dir().join("native-operation-targets 文档");
        let paths = [root.join("left"), root.join("right")];
        let state = Rc::new(RefCell::new(Presenter::new(
            paths[0].clone(),
            paths[1].clone(),
        )));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let captured = commands.clone();
        let accepting = Rc::new(Cell::new(true));
        let gate = accepting.clone();
        install(
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
            1,
            "operation form must own a separate visible native window adapter"
        );
        assert!(
            self.main.get_dialog_input_blocked(),
            "the owner must be modal while the form is open"
        );
        assert!(
            WindowInner::from_pub(self.main.window())
                .active_popups()
                .is_empty()
        );
        let child = children[0].clone();
        modeless::draw(&child, 640, 420, "operation-child");
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
        self.state
            .borrow_mut()
            .apply(Event::Operation(OperationEvent::Preview(Arc::new(preview))));
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
        assert!(
            self.commands.borrow().is_empty(),
            "opening a draft must not enqueue a transfer"
        );
        self.child()
    }

    fn assert_closed(&self, child: &MinimalSoftwareWindow) {
        slint::platform::update_timers_and_animations();
        assert!(!child.window().is_visible());
        assert!(self.main.window().is_visible());
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
    modeless::accessible_item(window, role, label).unwrap_or_else(|| panic!("missing {label}"))
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
            [Command::ConfirmOperation {id: actual, new_name: Some(name)}]
            if *actual == id && name == "renamed 文档.txXt"));
        h.assert_closed(&child);
        key(&child, Key::Return);
        assert_eq!(
            h.commands.borrow().len(),
            1,
            "a closed child cannot confirm twice"
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
            [Command::ConfirmOperation {id: actual, new_name: Some(actual_name)}]
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
        [Command::ConfirmOperation {id: actual, new_name: None}] if *actual == id));
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
            let commands = h.commands.borrow();
            let request = match commands.as_slice() {
                [Command::EnqueueSelectedCopy(request)] if !moving => request,
                [Command::EnqueueSelectedMove(request)] if moving => request,
                _ => panic!("the frozen intent must enqueue exactly once"),
            };
            assert_eq!(request.destination, destination);
            assert_eq!(
                request.files.fallback,
                Some(h.paths[pane.index()].join("source 文档.txt"))
            );
            drop(commands);
            h.assert_closed(&child);
            // This synthetic sink has no terminal progress reply. Close the
            // distinct modeless progress child through its real Escape route.
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
    assert_eq!(h.windows.borrow().len(), 2);
    refresh(&h.main, &h.state.borrow());
    assert_eq!(value(&child, "New name"), "retry 文档.txt");
    h.accepting.set(true);
    key(&child, Key::Return);
    assert!(matches!(h.commands.borrow().as_slice(),
        [Command::ConfirmOperation {id: actual, new_name: Some(name)}]
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
                    .apply(Event::PreferencesLoaded(Ok(Box::new(preferences))));
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

#[test]
fn native_operation_rejected_close_retains_modal_child_and_never_turns_into_confirmation() {
    let h = Harness::new();
    let (id, child) = h.operation(PaneId::Left, 2);
    h.accepting.set(false);
    child.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(child.window().is_visible());
    assert!(h.main.get_dialog_input_blocked());
    assert_eq!(h.windows.borrow().len(), 2);
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
        h.commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::ConfirmOperation { .. }))
    );
    if child.window().is_visible() {
        // A second explicit native close retries the same cancellation identity.
        child.window().dispatch_event(WindowEvent::CloseRequested);
    }
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
        assert!(
            h.commands.borrow().is_empty(),
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
fn native_operation_client_paints_the_native_icon_and_distinct_fallback_artwork() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let form = midnight_ui::OperationWindow::new().unwrap();
    form.set_title_text("Action".into());
    form.set_confirm_text("Proceed".into());
    form.set_body("Selected file 文档.txt\nCaptured source summary".into());
    form.invoke_apply_preferences(0, 0, "".into(), 11, 15);
    form.show().unwrap();
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
            [Command::ConfirmOperation { id: actual, new_name: None }] if *actual == id));
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
        h.accepting.set(true);
        key(progress, Key::Escape);
        assert!(!progress.window().is_visible());
        assert!(matches!(h.commands.borrow().as_slice(),
            [Command::CancelOperation(actual)] if *actual == id));
        assert!(
            h.main
                .get_operations()
                .iter()
                .any(|row| row.identity == id.0.to_string() && row.state == "Cancelling")
        );
    }
}
