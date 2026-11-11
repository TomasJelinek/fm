//! Actual pointer gestures must keep their original rows and leave clicks unchanged.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_ui::FileRow;
use slint::{
    ComponentHandle, ModelRc, VecModel,
    platform::{PointerEventButton, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

fn pointer(test: &ui::TestUi, event: WindowEvent) {
    test.window.window().dispatch_event(event);
}

#[test]
fn threshold_drag_uses_stable_identity_in_every_layout_and_suppresses_click() {
    let mut test = ui::TestUi::new();
    let rows = Rc::new(VecModel::from(vec![FileRow {
        identity: "immutable-source".into(),
        name: "source 文档.txt".into(),
        selected: true,
        ..Default::default()
    }]));
    test.window.set_left_rows(ModelRc::from(rows));
    let drags = Rc::new(RefCell::new(Vec::new()));
    let captured = drags.clone();
    test.window.on_row_drag(move |pane, row, identity, _, _| {
        captured.borrow_mut().push((pane, row, identity));
        true
    });
    let clicks = Rc::new(RefCell::new(Vec::new()));
    let captured = clicks.clone();
    test.window
        .on_row_click(move |pane, row, _, _| captured.borrow_mut().push((pane, row)));
    test.window
        .on_open(|_, _| panic!("drag release must never open the row"));
    for mode in [0, 1, 2] {
        test.window.set_left_view_mode(mode);
        test.draw();
        let start = ui::root_position(&test.window, "source 文档.txt", AccessibleRole::ListItem);
        let button = PointerEventButton::Left;
        pointer(
            &test,
            WindowEvent::PointerPressed {
                position: start,
                button,
            },
        );
        pointer(
            &test,
            WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(start.x + 3.0, start.y),
            },
        );
        assert_eq!(
            drags.borrow().len(),
            mode as usize,
            "small movement is a click"
        );
        let destination = slint::LogicalPosition::new(start.x + 16.0, start.y + 2.0);
        pointer(
            &test,
            WindowEvent::PointerMoved {
                position: destination,
            },
        );
        assert_eq!(
            drags.borrow().last(),
            Some(&(0, 0, "immutable-source".into()))
        );
        assert_eq!(
            drags.borrow().len(),
            mode as usize + 1,
            "one preparation per gesture"
        );
        pointer(
            &test,
            WindowEvent::PointerReleased {
                position: destination,
                button,
            },
        );
        assert!(
            clicks.borrow().is_empty(),
            "dragging a selected row retains its group"
        );
    }
    ui::click_root(&test.window, "source 文档.txt", AccessibleRole::ListItem);
    assert_eq!(clicks.borrow().as_slice(), &[(0, 0)]);
}

#[test]
fn installed_drag_feedback_shows_pending_copy_move_rejection_and_escape_cancels() {
    use midnight_core::*;
    use midnight_ui::Presenter;
    use slint::{Model, platform::Key};
    use std::sync::Arc;
    let mut test = ui::TestUi::new();
    let root = std::env::temp_dir();
    let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let send: Rc<dyn Fn(Command) -> bool> = Rc::new({
        let commands = commands.clone();
        move |command| {
            commands.borrow_mut().push(command);
            true
        }
    });
    midnight_ui::install(&test.window, state.clone(), send.clone());
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
    let entries: Arc<[Entry]> = vec![
        Entry::new(EntryId(1), root.join("source 文档.txt"), EntryKind::File),
        Entry::new(EntryId(2), root.join("target 資料"), EntryKind::Directory),
    ]
    .into();
    state.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    state
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    midnight_ui::refresh(&test.window, &state.borrow());
    test.draw();
    let source = ui::root_position(&test.window, "source 文档.txt", AccessibleRole::ListItem);
    let target = ui::root_position(&test.window, "target 資料", AccessibleRole::ListItem);
    let rows = test.window.get_left_rows();
    let (row, identity) = rows
        .iter()
        .enumerate()
        .find_map(|(row, entry)| (entry.name == "source 文档.txt").then_some((row, entry.identity)))
        .unwrap();
    assert!(
        test.window
            .invoke_row_drag(0, row as i32, identity, source.x, source.y)
    );
    assert_eq!(test.window.get_drag_feedback_state(), 1);
    assert_eq!(test.window.get_drag_label(), "Preparing drag…");
    let id = commands
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::PrepareDrag { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    state.borrow_mut().apply_with_sender(
        Event::DragDrop(DragDropEvent::Prepared { id, item_count: 1 }),
        send.as_ref(),
    );
    test.window.invoke_drag_moved(target.x, target.y);
    assert_eq!(test.window.get_drag_feedback_state(), 2);
    let first = commands
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::PreviewInternalDrop(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    for (intent, expected, caption) in [
        (Some(TransferIntent::Copy), 3, "Copy"),
        (Some(TransferIntent::Move), 4, "Move"),
        (None, 5, "Not allowed"),
    ] {
        if expected == 4 {
            test.window.invoke_drag_modifiers(false, true, false);
        }
        if expected == 5 {
            test.window.invoke_drag_modifiers(false, false, false);
        }
        let current = commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::PreviewInternalDrop(request) => Some(request.revision),
                _ => None,
            })
            .unwrap();
        state.borrow_mut().apply_with_sender(
            Event::DragDrop(DragDropEvent::InternalPreview {
                id,
                revision: current,
                intent,
            }),
            send.as_ref(),
        );
        test.window.invoke_drag_moved(target.x, target.y);
        test.draw();
        assert_eq!(test.window.get_drag_feedback_state(), expected);
        assert!(test.window.get_drag_label().starts_with(caption));
        let badge = ui::root_item(
            &test.window,
            test.window.get_drag_label().as_str(),
            AccessibleRole::Text,
        );
        let area = badge.geometry();
        let at = badge.map_to_native_window(area.origin);
        assert!(
            at.x >= 0.0
                && at.y >= 0.0
                && at.x + area.width() <= test.width as f32 + 0.5
                && at.y + area.height() <= test.height as f32 + 0.5
        );
    }
    test.window.invoke_drag_modifiers(true, false, false);
    assert!(first.cancel.is_cancelled());
    assert_eq!(test.window.get_drag_feedback_state(), 2);
    let pending = commands
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::PreviewInternalDrop(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    test.window.invoke_focus_files();
    ui::key(&test.window, Key::Escape);
    assert!(!test.window.get_drag_active());
    assert_eq!(test.window.get_drag_feedback_state(), 0);
    assert!(pending.cancel.is_cancelled());
    test.window.invoke_drag_released(target.x, target.y);
    assert!(
        commands
            .borrow()
            .iter()
            .all(|command| !matches!(command, Command::InternalDrop(_))),
        "release after Escape cannot submit a transfer"
    );
    test.window.invoke_drag_host_closed();
    test.window.hide().unwrap();
    state.borrow_mut().retire_all();
}
