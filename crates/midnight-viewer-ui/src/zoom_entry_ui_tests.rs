//! The typed zoom percentage of the image and PDF toolbar: a click or Ctrl+Y opens it,
//! Enter applies one saved zoom command, Escape keeps the zoom, values outside the range
//! are clamped with a hint and the user's decimal separator reads the value.
use super::image_presentation_tests::{self, match_fixture_viewport};
use super::*;

fn capture(open: &OpenWindow) -> Rc<RefCell<Vec<ViewerInput>>> {
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let received = inputs.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        received.borrow_mut().push(input);
        true
    });
    inputs
}

/// The zoom of each zoom command sent, and its anchor in the view.
fn zooms(inputs: &RefCell<Vec<ViewerInput>>) -> Vec<(ViewerZoomPreference, [f32; 2])> {
    inputs
        .borrow()
        .iter()
        .filter_map(|input| match input {
            ViewerInput::Image {
                input:
                    ViewerImageInput::ViewZoom {
                        zoom,
                        viewport_anchor,
                        ..
                    },
                ..
            } => Some((*zoom, *viewport_anchor)),
            _ => None,
        })
        .collect()
}

fn number_format(decimal: &str, group: &str) -> NumberLocale {
    NumberLocale {
        decimal_separator: decimal.into(),
        group_separator: group.into(),
        grouping: vec![3],
        repeat_last_group: true,
    }
}

#[test]
fn a_typed_percentage_and_enter_apply_one_saved_zoom_at_the_view_centre() {
    let (_runtime, manager, open, _service) = image_presentation_tests::open();
    match_fixture_viewport(&open);
    let inputs = capture(&open);
    open.window.invoke_zoom_edit_begin();
    assert!(open.window.get_zoom_editing());
    assert_eq!(
        open.window.get_zoom_entry(),
        "100",
        "the shown zoom, ready to type over"
    );
    open.window.set_zoom_entry("150%".into());
    open.window.invoke_zoom_entered("150%".into());
    assert!(!open.window.get_zoom_editing());
    assert_eq!(open.window.get_zoom_hint(), "");
    assert_eq!(
        zooms(&inputs),
        [(
            ViewerZoomPreference::ScaleBasisPoints(15_000),
            [200.0, 120.0]
        )]
    );
    assert_eq!(
        open.session.preferences().image_zoom,
        ViewerZoomPreference::ScaleBasisPoints(15_000),
        "the typed zoom is saved like every zoom command"
    );
    // The keyboard opens the same field.
    assert!(dispatch_key(&open, "viewer.zoom-to"));
    assert!(open.window.get_zoom_editing());
    manager.close_all();
}

#[test]
fn escape_or_leaving_the_field_keeps_the_zoom_and_the_viewer_open() {
    let (_runtime, manager, open, _service) = image_presentation_tests::open();
    match_fixture_viewport(&open);
    let inputs = capture(&open);
    open.window.invoke_zoom_edit_begin();
    open.window.set_zoom_entry("300".into());
    // Instantiate and focus the field, then press Escape in it.
    image_presentation_tests::bitmap();
    open.window
        .window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: slint::platform::Key::Escape.into(),
        });
    assert!(!open.window.get_zoom_editing());
    assert!(
        !open.closed.get(),
        "Escape in the field does not close the viewer"
    );
    assert!(zooms(&inputs).is_empty());
    // Leaving the field (focus elsewhere) cancels it the same way.
    open.window.invoke_zoom_edit_begin();
    open.window.invoke_zoom_edit_cancelled();
    assert!(!open.window.get_zoom_editing());
    assert!(zooms(&inputs).is_empty());
    // A second Escape, with the field closed, closes the viewer as before.
    open.window
        .window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: slint::platform::Key::Escape.into(),
        });
    assert!(open.closed.get());
    manager.close_all();
}

#[test]
fn values_outside_the_range_clamp_with_a_hint_and_text_is_never_silently_ignored() {
    let (_runtime, manager, open, _service) = image_presentation_tests::open();
    match_fixture_viewport(&open);
    let inputs = capture(&open);
    open.window.invoke_zoom_edit_begin();
    open.window.invoke_zoom_entered("abc".into());
    assert!(
        open.window.get_zoom_editing(),
        "the field stays open to correct"
    );
    assert_eq!(
        open.window.get_zoom_hint(),
        "Type a zoom percentage from 0.01% to 6400%, for example 150.5"
    );
    assert!(zooms(&inputs).is_empty());
    open.window.invoke_zoom_entered("9000".into());
    assert!(!open.window.get_zoom_editing());
    assert_eq!(open.window.get_zoom_hint(), "The largest zoom is 6400%");
    open.window.invoke_zoom_edit_begin();
    open.window.invoke_zoom_entered("0".into());
    assert_eq!(open.window.get_zoom_hint(), "The smallest zoom is 0.01%");
    assert_eq!(
        zooms(&inputs)
            .into_iter()
            .map(|(zoom, _)| zoom)
            .collect::<Vec<_>>(),
        [
            ViewerZoomPreference::ScaleBasisPoints(640_000),
            ViewerZoomPreference::ScaleBasisPoints(1),
        ]
    );
    manager.close_all();
}

#[test]
fn the_users_decimal_separator_shows_and_reads_the_zoom() {
    let (_runtime, manager, open, _service) = image_presentation_tests::open();
    match_fixture_viewport(&open);
    manager.set_number_locale(Rc::new(|| number_format(",", "\u{a0}")));
    let inputs = capture(&open);
    open.window.set_zoom_percent(206.66);
    open.window.invoke_zoom_edit_begin();
    assert_eq!(open.window.get_zoom_entry(), "206,7");
    open.window.invoke_zoom_entered("62,5 %".into());
    assert_eq!(
        zooms(&inputs)
            .into_iter()
            .map(|(zoom, _)| zoom)
            .collect::<Vec<_>>(),
        [ViewerZoomPreference::ScaleBasisPoints(6_250)]
    );
    open.window.invoke_zoom_edit_begin();
    open.window.invoke_zoom_entered("x".into());
    assert_eq!(
        open.window.get_zoom_hint(),
        "Type a zoom percentage from 0,01% to 6400%, for example 150,5"
    );
    manager.set_number_locale(Rc::new(NumberLocale::default));
    manager.close_all();
}

/// The accessible value of the control labelled `label`: what the footer shows.
fn shown_value(open: &OpenWindow, label: &str) -> String {
    use i_slint_core::{
        accessibility::AccessibleStringProperty, item_tree::ItemRc, window::WindowInner,
    };
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
        .accessible_string_property(AccessibleStringProperty::Value)
        .unwrap_or_default()
        .into()
}

/// The footer shows the zoom in the user's number format, as the typed field does: with a
/// decimal comma "62,5%", never Slint's own "62.5%".
#[test]
fn the_shown_zoom_uses_the_users_decimal_separator() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    match_fixture_viewport(&open);
    manager.set_number_locale(Rc::new(|| number_format(",", "\u{a0}")));
    for (revision, zoom, shown) in [(2, 0.625, "62,5%"), (3, 2.0, "200%"), (4, 0.0133, "1,3%")] {
        let mut zoomed = image_presentation_tests::frame();
        zoomed.revision = revision;
        zoomed.tiles = None;
        zoomed.pixels = vec![255; 400 * 240 * 4].into();
        let info = zoomed.image.as_mut().unwrap();
        info.zoom = zoom;
        info.visible_source = [400.0 / zoom, 240.0 / zoom];
        image_presentation_tests::present(
            &open,
            &service,
            &|_| {},
            vec![ViewerOutput::Frame(Box::new(zoomed))],
        );
        assert_eq!(shown_value(&open, "Image zoom percentage"), shown);
        // The field opens with the same number, ready to type over.
        open.window.invoke_zoom_edit_begin();
        assert_eq!(format!("{}%", open.window.get_zoom_entry()), shown);
        open.window.invoke_zoom_edit_cancelled();
    }
    // HTML shows whole percentages through the same format.
    let ViewerOutput::Frame(mut page) = text_presentation_tests::frame(5, 0, 0.2) else {
        unreachable!()
    };
    page.mode = VIEWER_MODE_HTML;
    page.modes = vec![VIEWER_MODE_HTML];
    image_presentation_tests::present(
        &open,
        &service,
        &|_| {},
        vec![
            ViewerOutput::Frame(page),
            ViewerOutput::WebState {
                generation: 7,
                policy: viewer_web::Policy::default(),
                policy_change_allowed: true,
                zoom: 1.24,
            },
        ],
    );
    assert_eq!(shown_value(&open, "HTML zoom percentage"), "124%");
    manager.set_number_locale(Rc::new(NumberLocale::default));
    manager.close_all();
}
