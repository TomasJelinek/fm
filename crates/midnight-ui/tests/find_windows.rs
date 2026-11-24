//! Exercise actual independent Slint windows, including callback reentrancy and bounds.
use crate::support::modeless;
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
                archive: None,
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
    // The host keeps the selection: this one selects the row a click asks for.
    let marks = Rc::new(slint::VecModel::from(vec![false, false]));
    window.set_marks(marks.clone().into());
    window.on_select_row({
        let marks = marks.clone();
        move |row, toggle, range| {
            assert!(
                !toggle && !range,
                "an accessible activation is a plain click"
            );
            for index in 0..2 {
                slint::Model::set_row_data(&*marks, index, index as i32 == row);
            }
        }
    });
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
    for (width, height, name) in [
        (1400, 720, "find-populated-wide"),
        (320, 200, "find-populated-minimum"),
    ] {
        modeless::draw_or_layout(&native, width, height, name, false);
        let list = modeless::accessible_item(&native, AccessibleRole::List, "Find results")
            .expect("populated results stay accessible");
        assert_eq!(
            list.accessible_string_property(AccessibleStringProperty::ItemCount)
                .as_deref(),
            Some("2"),
            "{width}x{height}"
        );
        assert!(
            modeless::accessible_item(&native, AccessibleRole::Button, "Stop").is_some(),
            "a running search keeps Stop at {width}x{height}"
        );
    }
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
    // The status line is a button that opens the details.
    let progress = modeless::accessible_item(
        &native,
        AccessibleRole::Button,
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
                archive: None,
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
    let status = status_line(&first);
    click_footer_action(&first, 320, 200, &status, "find-busy", true);
    draw(&first, 320, 200);
    click_footer_action(&first, 320, 200, "Copy", "find-busy", true);
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
    let status = "Error · click for details · 0 found · 0 examined";
    window.set_progress(status.into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    // Themes select colours only (ui/theme.slint), so every theme is laid out and
    // clicked, but only the first one is software-rendered per density and size.
    let themes = midnight_core::PreferenceTheme::ALL.map(|theme| theme.index());
    for scheme in themes {
        let render = scheme == themes[0];
        for density in 0..2 {
            window.invoke_apply_preferences(scheme, density, "".into(), 11, 15);
            let case = format!("find-theme{scheme}-density{density}");
            for (width, height) in [(860, 620), (320, 200)] {
                window.set_show_details(false);
                modeless::draw_or_layout(&native, width, height, &case, render);
                // The status line, not a footer button, opens the details.
                click_footer_action(&native, width, height, status, &case, render);
                assert!(
                    window.get_show_details(),
                    "Details: theme={scheme}, density={density}, {width}x{height}"
                );
                let details = format!("{case}-details");
                modeless::draw_or_layout(&native, width, height, &details, render);
                click_footer_action(&native, width, height, "Copy", &details, render);
                click_footer_action(&native, width, height, "Back", &details, render);
                assert!(
                    !window.get_show_details(),
                    "Back: theme={scheme}, density={density}, {width}x{height}"
                );
                click_footer_action(&native, width, height, "Close", &case, render);
            }
        }
    }
    let cases = (PreferenceTheme::ALL.len() * 2 * 2) as i32;
    assert_eq!(closed.get(), cases);
    assert_eq!(copies.get(), cases);
    window.hide().unwrap();
}

/// The label of the last line that opens Find details (the status under the results).
fn status_line(native: &MinimalSoftwareWindow) -> String {
    use i_slint_core::{
        accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
        window::WindowInner,
    };
    fn walk(item: ItemRc, found: &mut Option<String>) {
        if item.accessible_role() == AccessibleRole::Button
            && item
                .accessible_string_property(AccessibleStringProperty::Description)
                .is_some_and(|text| text == "Click for more details")
        {
            *found = item
                .accessible_string_property(AccessibleStringProperty::Label)
                .map(|label| label.to_string());
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            walk(item, found);
        }
    }
    let mut found = None;
    let root = WindowInner::from_pub(native.window()).component();
    walk(ItemRc::new_root(root), &mut found);
    found.expect("Find shows a status line that opens details")
}

// Compact footers keep Close fixed and scroll the remaining real controls. A
// button's center can be inside the window but underneath that fixed Close area.
fn click_footer_action(
    native: &MinimalSoftwareWindow,
    width: u32,
    height: u32,
    label: &str,
    case: &str,
    render: bool,
) {
    use i_slint_core::{
        item_tree::{ItemRc, ParentItemTraversalMode},
        items::{AccessibleRole, Flickable},
    };
    let bounds = |item: &ItemRc| {
        let rect = item.geometry();
        let at = item.map_to_native_window(rect.origin);
        (at.x, at.y, at.x + rect.width(), at.y + rect.height())
    };
    for attempt in 0..4 {
        // Wheel input scrolls immediately (no animation), so layout suffices here.
        let name = format!("{case}-{label}-{attempt}");
        modeless::draw_or_layout(native, width, height, &name, render);
        // An action that does not fit beside Close is in the footer's More… menu.
        if modeless::accessible_item(native, AccessibleRole::Button, label).is_none()
            && modeless::accessible_item(native, AccessibleRole::Button, "More…").is_some()
        {
            click(native, "More…");
            click(native, label);
            return;
        }
        let item = modeless::accessible_item(native, AccessibleRole::Button, label)
            .unwrap_or_else(|| panic!("missing footer action {label} at {width}x{height}"));
        let area = bounds(&item);
        let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
        let mut scroll = None;
        while let Some(ancestor) = parent {
            if ancestor.downcast::<Flickable>().is_some() {
                let clip = bounds(&ancestor);
                let delta = if area.0 < clip.0 - 0.5 {
                    clip.0 - area.0
                } else if area.2 > clip.2 + 0.5 {
                    clip.2 - area.2
                } else {
                    0.0
                };
                if delta != 0.0 {
                    scroll = Some((clip, delta));
                    break;
                }
            }
            parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        let Some((clip, delta_x)) = scroll else {
            click(native, label);
            return;
        };
        native
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(clip.0 + 4.0, clip.1 + 4.0),
                delta_x,
                delta_y: 0.0,
            });
        slint::platform::update_timers_and_animations();
    }
    panic!("footer action {label} could not be exposed at {width}x{height}");
}

/// A Find window driven by the real presenter, with a search of `file-N.txt` results
/// (N bytes each), and the commands it sent.
struct Search {
    main: MainWindow,
    state: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    find: Rc<MinimalSoftwareWindow>,
    root: std::path::PathBuf,
    request: FindRequest,
}

impl Search {
    fn new(windows: &Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>, count: u64) -> Self {
        let main = MainWindow::new().unwrap();
        let root = std::env::temp_dir().join("find-multi-selection");
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
                Command::StartFind(request) => Some((**request).clone()),
                _ => None,
            })
            .unwrap();
        let search = Self {
            main,
            state,
            commands,
            find,
            root,
            request,
        };
        search.stream(1, count);
        search
    }

    /// Results `first..first + count` arrive.
    fn stream(&self, first: u64, count: u64) {
        let results: Vec<_> = (first..first + count)
            .map(|n| {
                std::sync::Arc::new(FindResult {
                    id: FindResultId(n),
                    duplicate_group: None,
                    duplicate_group_size: 0,
                    archive: None,
                    entry: FindEntry {
                        path: self.path(n),
                        kind: EntryKind::File,
                        metadata: EntryMetadata {
                            size: Some(n),
                            ..Default::default()
                        },
                        identity: TransferIdentity {
                            device: 1,
                            object: [n as u8; 16],
                        },
                        revision: FindRevision([0; 6]),
                    },
                })
            })
            .collect();
        self.apply(FindUpdate::Batch(results.into()));
    }

    fn apply(&self, update: FindUpdate) {
        self.state
            .borrow_mut()
            .apply(Event::Find(Box::new(FindEvent {
                session: self.request.session,
                generation: self.request.generation,
                update,
            })));
        midnight_ui::refresh(&self.main, &self.state.borrow());
        draw(&self.find, 860, 620);
    }

    fn finish(&self) {
        self.apply(FindUpdate::Finished(FindSummary {
            completion: FindCompletion::Completed,
            progress: FindProgress::default(),
            reached_budgets: vec![],
            diagnostics: vec![],
        }));
    }

    /// The first results sort into the order of `ids`.
    fn sort(&self, ids: &[u64]) {
        let mut positions = vec![0; ids.len()];
        for (index, id) in ids.iter().enumerate() {
            positions[*id as usize - 1] = index;
        }
        self.apply(FindUpdate::Sorted(std::sync::Arc::new(FindOrder {
            revision: 0,
            sort: FindSortSpec::default(),
            ids: ids.iter().map(|id| FindResultId(*id)).collect(),
            positions: positions.into(),
        })));
    }

    fn path(&self, n: u64) -> std::path::PathBuf {
        self.root.join(format!("file-{n}.txt"))
    }

    /// The list row that shows result `n`.
    fn row(&self, n: u64) -> i_slint_core::item_tree::ItemRc {
        use i_slint_core::{
            accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
            window::WindowInner,
        };
        fn walk(item: ItemRc, token: &str) -> Option<ItemRc> {
            if item.accessible_role() == AccessibleRole::ListItem
                && item
                    .accessible_string_property(AccessibleStringProperty::Value)
                    .is_some_and(|value| value == token)
            {
                return Some(item);
            }
            let mut child = item.first_child();
            while let Some(item) = child {
                child = item.next_sibling();
                if let Some(found) = walk(item, token) {
                    return Some(found);
                }
            }
            None
        }
        let token = format!("{}:{n}", self.request.generation.0);
        let root = WindowInner::from_pub(self.find.window()).component();
        walk(ItemRc::new_root(root), &token).unwrap_or_else(|| panic!("no row shows {token}"))
    }

    /// The results `1..=count` whose rows report themselves selected.
    fn selected(&self, count: u64) -> Vec<u64> {
        use i_slint_core::accessibility::AccessibleStringProperty;
        (1..=count)
            .filter(|n| {
                self.row(*n)
                    .accessible_string_property(AccessibleStringProperty::ItemSelected)
                    .is_some_and(|selected| selected == "true")
            })
            .collect()
    }

    /// Press and release `button` on result `n`'s row, holding `modifiers`.
    fn press(&self, n: u64, button: slint::platform::PointerEventButton, modifiers: &[Key]) {
        let row = self.row(n);
        let origin = row.map_to_native_window(row.geometry().origin);
        let position =
            slint::LogicalPosition::new(origin.x + 40.0, origin.y + row.geometry().height() / 2.0);
        let window = self.find.window();
        for modifier in modifiers {
            window.dispatch_event(WindowEvent::KeyPressed {
                text: (*modifier).into(),
            });
        }
        window.dispatch_event(WindowEvent::PointerPressed { position, button });
        window.dispatch_event(WindowEvent::PointerReleased { position, button });
        for modifier in modifiers.iter().rev() {
            window.dispatch_event(WindowEvent::KeyReleased {
                text: (*modifier).into(),
            });
        }
        draw(&self.find, 860, 620);
    }

    fn click(&self, n: u64, modifiers: &[Key]) {
        self.press(n, slint::platform::PointerEventButton::Left, modifiers);
    }

    fn key(&self, text: impl Into<slint::SharedString>, modifiers: &[Key]) {
        let text = text.into();
        let window = self.find.window();
        for modifier in modifiers {
            window.dispatch_event(WindowEvent::KeyPressed {
                text: (*modifier).into(),
            });
        }
        window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window.dispatch_event(WindowEvent::KeyReleased { text });
        for modifier in modifiers.iter().rev() {
            window.dispatch_event(WindowEvent::KeyReleased {
                text: (*modifier).into(),
            });
        }
        draw(&self.find, 860, 620);
    }

    /// The clipboard texts sent so far.
    fn copied(&self) -> Vec<String> {
        self.commands
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::CopyText { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// Run the captured panel conversion on a worker, then deliver its ordinary
    /// completion through the same event/sender path as the application.
    fn complete_feed_build(&self) {
        let request = self
            .commands
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::BuildFindFeed(request) => Some(request.clone()),
                _ => None,
            })
            .expect("Feed to panel submits a captured worker request");
        let receipt = std::thread::spawn(move || FindFeedBuildResult {
            request: request.stamp(),
            result: request.prepare(),
        })
        .join()
        .unwrap();
        self.state.borrow_mut().apply_with_sender(
            Event::FindFeedBuilt(Box::new(receipt)),
            &|command| {
                self.commands.borrow_mut().push(command);
                true
            },
        );
        midnight_ui::refresh(&self.main, &self.state.borrow());
    }

    fn close(self) {
        self.main.invoke_close_find_windows();
        self.main.hide().unwrap();
        self.state.borrow_mut().retire_all();
        let retired = self.state.borrow_mut().take_retired();
        std::thread::spawn(move || drop(retired)).join().unwrap();
    }
}

#[test]
fn find_results_select_several_rows_by_pointer_and_keys_and_keep_them_by_identity() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let search = Search::new(&windows, 6);
    // A click selects one row; Ctrl+click adds one; Shift+click selects the range from
    // the anchor (the Ctrl+clicked row), replacing the selection.
    search.click(1, &[]);
    assert_eq!(search.selected(6), [1]);
    search.click(3, &[Key::Control]);
    assert_eq!(search.selected(6), [1, 3]);
    search.click(5, &[Key::Shift]);
    assert_eq!(search.selected(6), [3, 4, 5]);
    let status = status_line(&search.find);
    assert!(
        status.contains("· 3 selected · "),
        "the status line counts the selection and its size: {status}"
    );
    // Results that stream in arrive unselected; a sort moves the selected rows.
    search.stream(7, 2);
    search.sort(&[8, 7, 6, 5, 4, 3, 2, 1]);
    assert_eq!(search.selected(8), [3, 4, 5]);
    // Shift+Down extends from the anchor (result 3) to the row under the cursor's.
    search.key(Key::DownArrow, &[Key::Shift]);
    assert_eq!(search.selected(8), [3, 4]);
    // Plain Down selects the row it reaches alone.
    search.key(Key::DownArrow, &[]);
    assert_eq!(search.selected(8), [3]);
    search.key("a", &[Key::Control]);
    assert_eq!(search.selected(8), [1, 2, 3, 4, 5, 6, 7, 8]);
    search.key("d", &[Key::Control]);
    assert!(search.selected(8).is_empty());
    // Space toggles the cursor's row; Insert toggles it and moves down.
    search.key(Key::Space, &[]);
    assert_eq!(search.selected(8), [3]);
    search.key(Key::Insert, &[]);
    assert!(search.selected(8).is_empty());
    search.key(Key::Insert, &[]);
    assert_eq!(search.selected(8), [2]);
    // Ctrl+Up moves the cursor alone (to results 2, then 3); Space then adds its row.
    search.key(Key::UpArrow, &[Key::Control]);
    search.key(Key::UpArrow, &[Key::Control]);
    assert_eq!(search.selected(8), [2]);
    search.key(Key::Space, &[]);
    assert_eq!(search.selected(8), [2, 3]);
    // A new search clears the selection.
    search.finish();
    click(&search.find, "Find");
    draw(&search.find, 860, 620);
    let status = status_line(&search.find);
    assert!(!status.contains("selected"), "{status}");
    search.close();
}

#[test]
fn selected_and_cursor_rows_look_distinct_in_every_theme() {
    use i_slint_core::{
        accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
        window::WindowInner,
    };
    use midnight_ui::FindRow;
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    // Rows 0 and 1 plain, 2 and 3 selected, 4 the cursor's: each kind on an even row,
    // compared with the plain row of the same parity.
    let rows: Vec<FindRow> = (0..5)
        .map(|index| FindRow {
            token: format!("1:{}", index + 1).into(),
            name: format!("r{index}").into(),
            ..Default::default()
        })
        .collect();
    window.set_rows(Rc::new(slint::VecModel::from(rows)).into());
    window.set_marks(Rc::new(slint::VecModel::from(vec![false, false, true, true, false])).into());
    window.set_selected_row(4);
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    let row = |token: &str| -> ItemRc {
        fn walk(item: ItemRc, token: &str) -> Option<ItemRc> {
            if item.accessible_role() == AccessibleRole::ListItem
                && item
                    .accessible_string_property(AccessibleStringProperty::Value)
                    .is_some_and(|value| value == token)
            {
                return Some(item);
            }
            let mut child = item.first_child();
            while let Some(item) = child {
                child = item.next_sibling();
                if let Some(found) = walk(item, token) {
                    return Some(found);
                }
            }
            None
        }
        let root = WindowInner::from_pub(native.window()).component();
        walk(ItemRc::new_root(root), token).unwrap()
    };
    for scheme in PreferenceTheme::ALL.map(|theme| theme.index()) {
        window.invoke_apply_preferences(scheme, 0, "".into(), 13, 17);
        let pixels = modeless::render(&native, 860, 620, &format!("find-selection-theme{scheme}"));
        // At a row's left: its cursor stroke, its selection square and, between the
        // square and the text, its background.
        let sample = |index: usize| {
            let item = row(&format!("1:{}", index + 1));
            let origin = item.map_to_native_window(item.geometry().origin);
            let height = item.geometry().height();
            let at =
                |x: f32, y: f32| pixels[(origin.y + y) as usize * 860 + (origin.x + x) as usize];
            (at(1.0, 2.0), at(4.0, height / 2.0), at(9.0, 3.0))
        };
        let [plain_even, plain_odd, selected_even, selected_odd, cursor] =
            [0, 1, 2, 3, 4].map(sample);
        let theme = format!("theme {scheme}");
        assert_ne!(selected_even.1, plain_even.1, "{theme}: selection square");
        assert_ne!(selected_odd.1, plain_odd.1, "{theme}: selection square");
        assert_ne!(
            selected_even.1, selected_even.2,
            "{theme}: square on its own row"
        );
        assert_ne!(
            selected_even.2, plain_even.2,
            "{theme}: selected background"
        );
        assert_ne!(selected_odd.2, plain_odd.2, "{theme}: selected background");
        assert_ne!(cursor.0, plain_even.0, "{theme}: cursor stroke");
        assert_ne!(cursor.2, plain_even.2, "{theme}: cursor background");
        assert_ne!(
            cursor.2, selected_even.2,
            "{theme}: cursor against selection"
        );
        for (index, selected) in [false, false, true, true, false].into_iter().enumerate() {
            assert_eq!(
                row(&format!("1:{}", index + 1))
                    .accessible_string_property(AccessibleStringProperty::ItemSelected)
                    .as_deref(),
                Some(if selected { "true" } else { "false" }),
                "{theme}: row {index} reports its selection"
            );
        }
    }
    window.hide().unwrap();
}

#[test]
fn find_copy_menu_and_feed_act_on_the_selected_results_or_the_acted_on_row() {
    use midnight_ui::FeedPresentation;
    use slint::{Model, platform::PointerEventButton};
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let search = Search::new(&windows, 5);
    search.finish();
    let raw = |n: u64| search.path(n).to_string_lossy().into_owned();
    search.click(2, &[]);
    search.click(4, &[Key::Control]);
    // Ctrl+C copies the full paths of the selection, which holds the cursor's row.
    search.key("c", &[Key::Control]);
    assert_eq!(search.copied(), [format!("{}\n{}", raw(2), raw(4))]);
    // The context menu (Shift+F10) on a selected row acts on the whole selection.
    search.key(Key::F10, &[Key::Shift]);
    click(&search.find, "Copy names (2)");
    assert_eq!(search.copied()[1], "file-2.txt\nfile-4.txt");
    // A right click outside the selection selects that row alone and acts on it.
    search.press(5, PointerEventButton::Right, &[]);
    assert_eq!(search.selected(5), [5]);
    click(&search.find, "Copy full paths");
    assert_eq!(search.copied()[2], raw(5));
    // After Ctrl+D, Ctrl+C acts on the cursor's row alone.
    search.key("d", &[Key::Control]);
    search.key("c", &[Key::Control]);
    assert_eq!(search.copied()[3], raw(5));
    // The Menu key opens the menu on the cursor's row; Down and Enter choose from it
    // (Go to file, View, Edit, Open, then Copy names).
    search.key(Key::Menu, &[]);
    for _ in 0..4 {
        search.key(Key::DownArrow, &[]);
    }
    search.key(Key::Return, &[]);
    assert_eq!(search.copied()[4], "file-5.txt");
    // Feed to panel: the selected results arrive marked, the cursor on the Find's.
    search.click(1, &[]);
    search.click(3, &[Key::Shift]);
    click(&search.find, "Feed to panel");
    let feed = search.main.global::<FeedPresentation>();
    assert!(!feed.get_left().open, "wait for the captured worker result");
    search.complete_feed_build();
    assert!(feed.get_left().open);
    let marks: Vec<bool> = feed
        .get_left_rows()
        .iter()
        .map(|row| row.selected)
        .collect();
    assert_eq!(marks, [true, true, true, false, false]);
    assert_eq!(feed.get_left().cursor, 2);
    let status = feed.get_left().status;
    let mut lines = status.lines();
    assert!(lines.next().unwrap().contains("file-3.txt"));
    assert!(lines.any(|line| line.starts_with("3 of 5 selected")));
    search.close();
}

fn late_find_history_prefill(draft: Option<&str>) {
    use i_slint_core::{
        accessibility::{AccessibilityAction, AccessibleStringProperty},
        items::AccessibleRole,
    };
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let main = MainWindow::new().unwrap();
    let root = std::env::temp_dir().join("late-find-mask-history");
    let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
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
    let input =
        modeless::accessible_item(&find, AccessibleRole::TextInput, "File name masks").unwrap();
    assert_eq!(
        input
            .accessible_string_property(AccessibleStringProperty::Value)
            .as_deref(),
        Some("*.*")
    );
    if let Some(draft) = draft {
        modeless::click_item(&find, &input, "File name masks");
        input.accessible_action(&AccessibilityAction::SetSelectionOffsets(0, 3));
        find.window()
            .dispatch_event(WindowEvent::KeyPressed { text: draft.into() });
        find.window()
            .dispatch_event(WindowEvent::KeyReleased { text: draft.into() });
    }
    let mut preferences = UserPreferences::default();
    preferences.find.mask_history = vec!["*.saved;文档*".into(), "*.older".into()];
    state
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(
            AcceptedPreferences {
                preferences,
                comparators: FileComparatorSnapshot::default(),
            },
        ))));
    midnight_ui::refresh(&main, &state.borrow());
    draw(&find, 860, 620);
    let expected = draft.unwrap_or("*.saved;文档*");
    assert_eq!(
        input
            .accessible_string_property(AccessibleStringProperty::Value)
            .as_deref(),
        Some(expected)
    );
    click(&find, "Find");
    let masks = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::StartFind(request) => Some(request.masks.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(masks, expected);
    main.invoke_close_find_windows();
    main.hide().unwrap();
    state.borrow_mut().retire_all();
    let retired = state.borrow_mut().take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
}

#[test]
fn late_loaded_find_history_prefills_an_untouched_filename_query() {
    late_find_history_prefill(None);
}

#[test]
fn late_loaded_find_history_preserves_a_user_edited_filename_query() {
    late_find_history_prefill(Some("*.my-draft"));
}
