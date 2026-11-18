//! Visible Find uses the existing guarded command and browser focus handoff.
use super::*;

fn assert_focus_handoff(inputs: &[ViewerInput], web: bool, frame: u64) {
    assert_eq!(inputs.len(), usize::from(web));
    if web {
        assert!(matches!(
            inputs.first(),
            Some(ViewerInput::Web {
                frame: actual,
                input: viewer_web::CommandKind::Focus(false),
            }) if *actual == frame
        ));
    }
}

#[test]
fn bound_find_action_reuses_dispatch_and_retains_query_on_supported_routes() {
    let (_runtime, manager, open, _service) = image_presentation_tests::open();
    open.timer.stop();
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let capture = inputs.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        capture.borrow_mut().push(input);
        true
    });
    *open.key_resolver.borrow_mut() =
        Rc::new(|_, _| panic!("the Find action must not synthesize a keyboard shortcut"));
    for mode in [1, 3, 2, 4, VIEWER_MODE_HTML] {
        apply_mode(&open, mode, false, false);
        let web = mode == VIEWER_MODE_HTML;
        open.window.set_query("retained words".into());
        open.window.set_search_visible(false);
        for _ in 0..2 {
            open.window.invoke_focus_content();
            inputs.borrow_mut().clear();
            open.window.invoke_find();
            assert!(open.window.get_search_visible(), "mode {mode}");
            assert!(!open.window.get_content_focused(), "mode {mode}");
            assert_eq!(open.window.get_query(), "retained words");
            assert_focus_handoff(&inputs.borrow(), web, open.frame.get());
        }
        open.window.set_search_visible(false);
        open.window.invoke_focus_content();
        inputs.borrow_mut().clear();
        assert!(dispatch_key(&open, "viewer.find"));
        assert!(open.window.get_search_visible());
        assert!(!open.window.get_content_focused());
        assert_eq!(open.window.get_query(), "retained words");
        assert_focus_handoff(&inputs.borrow(), web, open.frame.get());
    }
    for layout in [-1, 0, 1] {
        apply_mode(&open, VIEWER_MODE_IMAGE, true, false);
        open.window.set_document_layout(layout);
        open.window
            .set_document_page(if layout < 0 { 0 } else { 2 });
        open.window.set_search_visible(false);
        open.window.invoke_focus_content();
        inputs.borrow_mut().clear();
        open.window.invoke_find();
        assert!(!open.window.get_search_visible());
        assert!(open.window.get_content_focused());
        assert_eq!(open.window.get_query(), "retained words");
        assert!(!dispatch_key(&open, "viewer.find"));
        assert!(inputs.borrow().is_empty());
    }
    manager.close_all();
}
