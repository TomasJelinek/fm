//! Exercise actual independent Slint windows, including callback reentrancy and bounds.
#[path = "support/modeless.rs"]
mod modeless;
use midnight_core::*;
use midnight_ui::{FindWindow, MainWindow, Presenter};
use modeless::{Windows, click};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

fn draw(window: &MinimalSoftwareWindow, width: u32, height: u32) {
    modeless::draw(window, width, height, "find");
}

#[test]
fn find_does_not_open_a_tool_after_the_main_window_is_hidden() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let main = MainWindow::new().unwrap();
    let root = std::env::temp_dir();
    let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
    midnight_ui::install(&main, state.clone(), Rc::new(|_| true));
    main.show().unwrap();
    main.hide().unwrap();

    main.invoke_find_files(0);
    assert_eq!(
        windows.borrow().len(),
        1,
        "a closed owner cannot own a dialog"
    );
    for _ in 0..4 {
        midnight_ui::refresh(&main, &state.borrow());
    }
    assert_eq!(
        windows.borrow().len(),
        1,
        "refresh must not retry failed creation"
    );
    main.invoke_close_find_windows();
    state.borrow_mut().retire_all();
}

#[test]
fn wildcard_search_streamed_orders_and_restart_preserve_backend_reclamation() {
    use std::sync::Arc;
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let main = MainWindow::new().unwrap();
    let root = std::env::temp_dir().join("Attachments/ooa-fixture/long-native-result-root");
    let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    midnight_ui::install(
        &main,
        state.clone(),
        Rc::new({
            let commands = commands.clone();
            move |command| {
                commands.borrow_mut().push(command);
                true
            }
        }),
    );
    main.show().unwrap();
    draw(&windows.borrow()[0], 1000, 700);
    main.invoke_find_files(0);
    let find = windows.borrow()[1].clone();
    draw(&find, 860, 620);
    click(&find, "Find");
    let request = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::StartFind(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(request.masks, "*.*");
    assert!(request.recursive);
    let results: Vec<_> = ["b.rs", "a.txt"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            Arc::new(FindResult {
                id: FindResultId(index as u64 + 1),
                duplicate_group: None,
                duplicate_group_size: 0,
                entry: FindEntry {
                    path: root.join(name),
                    kind: EntryKind::File,
                    metadata: EntryMetadata {
                        size: Some(1),
                        ..Default::default()
                    },
                    identity: TransferIdentity {
                        device: 1,
                        object: [index as u8; 16],
                    },
                    revision: FindRevision([0; 6]),
                },
            })
        })
        .collect();
    let apply = |update| {
        state.borrow_mut().apply(Event::Find(Box::new(FindEvent {
            session: request.session,
            generation: request.generation,
            update,
        })))
    };
    apply(FindUpdate::Batch(results.into()));
    midnight_ui::refresh(&main, &state.borrow());
    let old = Arc::new(FindOrder {
        revision: 0,
        sort: FindSortSpec::default(),
        ids: vec![FindResultId(1), FindResultId(2)].into(),
        positions: vec![0, 1].into(),
    });
    let old_weak = Arc::downgrade(&old);
    apply(FindUpdate::Sorted(old));
    midnight_ui::refresh(&main, &state.borrow());
    apply(FindUpdate::Sorted(Arc::new(FindOrder {
        revision: 0,
        sort: FindSortSpec::default(),
        ids: vec![FindResultId(2), FindResultId(1)].into(),
        positions: vec![1, 0].into(),
    })));
    // Model the application's worker cleanup before painting. The row model now
    // owns the last reference to the previous potentially large permutation.
    let retired = state.borrow_mut().take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert!(old_weak.upgrade().is_some());
    midnight_ui::refresh(&main, &state.borrow());
    assert!(
        old_weak.upgrade().is_some(),
        "refresh must retain large orders for backend disposal"
    );
    let retired = state.borrow_mut().take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert!(old_weak.upgrade().is_none());
    draw(&find, 860, 620);
    click(&find, "Sort by Extension");
    assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::SortFind(request) if request.sort.column == FindSortColumn::Extension)));
    apply(FindUpdate::Finished(FindSummary {
        completion: FindCompletion::Completed,
        progress: FindProgress::default(),
        reached_budgets: vec![],
        diagnostics: vec![],
    }));
    midnight_ui::refresh(&main, &state.borrow());
    draw(&find, 860, 620);
    click(&find, "Find");
    assert_eq!(
        commands
            .borrow()
            .iter()
            .filter(|command| matches!(command, Command::StartFind(_)))
            .count(),
        2
    );
    main.invoke_close_find_windows();
    main.hide().unwrap();
    state.borrow_mut().retire_all();
    let retired = state.borrow_mut().take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
}

#[test]
fn find_accessible_rows_select_without_revealing_and_focus_keyboard_navigation() {
    use i_slint_core::{
        accessibility::{AccessibilityAction, AccessibleStringProperty},
        items::AccessibleRole,
    };
    use midnight_ui::FindRow;
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    window.invoke_apply_preferences(0, 0, "".into(), 11, 15);
    window.set_rows(
        Rc::new(slint::VecModel::from(vec![
            FindRow {
                token: "7:9".into(),
                name: "Žluťoučký 日本語.txt".into(),
                extension: "txt".into(),
                path: "D:\\fixture\\文档".into(),
                size: "42 B".into(),
                modified: "07.11.2026 16:00".into(),
                ..Default::default()
            },
            FindRow {
                token: "7:10".into(),
                name: "next.txt".into(),
                path: "D:\\fixture".into(),
                ..Default::default()
            },
        ]))
        .into(),
    );
    let revealed = Rc::new(RefCell::new(Vec::new()));
    let captured = revealed.clone();
    window.on_reveal(move |token| captured.borrow_mut().push(token));
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 860, 620);
    let list = modeless::accessible_item(&native, AccessibleRole::List, "Find results")
        .expect("named accessible result list");
    assert_eq!(
        list.accessible_string_property(AccessibleStringProperty::ItemCount)
            .as_deref(),
        Some("2")
    );
    let label = "Žluťoučký 日本語.txt · txt · D:\\fixture\\文档 · 42 B · 07.11.2026 16:00";
    let row = modeless::accessible_item(&native, AccessibleRole::ListItem, label)
        .expect("complete accessible result row");
    assert!(
        row.geometry().size.height < 40.0,
        "Find results must be compact single rows"
    );
    let header =
        modeless::accessible_item(&native, AccessibleRole::Button, "Sort by Name").unwrap();
    let header_top = header.map_to_native_window(header.geometry().origin).y;
    let row_top = row.map_to_native_window(row.geometry().origin).y;
    assert!(
        header_top + header.geometry().size.height <= row_top + 0.5,
        "column header must be above, not centered through, result rows: header={header_top}, row={row_top}"
    );
    let title = modeless::accessible_item(&native, AccessibleRole::Text, "Name ↑").unwrap();
    let name =
        modeless::accessible_item(&native, AccessibleRole::Text, "Žluťoučký 日本語.txt").unwrap();
    assert!(
        (title.map_to_native_window(title.geometry().origin).x
            - name.map_to_native_window(name.geometry().origin).x)
            .abs()
            < 1.0,
        "header and filename text must share the column's left edge"
    );
    assert_eq!(
        row.accessible_string_property(AccessibleStringProperty::ItemSelectable)
            .as_deref(),
        Some("true")
    );
    row.accessible_action(&AccessibilityAction::Default);
    assert_eq!(window.get_selected_row(), 0);
    assert_eq!(
        row.accessible_string_property(AccessibleStringProperty::ItemSelected)
            .as_deref(),
        Some("true")
    );
    assert!(
        revealed.borrow().is_empty(),
        "accessible selection must not activate a result"
    );
    native.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::DownArrow.into(),
    });
    assert_eq!(
        window.get_selected_row(),
        1,
        "accessible selection must focus the result keyboard handler"
    );
    native.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Return.into(),
    });
    assert_eq!(
        revealed.borrow().as_slice(),
        &[slint::SharedString::from("7:10")]
    );
    window.set_running(true);
    window.set_current_directory("Scanning: D:\\fixture\\文档".into());
    window.set_progress("2 results · 42 files examined".into());
    modeless::draw(&native, 1400, 720, "find-populated-wide");
    modeless::draw(&native, 320, 200, "find-populated-minimum");
    window.hide().unwrap();
}

#[test]
fn find_column_headers_dispatch_pointer_sort_and_location_is_a_separate_line() {
    use i_slint_core::items::AccessibleRole;
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    let sorts = Rc::new(RefCell::new(Vec::new()));
    let captured = sorts.clone();
    window.on_sort(move |column| captured.borrow_mut().push(column));
    let path = "Scanning: D:\\Žluťoučký\\資料\\nested";
    window.set_current_directory(path.into());
    window.set_progress("42 files examined · 3 results".into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 1400, 720);
    for name in ["Name", "Extension", "Path", "Size", "Modified"] {
        click(&native, &format!("Sort by {name}"));
    }
    assert_eq!(sorts.borrow().as_slice(), [0, 1, 2, 3, 4]);
    let modes = Rc::new(RefCell::new(Vec::new()));
    let captured = modes.clone();
    window.on_search(move |draft| captured.borrow_mut().push(draft.mode));
    window.invoke_submit();
    for keys in [
        vec![Key::DownArrow, Key::Return],
        vec![Key::DownArrow, Key::Return],
    ] {
        let combo = modeless::accessible_item(&native, AccessibleRole::Combobox, "Find match mode")
            .unwrap();
        let rect = combo.geometry();
        let origin = combo.map_to_native_window(rect.origin);
        let position = slint::LogicalPosition::new(
            origin.x + rect.size.width / 2.0,
            origin.y + rect.size.height / 2.0,
        );
        for event in [
            WindowEvent::PointerPressed {
                position,
                button: slint::platform::PointerEventButton::Left,
            },
            WindowEvent::PointerReleased {
                position,
                button: slint::platform::PointerEventButton::Left,
            },
        ] {
            native.window().dispatch_event(event);
        }
        draw(&native, 1400, 720);
        for key in keys {
            native
                .window()
                .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
            native
                .window()
                .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
        }
        draw(&native, 1400, 720);
        window.invoke_submit();
    }
    assert_eq!(
        modes.borrow().as_slice(),
        [0, 1, 2],
        "actual chooser must preserve Masks and expose Regex/Expression"
    );
    let location = modeless::accessible_item(&native, AccessibleRole::Text, path).unwrap();
    let progress = modeless::accessible_item(
        &native,
        AccessibleRole::Text,
        "42 files examined · 3 results",
    )
    .unwrap();
    assert!(
        location.map_to_native_window(location.geometry().origin).y
            < progress.map_to_native_window(progress.geometry().origin).y
    );
    window.hide().unwrap();
}
#[test]
fn modeless_find_streams_results_and_closes_independently_at_320_by_200() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let main = MainWindow::new().unwrap();
    let root = std::env::temp_dir();
    let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let accepting = Rc::new(Cell::new(true));
    midnight_ui::install(
        &main,
        state.clone(),
        Rc::new({
            let commands = commands.clone();
            let accepting = accepting.clone();
            move |command| {
                if !accepting.get() {
                    return false;
                }
                commands.borrow_mut().push(command);
                true
            }
        }),
    );
    main.show().unwrap();
    draw(&windows.borrow()[0], 1000, 700);
    main.invoke_find_files(0);
    main.invoke_find_files(1);
    assert_eq!(
        windows.borrow().len(),
        3,
        "Find must use separate native windows"
    );
    let first = windows.borrow()[1].clone();
    let second = windows.borrow()[2].clone();
    draw(&first, 320, 200);
    click(&first, "Find");
    let request = commands
        .borrow()
        .iter()
        .find_map(|c| match c {
            Command::StartFind(r) => Some(r.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(request.roots.as_slice(), std::slice::from_ref(&root));
    assert!(request.recursive);
    let entry = FindEntry {
        path: root.join("Žluťoučký 日本語.txt"),
        kind: EntryKind::File,
        metadata: EntryMetadata {
            size: Some(42),
            ..Default::default()
        },
        identity: TransferIdentity {
            device: 1,
            object: [1; 16],
        },
        revision: FindRevision([0; 6]),
    };
    state.borrow_mut().apply(Event::Find(Box::new(FindEvent {
        session: request.session,
        generation: request.generation,
        update: FindUpdate::Batch(
            vec![std::sync::Arc::new(FindResult {
                id: FindResultId(1),
                duplicate_group: None,
                duplicate_group_size: 0,
                entry,
            })]
            .into(),
        ),
    })));
    midnight_ui::refresh(&main, &state.borrow());
    draw(&first, 860, 620);
    draw(&first, 320, 200);
    click(&first, "Stop");
    assert!(
        commands.borrow().iter().any(
            |c| matches!(c, Command::CancelFind { session, .. } if *session == request.session)
        )
    );
    accepting.set(false);
    click(&first, "Close");
    assert!(
        first.window().is_visible(),
        "full queue must keep the search owner visible"
    );
    first.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(
        first.window().is_visible(),
        "native X must also wait for queue admission"
    );
    accepting.set(true);
    draw(&first, 320, 200);
    click(&first, "Details");
    draw(&first, 320, 200);
    click(&first, "Copy");
    assert!(commands.borrow().iter().any(
        |c| matches!(c, Command::CopyText { text, .. } if text.contains("Request queue is busy"))
    ));
    draw(&first, 320, 200);
    first.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(
        !first.window().is_visible(),
        "native X follows semantic session close"
    );
    assert!(second.window().is_visible());
    assert!(main.window().is_visible());
    main.invoke_close_find_windows();
    assert!(
        !second.window().is_visible(),
        "host shutdown must close its modeless windows"
    );
    midnight_ui::refresh(&main, &state.borrow());
    assert_eq!(
        windows.borrow().len(),
        3,
        "late refresh must not resurrect closed host windows"
    );
    assert!(
        commands
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::CloseFind(id) if *id != request.session))
    );
    main.hide().unwrap();
    state.borrow_mut().retire_all();
}

#[test]
fn find_actions_and_details_remain_accessible_in_every_theme_and_density() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    let closed = Rc::new(Cell::new(0));
    let copies = Rc::new(Cell::new(0));
    window.on_close_session({
        let closed = closed.clone();
        move || closed.set(closed.get() + 1)
    });
    window.on_copy_details({
        let copies = copies.clone();
        move || copies.set(copies.get() + 1)
    });
    window.set_error("Access denied\n\nFolder: C:\\Žluťoučký 日本語\\Long folder".into());
    window.set_details("100 files · 12 folders\n1 access error\nNo partial content match was treated as a complete result.".into());
    window.set_attention(true);
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    for scheme in midnight_core::PreferenceTheme::ALL.map(|theme| theme.index()) {
        for density in 0..2 {
            window.invoke_apply_preferences(scheme, density, "".into(), 11, 15);
            for (width, height) in [(860, 620), (320, 200)] {
                window.set_show_details(false);
                draw(&native, width, height);
                click(&native, "Details");
                draw(&native, width, height);
                click(&native, "Copy");
                click(&native, "Query");
                click(&native, "Close");
            }
        }
    }
    let cases = (PreferenceTheme::ALL.len() * 2 * 2) as i32;
    assert_eq!(closed.get(), cases);
    assert_eq!(copies.get(), cases);
    window.hide().unwrap();
}
