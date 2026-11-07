//! Real keyboard/pointer routing; mock commands never touch a filesystem.
#[path = "support/ui.rs"]
mod ui;
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::platform::Key;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};
use ui::{TestUi, chord, key};

#[test]
fn rename_delete_create_dialogs_preserve_confirmation_and_editor_focus() {
    let mut ui = TestUi::new();
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
    key(&ui.window, Key::F2);
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
    key(&ui.window, "new"); // Only the basename is selected; .txt survives.
    key(&ui.window, Key::Return);
    assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::ConfirmOperation { id, new_name: Some(name) } if *id == request.id && name == "new.txt")));
    assert!(ui.window.get_operation_token().is_empty());

    commands.borrow_mut().clear();
    chord(&ui.window, &[Key::Shift], Key::Delete);
    assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::PrepareOperation(request) if request.kind == MutationKind::Delete(DeleteMode::Permanent))));
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
    assert!(ui.window.get_operation_dangerous());
    key(&ui.window, Key::Return); // Destructive confirmation defaults to Cancel.
    assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::CancelOperation(id) if *id == deletion.id)));
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::ConfirmOperation { .. }))
    );
    ui.window.invoke_focus_command();
    commands.borrow_mut().clear();
    key(&ui.window, Key::Delete);
    chord(&ui.window, &[Key::Shift], Key::F4);
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
            chord(&ui.window, &[Key::Shift], Key::F4);
        } else {
            key(&ui.window, Key::F7);
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
        key(&ui.window, expected_name);
        let expected_name = if kind == MutationKind::CreateFile {
            reject_confirmation.set(true);
            key(&ui.window, Key::Return);
            assert_eq!(ui.window.get_operation_token(), creation.id.0.to_string());
            assert_eq!(
                ui.window.get_focused_pane(),
                -1,
                "failed submission must retain dialog focus"
            );
            key(&ui.window, "1");
            key(&ui.window, Key::Escape);
            assert_eq!(ui.window.get_operation_token(), creation.id.0.to_string());
            assert_eq!(
                ui.window.get_focused_pane(),
                -1,
                "failed cancellation must retain dialog focus"
            );
            key(&ui.window, "2");
            reject_confirmation.set(false);
            format!("{expected_name}12")
        } else {
            expected_name.to_owned()
        };
        key(&ui.window, Key::Return);
        assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::ConfirmOperation { id, new_name: Some(name) }
            if *id == creation.id && *name == expected_name)));
    }
}

#[test]
fn empty_file_toolbar_uses_its_panel_and_escape_does_not_confirm() {
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
    // The shared name dialog consumes Escape without submitting the draft.
    let cancelled = Rc::new(RefCell::new(Vec::new()));
    let sink = cancelled.clone();
    ui.window
        .on_cancel_operation(move |token| sink.borrow_mut().push(token));
    ui.window
        .on_confirm_operation(|_, _| panic!("Escape must not create a file"));
    ui.window.set_operation_token("41".into());
    ui.window.set_operation_title("New empty file".into());
    ui.window.set_operation_has_name(true);
    ui.window.invoke_begin_operation();
    ui.draw();
    key(&ui.window, "draft.txt");
    key(&ui.window, Key::Escape);
    assert_eq!(
        cancelled.borrow().as_slice(),
        &[slint::SharedString::from("41")]
    );
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
