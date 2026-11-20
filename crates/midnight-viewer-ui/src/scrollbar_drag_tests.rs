//! A scrollbar drag over a PDF lands where the thumb is released, also when the frames
//! the backend paints for earlier drag steps arrive during the drag. In the Continuous
//! layout each of those frames can carry a new surface: the backend prepares a new one
//! for every page the view reaches.
use super::image_presentation_tests::{self, present as present_outputs};
use super::*;
use slint::platform::{PointerEventButton, WindowEvent};
use std::sync::Arc;

const PAGES: u32 = 24;

/// A document raster as the backend publishes it for a scroll position.
fn document(revision: u64, surface: u64, position: f64, layout: DocumentLayout) -> ViewerOutput {
    let page = 1 + (position * f64::from(PAGES - 1)).round() as u32;
    ViewerOutput::Frame(Box::new(ViewerFrame {
        generation: 2,
        raster: None,
        raster_rebase: None,
        media_control: None,
        position_sequence: [0; 2],
        revision,
        mode: VIEWER_MODE_IMAGE,
        modes: vec![VIEWER_MODE_IMAGE],
        wrap_enabled: false,
        wrap_available: false,
        image: Some(ViewerImageInfo {
            surface: ViewerSurfaceId(surface),
            origin: [0.0; 2],
            visible_source: [400.0, 240.0],
            raster_scale: 1.0,
            pan_sequence: 0,
            width: 400,
            height: 520,
            zoom: 1.0,
            fitted: false,
            page,
            page_count: PAGES,
            document_layout: Some(layout),
        }),
        width: 400,
        height: 240,
        pixels: Arc::from(vec![255; 400 * 240 * 4]),
        tiles: None,
        accessible_text: "Document".into(),
        position,
        vertical_extent: 0.04,
        horizontal_position: 0.0,
        horizontal_extent: 1.0,
        caption: "PDF".into(),
        status: format!("Page {page} / {PAGES}"),
    }))
}

/// The middle of the vertical thumb when it shows `position` (`viewer-position-bar.slint`).
fn thumb(window: &ViewerWindow, position: f32) -> slint::LogicalPosition {
    let height = window.get_vertical_bar_height();
    let length = height.min((height * window.get_vertical_extent().clamp(0.0, 1.0)).max(28.0));
    slint::LogicalPosition::new(
        window.get_vertical_bar_left() + window.get_vertical_bar_width() / 2.0,
        window.get_vertical_bar_top() + position * (height - length) + length / 2.0,
    )
}

/// Press the thumb, move it in four steps while the backend answers each, release it.
fn drag_lands(layout: DocumentLayout, from: f32, to: f32) {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let seeks = Rc::new(RefCell::new(Vec::new()));
    let capture = seeks.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        if let ViewerInput::Seek(position) = input {
            capture.borrow_mut().push(position);
        }
        true
    });
    let mut revision = 2;
    let mut surface = 30;
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![document(revision, surface, f64::from(from), layout)],
    );
    assert!(open.window.get_image_mode() && !open.window.get_tiled());
    let window = open.window.window();
    let start = thumb(&open.window, from);
    window.dispatch_event(WindowEvent::PointerMoved { position: start });
    window.dispatch_event(WindowEvent::PointerPressed {
        position: start,
        button: PointerEventButton::Left,
    });
    for step in 1..=4 {
        let target = from + (to - from) * step as f32 / 4.0;
        window.dispatch_event(WindowEvent::PointerMoved {
            position: thumb(&open.window, target),
        });
        // The backend paints this step before the next one arrives. A Continuous
        // document's view reaches another page, which has another surface.
        let shown = *seeks.borrow().last().expect("the drag seeks");
        revision += 1;
        if layout == DocumentLayout::Continuous {
            surface += 1;
        }
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![document(revision, surface, shown, layout)],
        );
    }
    window.dispatch_event(WindowEvent::PointerReleased {
        position: thumb(&open.window, to),
        button: PointerEventButton::Left,
    });
    let last = *seeks.borrow().last().unwrap();
    assert!(
        (last - f64::from(to)).abs() < 0.002,
        "{layout:?}: the drag's last seek was {last}, released at {to}: {:?}",
        seeks.borrow()
    );
    revision += 1;
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![document(revision, surface + 1, last, layout)],
    );
    let window = &open.window;
    for (what, value) in [
        ("thumb", window.get_position()),
        ("content", window.get_displayed_position()),
        ("drawn thumb", window.get_vertical_thumb_position()),
    ] {
        assert!(
            (value - to).abs() < 0.005,
            "{layout:?}: the {what} shows {value}, released at {to}"
        );
    }
    manager.close_all();
}

#[test]
fn a_continuous_document_drag_across_pages_lands_where_the_thumb_is_released() {
    drag_lands(DocumentLayout::Continuous, 0.28, 0.32);
}

#[test]
fn a_continuous_document_drag_back_across_pages_lands_where_the_thumb_is_released() {
    drag_lands(DocumentLayout::Continuous, 0.98, 0.94);
}

#[test]
fn a_paged_document_drag_lands_where_the_thumb_is_released() {
    drag_lands(DocumentLayout::Paged, 0.28, 0.32);
}
