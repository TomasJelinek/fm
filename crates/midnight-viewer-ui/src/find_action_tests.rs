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

#[test]
fn find_buttons_and_f3_share_word_options_and_clear_removes_query() {
    let (_runtime, manager, open, _service) = image_presentation_tests::open();
    open.timer.stop();
    apply_mode(&open, 1, false, false);
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let capture = inputs.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        capture.borrow_mut().push(input);
        true
    });
    open.window.set_query("needle".into());
    open.window.set_case_sensitive(true);
    open.window.set_whole_words(true);
    open.window.invoke_search(false);
    assert!(dispatch_key(&open, "viewer.find-previous"));
    for (index, backwards) in [false, true].into_iter().enumerate() {
        assert!(matches!(&inputs.borrow()[index], ViewerInput::Search {
            query, backwards: actual, case_sensitive: true, whole_words: true,
        } if query == "needle" && *actual == backwards));
    }
    open.window.invoke_clear_search();
    assert!(open.window.get_query().is_empty());
    assert!(
        matches!(inputs.borrow().last(), Some(ViewerInput::Search { query, .. }) if query.is_empty())
    );
    for mode in [VIEWER_MODE_HTML, 3] {
        apply_mode(&open, mode, false, false);
        assert!(!open.window.get_whole_words_available());
        assert!(
            open.window.get_whole_words(),
            "unsupported modes retain the text preference"
        );
        open.window.set_query("source bytes".into());
        open.window.invoke_search(false);
        assert!(matches!(
            inputs.borrow().last(),
            Some(ViewerInput::Search {
                whole_words: false,
                ..
            })
        ));
        assert!(dispatch_key(&open, "viewer.find-next"));
        assert!(matches!(
            inputs.borrow().last(),
            Some(ViewerInput::Search {
                whole_words: false,
                ..
            })
        ));
        open.window.invoke_clear_search();
        assert!(
            matches!(inputs.borrow().last(), Some(ViewerInput::Search { query, whole_words: false, .. }) if query.is_empty())
        );
    }
    apply_mode(&open, 2, false, false);
    assert!(open.window.get_whole_words_available());
    manager.close_all();
}

#[test]
fn rendered_find_bar_fits_normal_and_narrow_windows_and_clear_is_clickable() {
    use i_slint_core::{
        accessibility::{AccessibilityAction, AccessibleStringProperty},
        item_tree::ItemRc,
        window::WindowInner,
    };
    use slint::platform::{PointerEventButton, WindowEvent};
    fn control(open: &OpenWindow, label: &str) -> ItemRc {
        fn find(item: ItemRc, label: &str) -> Option<ItemRc> {
            if item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|value| value == label)
            {
                return Some(item);
            }
            let mut child = item.first_child();
            while let Some(item) = child {
                child = item.next_sibling();
                if let Some(found) = find(item, label) {
                    return Some(found);
                }
            }
            None
        }
        let inner = WindowInner::from_pub(open.window.window());
        inner.ensure_tree_instantiated();
        find(ItemRc::new_root(inner.component()), label)
            .unwrap_or_else(|| panic!("missing {label}"))
    }
    fn bounds(item: &ItemRc) -> [f32; 4] {
        let rect = item.geometry();
        let origin = item.map_to_window(rect.origin);
        [
            origin.x,
            origin.y,
            origin.x + rect.width(),
            origin.y + rect.height(),
        ]
    }
    let (_runtime, manager, open, _service) = image_presentation_tests::open();
    open.timer.stop();
    apply_mode(&open, 1, false, false);
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let capture = inputs.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        capture.borrow_mut().push(input);
        true
    });
    for width in [800, 350] {
        open.window
            .window()
            .set_size(slint::LogicalSize::new(width as f32, 480.0));
        open.window.set_query("needle".into());
        open.window.set_whole_words(false);
        open.window.invoke_find();
        let (size, pixels) = image_presentation_tests::bitmap();
        let mut geometry = Vec::new();
        for label in [
            "Find in file",
            "Clear find text",
            "Case sensitive",
            "Find whole words",
            "Previous match",
            "Next match",
        ] {
            let area = bounds(&control(&open, label));
            assert!(
                area[0] >= 0.0
                    && area[1] >= 0.0
                    && area[2] <= width as f32 + 0.5
                    && area[3] <= 480.5,
                "{label} outside {width}px window: {area:?}"
            );
            assert!(area[2] > area[0] && area[3] > area[1], "empty {label}");
            geometry.push(area);
        }
        let row = |bounds: [f32; 4]| (bounds[1] + bounds[3]) / 2.0;
        if width == 800 {
            assert!((row(geometry[0]) - row(geometry[5])).abs() < 2.0);
        } else {
            assert!(
                geometry[2][1] >= geometry[0][3],
                "narrow options overlap query"
            );
            assert!((row(geometry[2]) - row(geometry[5])).abs() < 2.0);
        }
        assert!(
            geometry[1][0] >= geometry[0][2],
            "clear button covers entered text"
        );
        control(&open, "Find whole words").accessible_action(&AccessibilityAction::Default);
        assert!(open.window.get_whole_words());
        control(&open, "Next match").accessible_action(&AccessibilityAction::Default);
        assert!(
            matches!(inputs.borrow().last(), Some(ViewerInput::Search { whole_words: true, query, .. }) if query == "needle")
        );
        let directory = std::env::var_os("MIDNIGHT_UI_RENDER_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../_build/viewer-find-polish")
            });
        std::fs::create_dir_all(&directory).unwrap();
        let file =
            std::fs::File::create(directory.join(format!("viewer-find-{width}.png"))).unwrap();
        let mut encoder = png::Encoder::new(file, size.width, size.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(
                &pixels
                    .into_iter()
                    .flat_map(|pixel| [pixel.r, pixel.g, pixel.b])
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let clear = geometry[1];
        let position =
            slint::LogicalPosition::new((clear[0] + clear[2]) / 2.0, (clear[1] + clear[3]) / 2.0);
        open.window
            .window()
            .dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
        open.window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
        assert!(
            open.window.get_query().is_empty(),
            "clear pointer missed at width {width}"
        );
        assert!(
            matches!(inputs.borrow().last(), Some(ViewerInput::Search { query, .. }) if query.is_empty())
        );
    }
    manager.close_all();
}
