//! Tests authored before the continuous-document implementation; execution awaits a build lease.
use super::*;
use midnight_core::{DocumentLayout, ViewerImageInput, ViewerZoomPreference};
use midnight_image_viewer_host::Metadata;

fn pages() -> [Metadata; 3] {
    [
        Metadata {
            width: 400,
            height: 600,
        },
        Metadata {
            width: 1000,
            height: 400,
        },
        Metadata {
            width: 600,
            height: 800,
        },
    ]
}

fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 0.00001,
        "{actual} != {expected}"
    );
}

#[test]
fn continuous_mixed_pages_are_centered_and_scroll_over_both_boundaries() {
    let mut view = DocumentView::new(&pages(), [400, 400]).unwrap();
    assert_eq!(view.layout, DocumentLayout::Paged);
    assert!(view.geometry.fitted);
    view.painted(7);
    assert!(view.set_layout(7, DocumentLayout::Continuous));
    assert_eq!(view.placements[0].origin, [300.0, 0.0]);
    assert_eq!(view.placements[1].origin, [0.0, 624.0]);
    assert_eq!(view.placements[2].origin, [200.0, 1048.0]);
    assert_eq!(view.canvas, [1000.0, 1848.0]);
    let scale = view.geometry.scale;
    view.seek(1, 0.5);
    assert_eq!(view.page, 1);
    assert_eq!((view.geometry.width, view.geometry.height), (1000, 400));
    assert_eq!(
        view.geometry.scale, scale,
        "ordinary scrolling must not refit"
    );
    view.seek(1, 1.0);
    assert_eq!(view.page, 2);
    assert_eq!(view.metrics(1).0, 1.0);
    view.scroll([0.0, 1e9]);
    assert_eq!(view.metrics(1).0, 1.0);
    view.seek(1, 0.0);
    assert_eq!(view.page, 0);
    assert_eq!(view.metrics(1).0, 0.0);
}

#[test]
fn layout_switch_anchors_the_published_page_instead_of_the_unpublished_camera() {
    let mut view = DocumentView::new(&pages(), [200, 120]).unwrap();
    view.geometry.actual();
    view.geometry.pan_to([80.0, 100.0]);
    view.painted(11);
    view.scroll([80.0, 200.0]);
    assert!(view.set_layout(11, DocumentLayout::Continuous));
    assert_eq!(view.geometry.scale, 1.0);
    near(view.geometry.origin[0], 380.0);
    near(view.geometry.origin[1], 100.0);
    assert_eq!(view.page, 0);
    assert!(!view.set_layout(10, DocumentLayout::Paged));
    view.painted(12);
    assert!(!view.set_layout(11, DocumentLayout::Paged));
    assert!(view.set_layout(12, DocumentLayout::Paged));
    near(view.geometry.origin[0], 80.0);
    near(view.geometry.origin[1], 100.0);
    assert_eq!(view.geometry.scale, 1.0);
}

#[test]
fn zoom_and_resize_preserve_published_local_center_with_manual_scale_precision() {
    let mut view = DocumentView::new(&pages(), [200, 120]).unwrap();
    view.painted(1);
    assert!(view.set_layout(1, DocumentLayout::Continuous));
    view.geometry.scale = 1.234567;
    view.geometry.fitted = false;
    view.pan_to([350.0, 200.0]);
    view.painted(2);
    let source = [400.0, 240.0];
    assert!(view.image_input(&ViewerImageInput::ViewZoom {
        zoom: ViewerZoomPreference::ScaleBasisPoints(20000),
        source_anchor: source,
        viewport_anchor: [50.0, 40.0],
    }));
    near(view.geometry.origin[0], 375.0);
    near(view.geometry.origin[1], 220.0);
    view.geometry.scale = 1.234567;
    view.pan_to([350.0, 200.0]);
    view.painted(3);
    view.scroll([100.0, 100.0]);
    view.resize([300, 200]);
    assert_eq!(view.geometry.scale, 1.234567);
    near(
        view.geometry.origin[0] + 150.0 / 1.234567,
        350.0 + 100.0 / 1.234567,
    );
    near(
        view.geometry.origin[1] + 100.0 / 1.234567,
        200.0 + 60.0 / 1.234567,
    );
}

#[test]
fn continuous_fit_uses_the_anchor_page_until_an_explicit_refit_or_page_jump() {
    let mut view = DocumentView::new(&pages(), [160, 120]).unwrap();
    view.painted(1);
    view.set_layout(1, DocumentLayout::Continuous);
    near(view.geometry.scale, 0.2);
    view.seek(1, 0.5);
    assert_eq!(view.page, 1);
    near(view.geometry.scale, 0.2);
    view.painted(2);
    view.image_input(&ViewerImageInput::Fit);
    near(view.geometry.scale, 0.16);
    assert!(view.geometry.fitted);
    view.seek(1, 1.0);
    near(view.geometry.scale, 0.16);
    view.painted(3);
    view.resize([320, 240]);
    near(view.geometry.scale, 0.3);
}

#[test]
fn gap_center_selects_the_nearest_page_and_continuous_page_controls_keep_manual_scale() {
    let mut view = DocumentView::new(&pages(), [200, 120]).unwrap();
    view.geometry.actual();
    view.painted(1);
    view.set_layout(1, DocumentLayout::Continuous);
    view.pan_to([350.0, 550.0]); // Center y=610: ten below page one, fourteen above page two.
    assert_eq!(view.page, 0);
    view.pan_to([350.0, 554.0]);
    assert_eq!(view.page, 1);
    view.geometry.scale = 1.234567;
    assert!(view.change_page(1));
    assert_eq!(view.page, 2);
    assert_eq!(view.geometry.scale, 1.234567);
    assert!(!view.geometry.fitted);
    assert!(view.change_page(-1));
    assert_eq!(view.page, 1);
    assert_eq!(view.geometry.scale, 1.234567);
}

#[test]
fn paged_navigation_preserves_manual_zoom_and_adapts_fit_to_the_next_page() {
    let mut view = DocumentView::new(&pages(), [160, 120]).unwrap();
    view.geometry.zoom(6.172835, view.geometry.center());
    let scale = view.geometry.scale;
    assert!(view.change_page(1));
    assert_eq!(view.geometry.scale, scale);
    assert!(!view.geometry.fitted);
    assert_eq!((view.geometry.width, view.geometry.height), (1000, 400));
    assert_eq!(view.geometry.origin, [0.0, 0.0]);
    view.image_input(&ViewerImageInput::Fit);
    near(view.geometry.scale, 0.16);
    assert!(view.change_page(-1));
    near(view.geometry.scale, 0.2);
    assert!(!view.change_page(-1));
    assert!(view.change_page(i32::MAX));
    assert_eq!(view.page, 2);
    assert!(!view.change_page(1));
}

#[test]
fn refit_before_page_turn_publication_keeps_the_explicit_target() {
    for layout in [DocumentLayout::Paged, DocumentLayout::Continuous] {
        let mut view = DocumentView::new(&pages(), [160, 120]).unwrap();
        view.painted(1);
        if layout == DocumentLayout::Continuous {
            view.set_layout(1, layout);
            view.painted(2);
        }
        assert!(view.change_page(1));
        assert!(view.image_input(&ViewerImageInput::Fit));
        assert_eq!(view.page, 1);
        near(view.geometry.scale, 0.16);
        view.resize([320, 240]);
        assert_eq!(view.page, 1);
        near(view.geometry.scale, 0.32);
    }
}

#[test]
fn page_turn_publication_restores_published_resize_anchor_policy() {
    let mut view = DocumentView::new(&pages(), [160, 120]).unwrap();
    view.geometry.actual();
    view.painted(1);
    view.set_layout(1, DocumentLayout::Continuous);
    view.painted(2);
    assert!(view.change_page(1));
    view.painted(3);
    view.scroll([0.0, 100.0]);
    view.resize([320, 240]);
    near(view.geometry.origin[0] + 160.0, 500.0);
    near(view.geometry.origin[1] + 120.0, 824.0);
}

#[test]
fn tall_document_coordinates_do_not_truncate_and_nonfinite_inputs_are_ignored() {
    let pages = vec![
        Metadata {
            width: 1000,
            height: 32768
        };
        4096
    ];
    let mut view = DocumentView::new(&pages, [400, 300]).unwrap();
    view.painted(1);
    view.set_layout(1, DocumentLayout::Continuous);
    assert!(view.canvas[1] > f64::from(u16::MAX));
    view.seek(1, 1.0);
    assert_eq!(view.page, 4095);
    assert_eq!(view.geometry.height, 32768);
    assert_eq!(view.metrics(1).0, 1.0);
    let before = view.geometry.clone();
    view.pan_to([f64::INFINITY, 0.0]);
    view.seek(1, f64::NAN);
    view.scroll([f64::NAN, f64::NEG_INFINITY]);
    assert_eq!(view.geometry, before);
    assert!(DocumentView::new(&[], [400, 300]).is_err());
    assert!(DocumentView::new(&vec![pages[0]; 4097], [400, 300]).is_err());
}
