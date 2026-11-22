//! A whole-file Copy in the window: Escape stops it before it closes the window, and the
//! copied text moves on to the clipboard service.
use super::image_presentation_tests::present as present_outputs;
use super::*;

#[test]
fn escape_stops_a_whole_file_copy_before_it_closes_the_window() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let received = Rc::new(RefCell::new(Vec::new()));
    let sink = received.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        sink.borrow_mut().push(input);
        true
    });
    let copied = RefCell::new(Vec::new());
    let copy = |text: String| copied.borrow_mut().push(text);
    present_outputs(&open, &service, &copy, vec![ViewerOutput::Copying(true)]);
    assert!(dispatch_key(&open, "viewer.close"));
    assert!(
        !open.closed.get(),
        "Escape stops the copy and keeps the window"
    );
    assert!(matches!(
        received.borrow().as_slice(),
        [ViewerInput::CancelCopy]
    ));
    // A copy that ends hands its text on once; Escape then closes as usual.
    present_outputs(
        &open,
        &service,
        &copy,
        vec![
            ViewerOutput::Copying(true),
            ViewerOutput::CopyText("the whole file\n".into()),
            ViewerOutput::Copying(false),
        ],
    );
    assert_eq!(copied.borrow().as_slice(), ["the whole file\n"]);
    assert!(dispatch_key(&open, "viewer.close"));
    assert!(open.closed.get());
    assert_eq!(received.borrow().len(), 1, "no second CancelCopy");
    manager.close_all();
}
