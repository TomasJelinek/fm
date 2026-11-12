//! Real F2/F7 input uses the installed presenter and backend tree authority.
#[path = "support/tree_filters.rs"]
mod support;
#[path = "support/ui.rs"]
mod ui;
use midnight_core::*;
use slint::platform::Key;
use support::Harness;

fn exercise(pane: PaneId, rename: bool) {
    let mut h = Harness::new(pane, false);
    h.ui.window.invoke_activate(pane.index() as i32);
    // The app's VisibleSync acknowledges committed native snapshots independently of
    // watches. This shared fixture normally does that only for watch tests.
    let visible = h.presenter.borrow().visible_directory(pane).unwrap();
    assert!((h.send)(Command::PaneCommitted(visible)));
    let view = h.view.as_ref().unwrap();
    let child = view.rows.iter().find(|row| row.depth > 0).unwrap();
    let row = child.id;
    let name = child.entry.display_name.clone();
    let position = h.rows().iter().position(|row| row.name == name).unwrap();
    h.ui.window
        .invoke_row_click(pane.index() as i32, position as i32, false, false);
    h.ui.window.invoke_focus_files();
    midnight_ui::refresh(&h.ui.window, &h.presenter.borrow());
    ui::key(&h.ui.window, if rename { Key::F2 } else { Key::F7 });
    let mut captured = false;
    loop {
        let event = h.receive();
        match &event {
            Event::Tree(event) => {
                if let TreeEvent::RenameCaptured {
                    stamp,
                    request,
                    result,
                } = event.as_ref()
                {
                    assert!(rename && !captured);
                    let source = result.as_ref().expect("native descendant capture");
                    assert_eq!(source.row(), row);
                    assert_eq!(source.stamp(), *stamp);
                    assert_eq!(source.request(), *request);
                    captured = true;
                }
            }
            Event::Operation(OperationEvent::Progress(progress))
                if progress.state == OperationState::Failed =>
            {
                assert_eq!(captured, rename);
                assert_eq!(
                    progress.kind,
                    if rename {
                        MutationKind::Rename
                    } else {
                        MutationKind::CreateDirectory
                    }
                );
                // No mutation service is installed; routing reaches real backend preparation safely.
                assert!(
                    progress.message.contains("does not support mutations"),
                    "{}",
                    progress.message
                );
                h.apply_event(event);
                break;
            }
            _ => {}
        }
        h.apply_event(event);
    }
}
#[test]
fn f2_native_left_expanded_descendant_routes_typed_capture() {
    exercise(PaneId::Left, true);
}
#[test]
fn f2_native_right_expanded_descendant_routes_typed_capture() {
    exercise(PaneId::Right, true);
}
#[test]
fn f7_native_expanded_tree_keeps_existing_create_directory_route() {
    exercise(PaneId::Left, false);
}
