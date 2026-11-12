//! Real keyboard/pointer routing; mock commands never touch a filesystem.
#[allow(dead_code)] // This fixture uses the platform/render helpers, not its pointer helpers.
#[path = "support/modeless.rs"]
mod modeless;
#[path = "support/ui.rs"]
mod ui;
#[path = "support/window_input.rs"]
mod window_input;
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};
use ui::{TestUi, chord, key};

#[test]
fn rename_delete_create_dialogs_preserve_confirmation_and_editor_focus() {
    let (mut ui, windows) = main_with_children();
    let path = PathBuf::from(std::path::MAIN_SEPARATOR_STR).join("fixture");
    let state = Rc::new(RefCell::new(Presenter::new(path.clone(), path.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let reject_confirmation = Rc::new(Cell::new(false));
    let reject = reject_confirmation.clone();
    let sink = commands.clone();
    install(
        &ui.window,
        state.clone(),
        Rc::new(move |command| {
            if reject.get()
                && matches!(
                    command,
                    Command::ConfirmOperation { .. } | Command::CancelOperation(_)
                )
            {
                return false;
            }
            sink.borrow_mut().push(command);
            true
        }),
    );
    let navigation = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::Navigate {
                pane: PaneId::Left,
                navigation,
                ..
            } => Some(*navigation),
            _ => None,
        })
        .unwrap();
    let entry = Entry::new(EntryId(1), path.join("文档.txt"), EntryKind::File);
    state.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        index: Arc::new(DirectoryIndex::from_entries(std::slice::from_ref(&entry))),
        entries: vec![entry.clone()].into(),
    });
    refresh(&ui.window, &state.borrow());
    ui.window.invoke_row_click(0, 1, false, false);
    ui.window.invoke_focus_files();
    commands.borrow_mut().clear();
    key_current(&ui, &windows, Key::F2);
    let request = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::PrepareOperation(request) => Some(request.clone()),
            _ => None,
        })
        .expect("F2 prepares focused entry");
    assert_eq!(request.kind, MutationKind::Rename);
    state
        .borrow_mut()
        .apply(Event::Operation(OperationEvent::Preview(Arc::new(
            OperationPreview {
                id: request.id,
                kind: request.kind,
                files: 1,
                folders: 0,
                file_bytes: None,
                sample: vec![MutationItem {
                    path: entry.path,
                    kind: entry.kind,
                    metadata: entry.metadata,
                }],
            },
        ))));
    refresh(&ui.window, &state.borrow());
    ui.draw();
    key_current(&ui, &windows, "new"); // Only the basename is selected; .txt survives.
    key_current(&ui, &windows, Key::Return);
    assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::ConfirmOperation { id, new_name: Some(name) } if *id == request.id && name == "new.txt")));
    assert!(ui.window.get_operation_token().is_empty());

    for (delete_key, permanent) in [
        (Key::Delete, true),
        (Key::F8, true),
        (Key::Delete, false),
        (Key::F8, false),
    ] {
        commands.borrow_mut().clear();
        ui.window.invoke_focus_files();
        chord(
            &ui.window,
            if permanent { &[Key::Shift] } else { &[] },
            delete_key,
        );
        let expected_mode = if permanent {
            DeleteMode::Permanent
        } else {
            DeleteMode::Trash
        };
        assert!(
            commands.borrow().iter().any(|command| matches!(command,
        Command::PrepareOperation(request) if request.kind == MutationKind::Delete(expected_mode)))
        );
        let deletion = commands
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::PrepareOperation(request) => Some(request.clone()),
                _ => None,
            })
            .unwrap();
        state
            .borrow_mut()
            .apply(Event::Operation(OperationEvent::Preview(Arc::new(
                OperationPreview {
                    id: deletion.id,
                    kind: deletion.kind,
                    files: 1,
                    folders: 0,
                    file_bytes: None,
                    sample: vec![],
                },
            ))));
        refresh(&ui.window, &state.borrow());
        ui.draw();
        assert_eq!(ui.window.get_operation_dangerous(), permanent);
        // Merely displaying the confirmation must never perform the operation.
        assert!(!commands.borrow().iter().any(|command| matches!(
            command,
            Command::ConfirmOperation { .. } | Command::CancelOperation(_)
        )));
        key_current(&ui, &windows, Key::Return);
        assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::ConfirmOperation { id, new_name: None } if *id == deletion.id)));
        assert!(!ui.window.get_embedded_modal_open(), "progress is modeless");
        assert!(!ui.window.get_dialog_input_blocked());
        assert!(
            windows
                .borrow()
                .iter()
                .skip(1)
                .any(|child| child.window().is_visible()),
            "confirmed Delete has visible progress while panel actions remain enabled"
        );
        assert!(
            !commands
                .borrow()
                .iter()
                .any(|command| matches!(command, Command::CancelOperation(_)))
        );
    }
    // Finish deferred popup retirement and its invoking-focus restoration before
    // the next explicit editor-focus action, as the native frame loop does.
    ui.draw();
    assert!(
        i_slint_core::window::WindowInner::from_pub(ui.window.window())
            .active_popups()
            .is_empty()
    );
    ui.window.invoke_focus_command();
    assert!(ui.window.get_command_focused());
    commands.borrow_mut().clear();
    key_current(&ui, &windows, Key::Delete);
    chord_current(&ui, &windows, &[Key::Shift], Key::F4);
    assert!(
        ui.window.get_command_focused(),
        "editor shortcuts must retain actual input focus"
    );
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::PrepareOperation(_)))
    );
    for (kind, expected_name) in [
        (MutationKind::CreateDirectory, "nested/文档"),
        (MutationKind::CreateFile, "文档 🦀.txt"),
    ] {
        ui.window.invoke_focus_files();
        commands.borrow_mut().clear();
        if kind == MutationKind::CreateFile {
            chord_current(&ui, &windows, &[Key::Shift], Key::F4);
        } else {
            key_current(&ui, &windows, Key::F7);
        }
        assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::PrepareOperation(request) if request.kind == kind)));
        let creation = commands
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::PrepareOperation(request) => Some(request.clone()),
                _ => None,
            })
            .unwrap();
        state
            .borrow_mut()
            .apply(Event::Operation(OperationEvent::Preview(Arc::new(
                OperationPreview {
                    id: creation.id,
                    kind: creation.kind,
                    files: 0,
                    folders: 0,
                    file_bytes: Some(0),
                    sample: vec![],
                },
            ))));
        refresh(&ui.window, &state.borrow());
        ui.draw();
        key_current(&ui, &windows, expected_name);
        let expected_name = if kind == MutationKind::CreateFile {
            reject_confirmation.set(true);
            key_current(&ui, &windows, Key::Return);
            assert_eq!(ui.window.get_operation_token(), creation.id.0.to_string());
            assert!(
                ui.window.get_embedded_modal_open(),
                "failed submission retains the embedded modal input gate"
            );
            assert!(!ui.window.get_dialog_input_blocked());
            key_current(&ui, &windows, "1");
            reject_confirmation.set(false);
            format!("{expected_name}1")
        } else {
            expected_name.to_owned()
        };
        key_current(&ui, &windows, Key::Return);
        assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::ConfirmOperation { id, new_name: Some(name) }
            if *id == creation.id && *name == expected_name)));
    }
}

#[test]
fn empty_file_toolbar_uses_its_panel() {
    let mut ui = TestUi::new();
    let commands = Rc::new(RefCell::new(Vec::new()));
    let sink = commands.clone();
    ui.window
        .on_mutation(move |pane, action| sink.borrow_mut().push((pane, action)));
    ui.draw();
    ui::click_root(
        &ui.window,
        "New empty file (Shift+F4)",
        i_slint_core::items::AccessibleRole::Button,
    );
    assert_eq!(commands.borrow().as_slice(), &[(0, 4)]);
}

#[test]
fn clicking_after_breadcrumb_edits_path_and_enter_submits_unicode() {
    let mut ui = TestUi::new();
    ui.window.set_left_path("C:\\folder".into());
    ui.window.invoke_focus_files();
    let paths = Rc::new(RefCell::new(Vec::new()));
    let sink = paths.clone();
    ui.window
        .on_navigate(move |pane, path| sink.borrow_mut().push((pane, path)));
    ui.draw();
    ui::click_root(
        &ui.window,
        "Edit folder path",
        i_slint_core::items::AccessibleRole::Button,
    );
    chord(&ui.window, &[Key::Control], "a");
    key(&ui.window, "C:\\custom 文档");
    key(&ui.window, Key::Return);
    assert_eq!(paths.borrow().as_slice(), &[(0, "C:\\custom 文档".into())]);
    ui.draw();
    ui::click_root(
        &ui.window,
        "Edit folder path",
        i_slint_core::items::AccessibleRole::Button,
    );
    key(&ui.window, "discard");
    key(&ui.window, Key::Escape);
    assert_eq!(paths.borrow().len(), 1);
    assert_eq!(ui.window.get_left_path_draft(), "C:\\folder");
}

type ChildWindows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;
fn main_with_children() -> (TestUi, ChildWindows) {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = midnight_ui::MainWindow::new().unwrap();
    midnight_ui::install_file_keys(&window);
    window.set_preferences_ready(true);
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    let mut test = TestUi {
        window,
        native,
        pixels: Vec::new(),
        width: 0,
        height: 0,
    };
    test.resize(1340, 820);
    (test, windows)
}
fn current(test: &TestUi, _windows: &ChildWindows) -> Rc<MinimalSoftwareWindow> {
    slint::platform::update_timers_and_animations();
    // Confirmed Delete keeps a modeless progress window while new panel actions
    // remain available. Only an active confirmation owns keyboard input here.
    if test.window.get_embedded_modal_open() {
        assert!(
            !i_slint_core::window::WindowInner::from_pub(test.window.window())
                .active_popups()
                .is_empty()
        );
        modeless::draw(
            &test.native,
            test.width as u32,
            test.height as u32,
            "mutation-embedded",
        );
    }
    test.native.clone()
}
fn key_current(test: &TestUi, windows: &ChildWindows, key: impl Into<slint::SharedString>) {
    window_input::key(&current(test, windows), key);
}
fn chord_current(
    test: &TestUi,
    windows: &ChildWindows,
    modifiers: &[Key],
    key: impl Into<slint::SharedString>,
) {
    window_input::chord(&current(test, windows), modifiers, key);
}
