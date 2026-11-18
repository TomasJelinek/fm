#[path = "support/embedded_form.rs"]
mod form;
#[path = "support/window_input.rs"]
mod input;
#[path = "support/embedded_file_list.rs"]
mod installed;
#[path = "support/modeless.rs"]
mod modeless;

use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_ui::FileListPresentation;
use modeless::{Windows, draw};
use slint::{
    ComponentHandle, LogicalPosition,
    platform::{Key, PointerEventButton, WindowAdapter, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// The shipped embedded Make File List form; Cancel closes it like the adapter does.
fn file_list_host() -> form::Host {
    let host = form::Host::new();
    host.apply_preferences(1, 0, "", 14, 16);
    let weak = host.main.as_weak();
    host.main
        .global::<FileListPresentation>()
        .on_cancel(move |_| {
            if let Some(main) = weak.upgrade() {
                main.global::<FileListPresentation>().set_active(false);
            }
        });
    host
}

fn begin(host: &form::Host) {
    let form = host.main.global::<FileListPresentation>();
    form.set_token("file-list".into());
    form.invoke_begin();
    host.settle();
}

#[test]
fn existing_output_is_a_separate_explicit_decision_at_minimum_size() {
    let host = file_list_host();
    let form = host.main.global::<FileListPresentation>();
    let confirmations = Rc::new(Cell::new(0));
    let captured = confirmations.clone();
    form.on_confirm(move |_| captured.set(captured.get() + 1));
    form.set_destination(2);
    form.set_busy(true);
    form.set_conflict_pending(true);
    form.set_conflict_text("文档-list.txt\n\nSize: 1,024 bytes\nModified: 2026-11-09 14:20".into());
    begin(&host);
    host.draw(328, 208, "file-list-conflict");
    assert_eq!(confirmations.get(), 0);
    host.click("Confirm existing file list output");
    assert_eq!(confirmations.get(), 1);
}

#[test]
fn file_list_keeps_output_explicit_and_escape_cancels_while_busy() {
    let host = file_list_host();
    let form = host.main.global::<FileListPresentation>();
    let generated = Rc::new(Cell::new(0));
    let captured = generated.clone();
    form.on_generate(move |_| captured.set(captured.get() + 1));
    form.set_preview_text("文档.txt\r\nfolder\r\n".into());
    form.set_summary("2 selected items".into());
    begin(&host);
    host.draw(744, 544, "file-list-clipboard");
    assert_eq!(form.get_template_text(), "$(FileName)$(CRLF)");
    assert_eq!(generated.get(), 0, "preview must not publish output");
    host.click("Create file list");
    assert_eq!(generated.get(), 1);
    form.set_busy(true);
    host.draw(744, 544, "file-list-busy");
    let create = host.item(AccessibleRole::Button, "Create file list");
    assert_eq!(
        create
            .accessible_string_property(AccessibleStringProperty::Enabled)
            .as_deref(),
        Some("false")
    );
    host.click("Create file list");
    assert_eq!(generated.get(), 1, "busy output cannot be submitted twice");
    form.invoke_begin();
    host.key(Key::Escape);
    assert!(!form.get_active(), "Escape cancels the busy form");
    assert!(!host.is_open());
}

#[test]
fn file_list_browse_is_explicit_and_footer_remains_reachable_at_small_sizes() {
    let host = file_list_host();
    let form = host.main.global::<FileListPresentation>();
    let browsed = Rc::new(Cell::new(0));
    let captured = browsed.clone();
    form.on_browse(move |_| captured.set(captured.get() + 1));
    form.set_destination(2);
    form.set_destination_path("一覧.txt".into());
    form.set_append_output(true);
    begin(&host);
    host.draw(744, 544, "file-list-file");
    assert_eq!(browsed.get(), 0);
    host.click("Browse file list destination");
    assert_eq!(browsed.get(), 1);
    for (density, size, width, height) in [(0, 11, 328, 208), (1, 18, 398, 308)] {
        host.apply_preferences(1, density, "", size, size);
        host.draw(width, height, "file-list-compact");
        form::assert_inside(&host.item(AccessibleRole::Button, "Cancel"), "Cancel");
        assert_eq!(form.get_destination_path(), "一覧.txt");
        assert!(form.get_append_output());
    }
    host.click("Cancel");
    assert!(!form.get_active());
    assert!(!host.is_open());
}

#[test]
fn file_list_template_input_reports_edits_and_retains_unicode_selection_offsets() {
    let host = file_list_host();
    let form = host.main.global::<FileListPresentation>();
    let edits = Rc::new(RefCell::new(Vec::new()));
    let captured = edits.clone();
    form.on_template_edited(move |_, text| captured.borrow_mut().push(text.to_string()));
    form.set_template_text("文档$(FileName)".into());
    begin(&host);
    host.draw(744, 544, "file-list-template");
    form.invoke_focus_template_at(6);
    host.key("-");
    assert_eq!(form.get_template_text(), "文档-$(FileName)");
    assert_eq!(form.get_template_cursor(), 7);
    assert_eq!(form.get_template_anchor(), 7);
    assert_eq!(&*edits.borrow(), &["文档-$(FileName)"]);
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
            main.get_embedded_modal_open(),
            "Make File List must gate background application input"
        );
        assert!(
            !main.get_dialog_input_blocked(),
            "Make File List keeps main input enabled"
        );
        assert_eq!(
            i_slint_core::window::WindowInner::from_pub(main.window())
                .active_popups()
                .len(),
            1
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
            main.get_embedded_modal_open(),
            "rejected cancellation retains the embedded modal input gate"
        );
        assert_eq!(
            i_slint_core::window::WindowInner::from_pub(main.window())
                .active_popups()
                .len(),
            1
        );
        accepting.set(true);
        form.invoke_cancel(form.get_token());
        slint::platform::update_timers_and_animations();
        assert!(native.window().is_visible());
        assert!(
            !main.get_embedded_modal_open(),
            "accepted close releases the embedded modal input gate"
        );
        assert!(!main.get_dialog_input_blocked());
        assert!(
            i_slint_core::window::WindowInner::from_pub(main.window())
                .active_popups()
                .is_empty()
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
fn file_list_preview_grows_with_the_dialog_and_resize_preserves_the_draft_caret() {
    let host = file_list_host();
    host.apply_preferences(3, 0, "", 13, 18);
    let form = host.main.global::<FileListPresentation>();
    form.set_template_text("文档$(FileName)$(CRLF)".into());
    form.set_preview_text("文档.txt\nnotes.txt\n".into());
    form.set_summary("2 selected items".into());
    let edits = Rc::new(Cell::new(0));
    let observed = edits.clone();
    form.on_template_edited(move |_, _| observed.set(observed.get() + 1));
    begin(&host);
    // The dialog opens at its own 720×520 size inside the larger main window.
    host.draw(1000, 820, "file-list-preview-normal");
    form.invoke_focus_template_at(6);
    host.settle();
    let preview = host.item(AccessibleRole::TextInput, "File list preview");
    let old_height = preview.geometry().height();
    let (_, before) = form::body(&preview);
    host.stretch_vertically(&preview);
    host.draw(1000, 820, "file-list-preview-expanded");
    let extra = form::body(&preview).1.height - before.height;
    assert!(extra >= 250.0, "the dialog grew by only {extra}");
    let new_height = preview.geometry().height();
    assert!(
        new_height >= old_height + extra * 0.85,
        "extra height belongs to preview: {old_height} -> {new_height} for {extra}"
    );
    assert_eq!(form.get_template_cursor(), 6);
    assert_eq!(form.get_template_anchor(), 6);
    assert_eq!(form.get_template_text(), "文档$(FileName)$(CRLF)");
    assert_eq!(edits.get(), 0, "resizing does not edit or submit a draft");
    let bounds = preview.geometry();
    let origin = preview.map_to_native_window(bounds.origin);
    let position = LogicalPosition::new(origin.x + 20.0, origin.y + 20.0);
    for event in [
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        },
    ] {
        host.native.window().dispatch_event(event);
    }
    host.key("x");
    assert_eq!(
        preview
            .accessible_string_property(AccessibleStringProperty::Value)
            .unwrap(),
        "文档.txt\nnotes.txt\n",
        "preview remains read-only"
    );
    for (density, font) in [(0, 11), (1, 32)] {
        host.apply_preferences(3, density, "", font, font);
        host.draw(328, 208, "file-list-preview-compact");
        form::assert_inside(&host.item(AccessibleRole::Button, "Cancel"), "Cancel");
    }
    host.click("Cancel");
    assert!(!host.is_open());
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

    // Preset and history choices are edits: each previews its own template once the
    // 180 ms debounce elapsed.
    let previewed = |expected: &str| {
        assert_eq!(form.get_template_text(), expected);
        h.tick(200);
        let preview = h.request();
        assert_eq!(preview.destination, FileListDestination::Preview);
        assert_eq!(preview.template, expected);
        assert!(Arc::ptr_eq(&first.capture, &preview.capture));
    };
    form.invoke_select_preset(token.clone(), 1);
    previewed("$(Drive)$(Path)$(FileName)$(CRLF)");
    form.invoke_select_history(token.clone(), 1);
    previewed("$(FileSize)$(LF)");
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
