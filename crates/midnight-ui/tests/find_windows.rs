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
fn find_accessible_rows_select_without_revealing_and_focus_keyboard_navigation() {
    use i_slint_core::{
        accessibility::{AccessibilityAction, AccessibleStringProperty},
        items::AccessibleRole,
    };
    use midnight_ui::FindRow;
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    window.set_rows(
        Rc::new(slint::VecModel::from(vec![
            FindRow {
                token: "7:9".into(),
                name: "Žluťoučký 日本語.txt".into(),
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
    let label = "Žluťoučký 日本語.txt\nD:\\fixture\\文档\n42 B\n07.11.2026 16:00";
    let row = modeless::accessible_item(&native, AccessibleRole::ListItem, label)
        .expect("complete accessible result row");
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
                id: FindResultId(0),
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
    accepting.set(true);
    draw(&first, 320, 200);
    click(&first, "Details");
    draw(&first, 320, 200);
    click(&first, "Copy");
    assert!(commands.borrow().iter().any(
        |c| matches!(c, Command::CopyText { text, .. } if text.contains("Request queue is busy"))
    ));
    draw(&first, 320, 200);
    first.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    assert!(!first.window().is_visible());
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
    for scheme in 0..9 {
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
    assert_eq!(closed.get(), 36);
    assert_eq!(copies.get(), 36);
    window.hide().unwrap();
}
