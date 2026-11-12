//! Existing Attributes/Times/Case controls use typed authority from either expanded pane.
#[path = "support/tree_filters.rs"]
mod support;
#[path = "support/ui.rs"]
mod ui;
use midnight_core::*;
use support::Harness;

fn exercise(pane: PaneId, case: bool) {
    let mut h = Harness::new(pane, false);
    h.ui.window.invoke_activate(pane.index() as i32);
    let visible = h.presenter.borrow().visible_directory(pane).unwrap();
    assert!((h.send)(Command::PaneCommitted(visible)));
    let view = h.view.as_ref().unwrap();
    let child = view.rows.iter().find(|row| row.depth > 0).unwrap();
    let name = child.entry.display_name.clone();
    let position = h.rows().iter().position(|row| row.name == name).unwrap();
    h.ui.window
        .invoke_row_click(pane.index() as i32, position as i32, false, false);
    h.ui.window.invoke_focus_files();
    midnight_ui::refresh(&h.ui.window, &h.presenter.borrow());
    h.ui.window.invoke_open_metadata(pane.index() as i32, case);
    let mut captured = false;
    loop {
        let event = h.receive();
        match &event {
            Event::Tree(event) => {
                if let TreeEvent::MetadataCaptured {
                    stamp,
                    request,
                    result,
                    ..
                } = event.as_ref()
                {
                    assert!(!captured);
                    let sources = result.as_ref().expect("native descendant capture");
                    assert_eq!(sources.root_count(), 1);
                    assert_eq!(sources.stamp(), *stamp);
                    assert_eq!(sources.request(), *request);
                    captured = true;
                }
            }
            Event::Operation(OperationEvent::Progress(progress))
                if progress.state == OperationState::Failed =>
            {
                assert!(captured);
                assert_eq!(progress.kind, MutationKind::EditMetadata);
                // This bridge fixture installs no mutation service. It must reach the real
                // backend preparation boundary without manufacturing a flat path request.
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
fn native_left_tree_attributes_use_captured_descendant() {
    exercise(PaneId::Left, false);
}
#[test]
fn native_right_tree_attributes_use_captured_descendant() {
    exercise(PaneId::Right, false);
}
#[test]
fn native_left_tree_case_uses_captured_descendant() {
    exercise(PaneId::Left, true);
}
#[test]
fn native_right_tree_case_uses_captured_descendant() {
    exercise(PaneId::Right, true);
}
