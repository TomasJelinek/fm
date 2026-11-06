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
