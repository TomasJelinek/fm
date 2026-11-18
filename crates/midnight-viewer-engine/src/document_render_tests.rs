//! Tests authored before the composition implementation; execution awaits a build lease.
use super::*;
use crate::document_view::DocumentView;
use midnight_core::DocumentLayout;
use midnight_image_viewer_host::{Frame, Metadata, Viewport};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

fn boundary() -> DocumentView {
    let mut view = DocumentView::new(
        &[
            Metadata {
                width: 400,
                height: 600,
            },
            Metadata {
                width: 1000,
                height: 400,
            },
        ],
        [400, 200],
    )
    .unwrap();
    view.geometry.actual();
    view.painted(1);
    view.set_layout(1, DocumentLayout::Continuous);
    view.pan_to([300.0, 550.0]);
    view
}

struct Renderer<'a> {
    wanted: &'a AtomicU64,
    hard_cancel: &'a AtomicBool,
    supersede: bool,
    cancel: bool,
    reject_source: bool,
    supersede_after_validation: bool,
    malformed: bool,
    requests: Vec<(usize, Viewport, Duration)>,
    validations: usize,
}

impl PageRenderer for Renderer<'_> {
    fn render_page(
        &mut self,
        page: usize,
        viewport: Viewport,
        hard_cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Frame, String> {
        assert!(
            !hard_cancel.load(Ordering::Acquire),
            "supersession must not cancel a session"
        );
        self.requests.push((page, viewport, timeout));
        if self.supersede {
            self.wanted.store(2, Ordering::Release);
            self.supersede = false;
        }
        if self.cancel {
            self.hard_cancel.store(true, Ordering::Release);
        }
        let color = if page == 0 {
            [255, 0, 0, 255]
        } else {
            [0, 0, 255, 255]
        };
        Ok(Frame {
            width: viewport.width + u32::from(self.malformed),
            height: viewport.height,
            rgba: color.repeat(viewport.width as usize * viewport.height as usize),
        })
    }

    fn validate(&mut self, _: &AtomicBool) -> Result<(), String> {
        self.validations += 1;
        if self.supersede_after_validation {
            self.wanted.store(2, Ordering::Release);
        }
        if self.reject_source {
            Err("source changed".into())
        } else {
            Ok(())
        }
    }
}

fn renderer<'a>(wanted: &'a AtomicU64, hard_cancel: &'a AtomicBool) -> Renderer<'a> {
    Renderer {
        wanted,
        hard_cancel,
        supersede: false,
        cancel: false,
        reject_source: false,
        supersede_after_validation: false,
        malformed: false,
        requests: Vec::new(),
        validations: 0,
    }
}

#[test]
fn composition_renders_both_visible_page_crops_and_leaves_the_gap_transparent() {
    let plan = DocumentRenderPlan::new(&boundary()).unwrap();
    let wanted = AtomicU64::new(1);
    let cancel = AtomicBool::new(false);
    let mut renderer = renderer(&wanted, &cancel);
    let Composition::Frame(frame) = compose(
        &mut renderer,
        &plan,
        1,
        &wanted,
        &cancel,
        Duration::from_secs(15),
    )
    .unwrap() else {
        panic!("current demand must publish")
    };
    assert_eq!((frame.width, frame.height), (400, 200));
    assert_eq!(renderer.requests.len(), 2);
    assert_eq!(renderer.requests[0].0, 0);
    assert_eq!(
        renderer.requests[0].1,
        Viewport {
            width: 400,
            height: 50,
            scale: 1.0,
            origin_x: 0.0,
            origin_y: 550.0
        }
    );
    assert_eq!(renderer.requests[1].0, 1);
    assert_eq!(
        renderer.requests[1].1,
        Viewport {
            width: 400,
            height: 126,
            scale: 1.0,
            origin_x: 300.0,
            origin_y: 0.0
        }
    );
    assert!(
        frame.rgba[..400 * 50 * 4]
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [255, 0, 0, 255])
    );
    assert!(
        frame.rgba[400 * 50 * 4..400 * 74 * 4]
            .iter()
            .all(|byte| *byte == 0)
    );
    assert!(
        frame.rgba[400 * 74 * 4..]
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [0, 0, 255, 255])
    );
    assert_eq!(renderer.validations, 1);
    assert!(renderer.requests[1].2 < renderer.requests[0].2);
}

#[test]
fn demand_replacement_abandons_between_pages_and_reuses_the_same_renderer() {
    let plan = DocumentRenderPlan::new(&boundary()).unwrap();
    let wanted = AtomicU64::new(1);
    let cancel = AtomicBool::new(false);
    let mut renderer = renderer(&wanted, &cancel);
    renderer.supersede = true;
    assert!(matches!(
        compose(
            &mut renderer,
            &plan,
            1,
            &wanted,
            &cancel,
            Duration::from_secs(15)
        )
        .unwrap(),
        Composition::Superseded
    ));
    assert_eq!(
        renderer.requests.len(),
        1,
        "obsolete demand must not render the second page"
    );
    assert_eq!(renderer.validations, 0);
    assert!(!cancel.load(Ordering::Acquire));
    assert!(matches!(
        compose(
            &mut renderer,
            &plan,
            2,
            &wanted,
            &cancel,
            Duration::from_secs(15)
        )
        .unwrap(),
        Composition::Frame(_)
    ));
    assert_eq!(
        renderer.requests.len(),
        3,
        "one existing renderer serves the latest complete demand"
    );
    assert_eq!(renderer.validations, 1);
}

#[test]
fn close_stops_between_pages_and_source_change_never_publishes_partial_pixels() {
    let plan = DocumentRenderPlan::new(&boundary()).unwrap();
    let wanted = AtomicU64::new(1);
    let cancel = AtomicBool::new(false);
    let mut renderer = renderer(&wanted, &cancel);
    renderer.cancel = true;
    assert!(
        compose(
            &mut renderer,
            &plan,
            1,
            &wanted,
            &cancel,
            Duration::from_secs(15)
        )
        .is_err()
    );
    assert_eq!(renderer.requests.len(), 1);
    assert_eq!(renderer.validations, 0);
    cancel.store(false, Ordering::Release);
    renderer.cancel = false;
    renderer.reject_source = true;
    assert!(
        compose(
            &mut renderer,
            &plan,
            1,
            &wanted,
            &cancel,
            Duration::from_secs(15)
        )
        .unwrap_err()
        .contains("source changed")
    );
    assert_eq!(renderer.validations, 1);
}

#[test]
fn replacement_during_final_validation_and_malformed_page_replies_never_publish() {
    let plan = DocumentRenderPlan::new(&boundary()).unwrap();
    let wanted = AtomicU64::new(1);
    let cancel = AtomicBool::new(false);
    let mut renderer = renderer(&wanted, &cancel);
    renderer.supersede_after_validation = true;
    assert!(matches!(
        compose(
            &mut renderer,
            &plan,
            1,
            &wanted,
            &cancel,
            Duration::from_secs(15)
        )
        .unwrap(),
        Composition::Superseded
    ));
    assert_eq!(renderer.requests.len(), 2);
    assert_eq!(renderer.validations, 1);
    renderer.supersede_after_validation = false;
    renderer.malformed = true;
    assert!(
        compose(
            &mut renderer,
            &plan,
            2,
            &wanted,
            &cancel,
            Duration::from_secs(15)
        )
        .is_err()
    );
    assert_eq!(renderer.requests.len(), 3);
    assert_eq!(renderer.validations, 1);
}

#[test]
fn large_dpi_viewports_bound_output_and_every_scratch_surface() {
    let mut view = boundary();
    view.resize([7680, 4320]);
    let plan = DocumentRenderPlan::new(&view).unwrap();
    assert!(plan.size[0] <= 4096 && plan.size[1] <= 4096);
    assert!(u64::from(plan.size[0]) * u64::from(plan.size[1]) <= 4_194_304);
    for slice in &plan.slices {
        assert!(slice.viewport.width <= 4096 && slice.viewport.height <= 4096);
        assert!(u64::from(slice.viewport.width) * u64::from(slice.viewport.height) <= 4_194_304);
        slice.viewport.validate().unwrap();
    }
}

#[test]
fn tiny_pages_are_never_silently_dropped_and_expired_work_has_no_render_calls() {
    let pages = vec![
        Metadata {
            width: 1,
            height: 1
        };
        4096
    ];
    let mut view = DocumentView::new(&pages, [1000, 1000]).unwrap();
    view.painted(1);
    view.set_layout(1, DocumentLayout::Continuous);
    view.geometry.scale = 1.0 / 32768.0;
    view.pan_to([0.0, 0.0]);
    let plan = DocumentRenderPlan::new(&view).unwrap();
    assert_eq!(plan.slices.len(), 4096);
    let wanted = AtomicU64::new(1);
    let cancel = AtomicBool::new(false);
    let mut renderer = renderer(&wanted, &cancel);
    assert!(compose(&mut renderer, &plan, 1, &wanted, &cancel, Duration::ZERO).is_err());
    assert!(renderer.requests.is_empty());
}
