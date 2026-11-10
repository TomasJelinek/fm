use super::*;
use slint::platform::{PointerEventButton, WindowEvent};
use std::sync::Arc;

fn frame(revision: u64, sequence: u64, position: f64) -> ViewerOutput {
    ViewerOutput::Frame(Box::new(ViewerFrame {
        generation: 7,
        position_sequence: [sequence, 0],
        revision,
        mode: 1,
        modes: vec![1],
        wrap_enabled: false,
        wrap_available: true,
        image: None,
        width: 2,
        height: 2,
        pixels: vec![255; 16].into(),
        tiles: None,
        accessible_text: Arc::from("Captured text"),
        position,
        vertical_extent: 0.1,
        horizontal_position: 0.0,
        horizontal_extent: 1.0,
        caption: "Synthetic text".into(),
        status: "Ready".into(),
    }))
}

#[test]
fn text_scrollbar_holds_latest_thumb_and_keeps_old_content_until_exact_ack_after_release() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    present_outputs(&open, &service, &|_| {}, vec![frame(10, 0, 0.2)]);
    let bar = &open.window;
    let height = bar.get_vertical_bar_height();
    assert!(height > 100.0);
    let thumb = (height * bar.get_vertical_extent()).max(28.0).min(height);
    let at = |position| {
        slint::LogicalPosition::new(
            bar.get_vertical_bar_left() + bar.get_vertical_bar_width() / 2.0,
            bar.get_vertical_bar_top() + position * (height - thumb) + thumb / 2.0,
        )
    };
    bar.window()
        .dispatch_event(WindowEvent::PointerMoved { position: at(0.2) });
    bar.window().dispatch_event(WindowEvent::PointerPressed {
        position: at(0.2),
        button: PointerEventButton::Left,
    });
    bar.window()
        .dispatch_event(WindowEvent::PointerMoved { position: at(0.8) });
    assert!((open.window.get_position() - 0.8).abs() < 0.0001);
    assert!((open.window.get_displayed_position() - 0.2).abs() < 0.0001);
    present_outputs(&open, &service, &|_| {}, vec![frame(11, 0, 0.3)]);
    assert!((open.window.get_position() - 0.8).abs() < 0.0001);
    assert!((open.window.get_displayed_position() - 0.2).abs() < 0.0001);
    bar.window().dispatch_event(WindowEvent::PointerReleased {
        position: at(0.8),
        button: PointerEventButton::Left,
    });
    present_outputs(&open, &service, &|_| {}, vec![frame(12, 0, 0.4)]);
    assert!((bar.get_vertical_thumb_position() - 0.8).abs() < 0.0001);
    assert!((bar.get_displayed_position() - 0.2).abs() < 0.0001);
    present_outputs(&open, &service, &|_| {}, vec![frame(13, 1, 0.801)]);
    assert!((open.window.get_position() - 0.801).abs() < 0.0001);
    assert!((open.window.get_displayed_position() - 0.801).abs() < 0.0001);
    manager.close_all();
}

#[test]
fn same_mode_selection_keeps_current_position_context_usable() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    present_outputs(&open, &service, &|_| {}, vec![frame(10, 0, 0.2)]);
    open.window.invoke_change_mode(0);
    open.window.invoke_seek(0.7);
    assert!((open.window.get_position() - 0.7).abs() < 0.0001);
    present_outputs(&open, &service, &|_| {}, vec![frame(11, 1, 0.701)]);
    assert!((open.window.get_displayed_position() - 0.701).abs() < 0.0001);
    manager.close_all();
}

#[test]
fn newer_completed_content_is_presented_during_drag_without_clearing_latest_thumb() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let received = submitted.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        received.borrow_mut().push(input);
        true
    });
    present_outputs(&open, &service, &|_| {}, vec![frame(10, 0, 0.2)]);
    open.window.invoke_seek(0.5);
    open.window.invoke_seek(0.8);
    assert_eq!(submitted.borrow().len(), 2);
    present_outputs(&open, &service, &|_| {}, vec![frame(11, 1, 0.501)]);
    assert!((open.window.get_position() - 0.8).abs() < 0.0001);
    assert!((open.window.get_displayed_position() - 0.501).abs() < 0.0001);
    present_outputs(&open, &service, &|_| {}, vec![frame(12, 1, 0.3)]);
    assert!((open.window.get_displayed_position() - 0.501).abs() < 0.0001);
    present_outputs(&open, &service, &|_| {}, vec![frame(13, 2, 0.801)]);
    assert!((open.window.get_position() - 0.801).abs() < 0.0001);
    assert!((open.window.get_displayed_position() - 0.801).abs() < 0.0001);
    manager.close_all();
}

#[test]
fn refused_file_and_mode_changes_do_not_fence_the_current_text_context() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let mut initial = frame(10, 0, 0.2);
    if let ViewerOutput::Frame(frame) = &mut initial {
        frame.modes = vec![1, 2];
    }
    present_outputs(&open, &service, &|_| {}, vec![initial]);
    // Closed input deterministically exercises refusal without racing a real worker.
    open.session.close();
    open.window.invoke_seek(0.7);
    let context = open.window.get_position_context();
    open.window.invoke_change_mode(1);
    assert!(!dispatch_key(&open, "viewer.next-file"));
    assert_eq!(open.window.get_position_context(), context);
    assert_eq!(
        open.text_position.borrow().positions(),
        [f64::from(0.7_f32), 0.0]
    );
    present_outputs(&open, &service, &|_| {}, vec![frame(11, 1, 0.701)]);
    assert!((open.window.get_displayed_position() - 0.701).abs() < 0.0001);
    manager.close_all();
}

#[test]
fn accepted_later_navigation_cancels_an_unsubmitted_scrollbar_retry() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    present_outputs(&open, &service, &|_| {}, vec![frame(10, 0, 0.2)]);
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let received = submitted.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        received.borrow_mut().push(input);
        true
    });
    open.text_position.borrow_mut().request(false, 0.9);
    // This models enqueue backpressure before the next accepted user gesture.
    open.text_position.borrow_mut().flush(|_| false);
    open.window.invoke_scroll(0.0, 20.0);
    open.text_position
        .borrow_mut()
        .flush(|_| panic!("superseded scrollbar retry crossed the wheel input"));
    assert_eq!(open.text_position.borrow().positions(), [0.2, 0.0]);
    assert!(matches!(
        submitted.borrow().as_slice(),
        [ViewerInput::Scroll { x: 0.0, y: 20.0 }]
    ));
    manager.close_all();
}
