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
fn file_list_native_close_waits_for_exact_request_release_and_does_not_reopen() {
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
    for cycle in 0..3 {
        main.invoke_make_file_list(0);
        assert_eq!(windows.borrow().len(), cycle + 2);
        let native = windows.borrow().last().unwrap().clone();
        assert!(native.window().is_visible());
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
        native.window().dispatch_event(WindowEvent::CloseRequested);
        assert!(
            native.window().is_visible(),
            "a full queue keeps the owner alive"
        );
        assert!(main.window().is_visible());
        accepting.set(true);
        native.window().dispatch_event(WindowEvent::CloseRequested);
        assert!(!native.window().is_visible());
        assert!(commands.borrow().iter().any(|command| {
            matches!(command, Command::CancelFileList { id: closed, generation: closed_generation }
                if *closed == id && *closed_generation == generation)
        }));
        midnight_ui::refresh(&main, &state.borrow());
        assert_eq!(windows.borrow().len(), cycle + 2);
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
