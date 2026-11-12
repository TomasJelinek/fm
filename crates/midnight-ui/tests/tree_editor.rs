//! Actual F4 input crosses the installed presenter and production tree capture lane.
#[path = "support/tree_filters.rs"]
mod support;
#[path = "support/ui.rs"]
mod ui;
use midnight_core::*;
use slint::platform::Key;
use support::Harness;

fn exercise(pane: PaneId) {
    let mut h = Harness::new(pane, false);
    h.ui.window.invoke_activate(pane.index() as i32);
    let row = h
        .view
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .find(|row| row.entry.display_name == "root.txt")
        .unwrap()
        .id;
    assert!(
        h.view
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .any(|row| row.depth > 0)
    );
    let position = h
        .rows()
        .iter()
        .position(|row| row.name == "root.txt")
        .unwrap();
    h.ui.window
        .invoke_row_click(pane.index() as i32, position as i32, false, false);
    h.ui.window.invoke_focus_files();
    midnight_ui::refresh(&h.ui.window, &h.presenter.borrow());
    ui::key(&h.ui.window, Key::F4);
    let mut captured = false;
    loop {
        let event = h.receive();
        match &event {
            Event::Tree(event) => {
                if let TreeEvent::EditorCaptured { stamp, result, .. } = event.as_ref() {
                    assert!(!captured, "one F4 must not capture twice");
                    let source = result.as_ref().expect("native root file capture");
                    assert_eq!(stamp.pane, pane);
                    assert_eq!(source.stamp(), *stamp);
                    assert_eq!(source.row(), row);
                    captured = true;
                }
            }
            Event::DesktopActionFinished { result, .. } => {
                assert!(captured, "typed source must precede native launch");
                // This fixture intentionally installs no EditorService. It proves
                // the full request route without spawning a desktop application.
                assert!(
                    result
                        .as_ref()
                        .unwrap_err()
                        .to_string()
                        .contains("External editing is unavailable")
                );
                h.apply_event(event);
                break;
            }
            _ => {}
        }
        h.apply_event(event);
    }
    assert!(captured);
}

#[test]
fn tree_editor_f4_routes_native_left_root_file_through_real_capture() {
    exercise(PaneId::Left);
}
#[test]
fn tree_editor_f4_routes_native_right_root_file_through_real_capture() {
    exercise(PaneId::Right);
}
