#[path = "support/window_input.rs"]
mod input;
#[path = "support/embedded_file_list.rs"]
mod installed;
#[path = "support/modeless.rs"]
mod modeless;

use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_ui::FileListWindow;
use modeless::{Windows, click, draw};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[test]
fn existing_output_is_a_separate_explicit_decision_at_minimum_size() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = FileListWindow::new().unwrap();
    window.invoke_apply_preferences(1, 0, "".into(), 14, 16);
    let confirmations = Rc::new(Cell::new(0));
    let captured = confirmations.clone();
    window.on_confirm(move || captured.set(captured.get() + 1));
    window.set_destination(2);
    window.set_busy(true);
    window.set_conflict_pending(true);
    window
        .set_conflict_text("文档-list.txt\n\nSize: 1,024 bytes\nModified: 2026-11-09 14:20".into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 320, 200, "file-list-conflict");
    assert_eq!(confirmations.get(), 0);
    click(&native, "Confirm existing file list output");
    assert_eq!(confirmations.get(), 1);
}

#[test]
fn file_list_keeps_output_explicit_and_escape_cancels_while_busy() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = FileListWindow::new().unwrap();
    window.invoke_apply_preferences(1, 0, "".into(), 14, 16);
    let generated = Rc::new(Cell::new(0));
    let captured = generated.clone();
    window.on_generate(move || captured.set(captured.get() + 1));
    let cancelled = Rc::new(Cell::new(0));
    let captured = cancelled.clone();
    window.on_cancel(move || captured.set(captured.get() + 1));
    window.set_preview_text("文档.txt\r\nfolder\r\n".into());
    window.set_summary("2 selected items".into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 720, 520, "file-list-clipboard");
    assert_eq!(window.get_template_text(), "$(FileName)$(CRLF)");
    assert_eq!(generated.get(), 0, "preview must not publish output");
    click(&native, "Create file list");
    assert_eq!(generated.get(), 1);
    window.set_busy(true);
    draw(&native, 720, 520, "file-list-busy");
    let create =
        modeless::accessible_item(&native, AccessibleRole::Button, "Create file list").unwrap();
    assert_eq!(
        create
            .accessible_string_property(AccessibleStringProperty::Enabled)
            .as_deref(),
        Some("false")
    );
    click(&native, "Create file list");
    assert_eq!(generated.get(), 1, "busy output cannot be submitted twice");
    window.invoke_begin();
    native.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    assert_eq!(cancelled.get(), 1);
    window.hide().unwrap();
}

#[test]
fn file_list_browse_is_explicit_and_footer_remains_reachable_at_small_sizes() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = FileListWindow::new().unwrap();
    window.invoke_apply_preferences(1, 0, "".into(), 14, 16);
    let browsed = Rc::new(Cell::new(0));
    let captured = browsed.clone();
    window.on_browse(move || captured.set(captured.get() + 1));
    let cancelled = Rc::new(Cell::new(0));
    let captured = cancelled.clone();
    window.on_cancel(move || captured.set(captured.get() + 1));
    window.set_destination(2);
    window.set_destination_path("一覧.txt".into());
    window.set_append_output(true);
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 720, 520, "file-list-file");
    assert_eq!(browsed.get(), 0);
    click(&native, "Browse file list destination");
    assert_eq!(browsed.get(), 1);
    for (density, size, width, height) in [(0, 11, 320, 200), (1, 18, 390, 300)] {
        window.invoke_apply_preferences(1, density, "".into(), size, size);
        draw(&native, width, height, "file-list-compact");
        let cancel = modeless::accessible_item(&native, AccessibleRole::Button, "Cancel")
            .expect("fixed cancel action");
        let geometry = cancel.geometry();
        let origin = cancel.map_to_native_window(geometry.origin);
        assert!(origin.y >= 0.0);
        assert!(origin.y + geometry.height() <= height as f32);
        assert_eq!(window.get_destination_path(), "一覧.txt");
        assert!(window.get_append_output());
    }
    click(&native, "Cancel");
    assert_eq!(cancelled.get(), 1);
    window.hide().unwrap();
}

#[test]
fn file_list_template_input_reports_edits_and_retains_unicode_selection_offsets() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = FileListWindow::new().unwrap();
    window.invoke_apply_preferences(1, 0, "".into(), 14, 16);
    let edits = Rc::new(RefCell::new(Vec::new()));
    let captured = edits.clone();
    window.on_template_edited(move |text| captured.borrow_mut().push(text.to_string()));
    window.set_template_text("文档$(FileName)".into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 720, 520, "file-list-template");
    window.invoke_focus_template_at(6);
    native
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: "-".into() });
    native
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: "-".into() });
    assert_eq!(window.get_template_text(), "文档-$(FileName)");
    assert_eq!(window.get_template_cursor(), 7);
    assert_eq!(window.get_template_anchor(), 7);
    assert_eq!(&*edits.borrow(), &["文档-$(FileName)"]);
    window.hide().unwrap();
}

#[test]
fn embedded_file_list_cancel_waits_for_exact_request_release_without_another_adapter() {
    use i_slint_core::{accessibility::AccessibilityAction, item_tree::ItemRc};
    use midnight_core::{Command, DirectoryIndex, Event, PaneId};
    use midnight_ui::{MainWindow, Presenter};
    use std::sync::Arc;

    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let main = MainWindow::new().unwrap();
    let root = std::env::temp_dir();
    let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let accepting = Rc::new(Cell::new(true));
    midnight_ui::install(
        &main,
        state.clone(),
        Rc::new({
            let commands = commands.clone();
            let accepting = accepting.clone();
            move |command| {
                if matches!(command, Command::CancelFileList { .. }) && !accepting.get() {
                    return false;
                }
                commands.borrow_mut().push(command);
                true
            }
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
    state.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        entries: Arc::from([]),
        index: Arc::new(DirectoryIndex::from_entries(&[])),
    });
    main.show().unwrap();
    midnight_ui::refresh(&main, &state.borrow());
    let mut retired_controls: Option<(ItemRc, ItemRc)> = None;
    for cycle in 0..3 {
        main.invoke_make_file_list(0);
        slint::platform::update_timers_and_animations();
        assert_eq!(
            windows.borrow().len(),
            1,
            "cycle {cycle} created a child window"
        );
        let native = windows.borrow()[0].clone();
        let before = commands.borrow().len();
        if let Some((generate, cancel)) = retired_controls.take() {
            generate.accessible_action(&AccessibilityAction::Default);
            cancel.accessible_action(&AccessibilityAction::Default);
            assert_eq!(
                commands.borrow().len(),
                before,
                "retired controls submitted a new draft"
            );
        }
        retired_controls = Some((
            modeless::accessible_popup_item(&native, AccessibleRole::Button, "Create file list")
                .unwrap(),
            modeless::accessible_popup_item(&native, AccessibleRole::Button, "Cancel").unwrap(),
        ));
        assert!(native.window().is_visible());
        assert!(
            main.get_dialog_input_blocked(),
            "Make File List is modal to its owner"
        );
        let (id, generation) = commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::RunFileList(request) => Some((request.id, request.generation)),
                _ => None,
            })
            .unwrap();

        accepting.set(false);
        let form = main.global::<midnight_ui::FileListPresentation>();
        form.invoke_cancel(form.get_token());
        assert!(
            native.window().is_visible(),
            "a full queue keeps the owner alive"
        );
        assert!(main.window().is_visible());
        assert!(
            main.get_dialog_input_blocked(),
            "rejected cancellation retains the modal lease"
        );
        accepting.set(true);
        form.invoke_cancel(form.get_token());
        assert!(native.window().is_visible());
        assert!(
            !main.get_dialog_input_blocked(),
            "accepted close releases the owner"
        );
        assert!(commands.borrow().iter().any(|command| {
            matches!(command, Command::CancelFileList { id: closed, generation: closed_generation }
                if *closed == id && *closed_generation == generation)
        }));
        midnight_ui::refresh(&main, &state.borrow());
        assert_eq!(windows.borrow().len(), 1);
        assert_eq!(
            windows
                .borrow()
                .iter()
                .filter(|window| window.window().is_visible())
                .count(),
            1,
            "only the main window remains visible after each close"
        );
    }
    main.invoke_close_file_list_window();
    main.hide().unwrap();
    state.borrow_mut().retire_all();
}

#[test]
fn file_list_preview_grows_with_the_window_and_resize_preserves_the_draft_caret() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = FileListWindow::new().unwrap();
    window.invoke_apply_preferences(3, 0, "".into(), 13, 18);
    window.set_template_text("文档$(FileName)$(CRLF)".into());
    window.set_preview_text("文档.txt\nnotes.txt\n".into());
    window.set_summary("2 selected items".into());
    let edits = Rc::new(Cell::new(0));
    let observed = edits.clone();
    window.on_template_edited(move |_| observed.set(observed.get() + 1));
    let weak = window.as_weak();
    window.on_cancel(move || weak.upgrade().unwrap().hide().unwrap());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 720, 520, "file-list-preview-normal");
    window.invoke_focus_template_at(6);
    let preview =
        modeless::accessible_item(&native, AccessibleRole::TextInput, "File list preview").unwrap();
    let old_height = preview.geometry().height();
    draw(&native, 1000, 820, "file-list-preview-expanded");
    let new_height = preview.geometry().height();
    assert!(
        new_height >= old_height + 260.0,
        "extra height belongs to preview: {old_height} -> {new_height}"
    );
    assert_eq!(window.get_template_cursor(), 6);
    assert_eq!(window.get_template_anchor(), 6);
    assert_eq!(window.get_template_text(), "文档$(FileName)$(CRLF)");
    assert_eq!(edits.get(), 0, "resizing does not edit or submit a draft");
    let bounds = preview.geometry();
    let origin = preview.map_to_native_window(bounds.origin);
    let position = slint::LogicalPosition::new(origin.x + 20.0, origin.y + 20.0);
    native.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: slint::platform::PointerEventButton::Left,
    });
    native
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: slint::platform::PointerEventButton::Left,
        });
    native
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: "x".into() });
    native
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: "x".into() });
    assert_eq!(
        preview
            .accessible_string_property(AccessibleStringProperty::Value)
            .unwrap(),
        "文档.txt\nnotes.txt\n",
        "preview remains read-only"
    );
    for (density, font) in [(0, 11), (1, 32)] {
        window.invoke_apply_preferences(3, density, "".into(), font, font);
        draw(&native, 320, 200, "file-list-preview-compact");
        let cancel = modeless::accessible_item(&native, AccessibleRole::Button, "Cancel").unwrap();
        let rect = cancel.geometry();
        let at = cancel.map_to_native_window(rect.origin);
        assert!(
            at.x >= 0.0
                && at.y >= 0.0
                && at.x + rect.width() <= 320.5
                && at.y + rect.height() <= 200.5
        );
    }
    click(&native, "Cancel");
    assert!(!window.window().is_visible());
}

#[test]
fn embedded_file_list_preserves_template_caret_and_capture_through_refresh_and_export() {
    use midnight_core::{
        Command, Event, FileListDestination, FileListEvent, FileListSource, FileListUpdate,
    };
    use midnight_ui::FileListPresentation;
    use std::sync::Arc;

    let h = installed::Harness::new();
    let first = h.request();
    let form = h.main.global::<FileListPresentation>();
    let token = form.get_token();
    assert_eq!(form.get_template_text(), "文档$(FileName)$(CRLF)");
    form.invoke_focus_template_at(6);
    h.tick(0);
    input::key(&h.native, "-");
    h.tick(0);
    assert_eq!(form.get_template_text(), "文档-$(FileName)$(CRLF)");
    assert_eq!(form.get_template_cursor(), 7);
    draw(&h.native, 390, 300, "embedded-file-list-resize");
    midnight_ui::refresh(&h.main, &h.state.borrow());
    h.tick(0);
    assert_eq!(form.get_template_cursor(), 7);
    form.invoke_insert_variable(token.clone(), 0);
    h.tick(0);
    assert_eq!(
        form.get_template_text(),
        "文档-$(FileName)$(FileName)$(CRLF)"
    );
    assert_eq!(form.get_template_cursor(), 18);
    h.apply(Event::FileList(Box::new(FileListEvent {
        id: first.id,
        generation: first.generation,
        result: Ok(FileListUpdate::Preview {
            text: "obsolete preview".into(),
            items: 1,
            truncated: false,
        }),
    })));
    assert!(
        form.get_preview_text().is_empty(),
        "stale preview replaced the edited draft"
    );
    h.tick(200);
    let preview = h.request();
    assert!(Arc::ptr_eq(&first.capture, &preview.capture));
    assert_eq!(preview.template, form.get_template_text().as_str());
    assert!(h.commands.borrow().iter().all(|command| {
        !matches!(command, Command::RunFileList(request) if request.destination != FileListDestination::Preview)
    }));

    form.invoke_select_preset(token.clone(), 1);
    h.tick(200);
    assert_eq!(
        form.get_template_text(),
        "$(Drive)$(Path)$(FileName)$(CRLF)"
    );
    form.invoke_select_history(token.clone(), 1);
    h.tick(200);
    assert_eq!(form.get_template_text(), "$(FileSize)$(LF)");
    h.main.invoke_activate(1);
    let destination = h.root.join("output 文档.txt");
    form.set_destination(2);
    form.set_destination_path(destination.to_str().unwrap().into());
    form.set_append_output(true);
    form.invoke_generate(token);
    let output = h.request();
    assert!(Arc::ptr_eq(&first.capture, &output.capture));
    assert_eq!(output.template, "$(FileSize)$(LF)");
    assert_eq!(
        output.destination,
        FileListDestination::File {
            path: destination,
            append: true
        }
    );
    let FileListSource::Native {
        entries, fallback, ..
    } = &output.capture.source
    else {
        panic!("native capture")
    };
    assert_eq!(entries.len(), 2);
    assert_eq!(*fallback, Some(midnight_core::EntryId(1)));
    assert!(form.get_busy());
    assert!(form.get_active());
    h.tick(200);
}

#[test]
fn embedded_file_list_browse_and_existing_output_confirmation_keep_exact_request() {
    use midnight_core::*;
    use midnight_ui::FileListPresentation;

    let h = installed::Harness::new();
    let form = h.main.global::<FileListPresentation>();
    let token = form.get_token();
    let preview = h.request();
    form.set_destination(2);
    form.invoke_browse(token.clone());
    let picker = h
        .commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::PickPath { request, input } => {
                assert_eq!(input.kind, PathPickerKind::TextSave);
                Some(*request)
            }
            _ => None,
        })
        .unwrap();
    assert!(form.get_busy());
    assert_eq!(
        h.request().generation,
        preview.generation,
        "Browse must not generate output"
    );
    let destination = h.root.join("picked 文档.txt");
    h.apply(Event::PathPicked {
        request: picker,
        result: Ok(Some(destination.clone())),
    });
    assert!(!form.get_busy());
    assert_eq!(form.get_destination_path(), destination.to_str().unwrap());
    form.invoke_generate(token.clone());
    let output = h.request();
    h.apply(Event::FileList(Box::new(FileListEvent {
        id: output.id,
        generation: output.generation,
        result: Ok(FileListUpdate::Conflict {
            entry: TransferEntry {
                item: MutationItem {
                    path: destination,
                    kind: EntryKind::File,
                    metadata: EntryMetadata::default(),
                },
                identity: TransferIdentity {
                    device: 1,
                    object: [7; 16],
                },
            },
            append: false,
        }),
    })));
    assert!(form.get_conflict_pending());
    assert!(
        h.commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::ConfirmFileList { .. }))
    );
    h.accepting.set(false);
    form.invoke_confirm(token.clone());
    assert!(form.get_conflict_pending());
    assert!(!form.get_error_text().is_empty());
    h.accepting.set(true);
    form.invoke_confirm(token);
    assert!(
        matches!(h.commands.borrow().last(), Some(Command::ConfirmFileList { id, generation })
        if *id == output.id && *generation == output.generation)
    );
    assert!(!form.get_conflict_pending());
    assert!(form.get_active());
    h.tick(0);
}
