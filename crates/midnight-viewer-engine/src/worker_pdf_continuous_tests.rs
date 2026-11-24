//! PDF input/publication regressions using the shared Worker fixture.
use super::*;
use midnight_core::{DocumentLayout, ViewerFrame, ViewerPixels};
use midnight_image_viewer_host::Metadata;

fn document_worker(
    viewport: [u32; 2],
    layout: DocumentLayout,
    manual_scale: Option<f64>,
) -> (Worker, Mailbox, ViewerFrame) {
    let (mut worker, output) = worker();
    worker.image_size = viewport;
    worker.geometry = ViewerGeometry {
        width: viewport[0],
        height: viewport[1],
        scale: 1.0,
    };
    worker.image = Some(crate::image_view::ImageView::new(
        crate::image_worker::ImageWorker::opened_for_test(
            vec![
                Metadata {
                    width: 400,
                    height: 600,
                },
                Metadata {
                    width: 1000,
                    height: 400,
                },
            ],
            true,
        ),
        worker.image_ids.clone(),
    ));
    worker.poll_image(&output);
    let mut displayed = publish_document(&mut worker, &output);
    if let Some(scale) = manual_scale {
        worker.input(
            ViewerInput::Image {
                frame: displayed.revision,
                input: ViewerImageInput::ActualPixels,
            },
            &output,
        );
        displayed = publish_document(&mut worker, &output);
        if scale != 1.0 {
            worker.input(
                ViewerInput::Image {
                    frame: displayed.revision,
                    input: ViewerImageInput::Zoom {
                        factor: scale,
                        anchor: None,
                    },
                },
                &output,
            );
            displayed = publish_document(&mut worker, &output);
        }
    }
    if layout != DocumentLayout::Paged {
        worker.input(
            ViewerInput::DocumentLayout {
                frame: displayed.revision,
                layout,
            },
            &output,
        );
        displayed = publish_document(&mut worker, &output);
    }
    (worker, output, displayed)
}

pub(super) fn publish_document(worker: &mut Worker, output: &Mailbox) -> ViewerFrame {
    try_publish_document(worker, output)
        .expect("document input must be followed by its replacement publication")
}

/// As [`publish_document`]; `None` when the camera shown last is still the latest.
pub(super) fn try_publish_document(worker: &mut Worker, output: &Mailbox) -> Option<ViewerFrame> {
    // Substitute only provider pixels: every band of the current camera's tiles. The
    // production publication path records both Worker/ImageFrames acceptance and the
    // document's published-camera snapshot.
    output.take();
    let image = worker.image.as_mut().unwrap();
    while let Some(band) = image.scene.next_band().unwrap() {
        let tiles = band
            .tiles
            .into_iter()
            .map(|(key, charge)| {
                let pixels = ViewerPixels::new(vec![255; 256 * 256 * 4], charge).unwrap();
                (key, Arc::new(pixels))
            })
            .collect();
        image.scene.complete_band(tiles).unwrap();
    }
    image.pending = None;
    image.validated = Some(image.revision);
    worker.paint_image(output).unwrap();
    output.take().into_iter().find_map(|event| match event {
        ViewerOutput::Frame(frame) => Some(*frame),
        _ => None,
    })
}

fn page_turn_then_resize(worker: &mut Worker, output: &Mailbox, displayed: &ViewerFrame) {
    worker.input(
        ViewerInput::DocumentPage {
            frame: displayed.revision,
            delta: 1,
        },
        output,
    );
    assert_eq!(
        worker.image.as_ref().unwrap().page,
        1,
        "page turn must reach page two"
    );
    worker.input(
        ViewerInput::Resize(ViewerGeometry {
            width: 320,
            height: 240,
            scale: 1.0,
        }),
        output,
    );
}

fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 0.00001,
        "{actual} != {expected}"
    );
}

#[test]
fn page_turn_then_resize_before_paint_keeps_paged_fit_target_and_new_frame() {
    let (mut worker, output, displayed) = document_worker([160, 120], DocumentLayout::Paged, None);
    page_turn_then_resize(&mut worker, &output, &displayed);
    let replacement = publish_document(&mut worker, &output);
    let image = replacement.image.as_ref().unwrap();
    assert_eq!(image.page, 2);
    assert_eq!(image.document_layout, Some(DocumentLayout::Paged));
    assert!(image.fitted);
    near(image.zoom, 0.32);
    assert_eq!(image.origin, [0.0, -175.0]);
    assert!(replacement.revision > displayed.revision);
}

#[test]
fn page_turn_then_resize_before_paint_keeps_paged_manual_target_and_anchor() {
    let (mut worker, output, displayed) =
        document_worker([160, 120], DocumentLayout::Paged, Some(1.234567));
    page_turn_then_resize(&mut worker, &output, &displayed);
    let replacement = publish_document(&mut worker, &output);
    let image = replacement.image.as_ref().unwrap();
    assert_eq!(image.page, 2);
    assert!(!image.fitted);
    assert_eq!(image.zoom, 1.234567);
    assert_eq!(
        image.origin,
        [0.0, 0.0],
        "new-page leading edges clamp the larger viewport"
    );
}

#[test]
fn page_turn_then_resize_before_paint_keeps_reachable_continuous_fit_and_manual_targets() {
    for manual in [None, Some(1.234567)] {
        let (mut worker, output, displayed) =
            document_worker([160, 120], DocumentLayout::Continuous, manual);
        page_turn_then_resize(&mut worker, &output, &displayed);
        let replacement = publish_document(&mut worker, &output);
        let image = replacement.image.as_ref().unwrap();
        assert_eq!(image.page, 2);
        assert_eq!(image.document_layout, Some(DocumentLayout::Continuous));
        match manual {
            Some(scale) => {
                assert!(!image.fitted);
                assert_eq!(image.zoom, scale);
                near(image.origin[0] + 160.0 / scale, 500.0);
                near(image.origin[1] + 120.0 / scale, 824.0);
            }
            None => {
                assert!(image.fitted);
                near(image.zoom, 0.32);
                near(image.origin[0], 0.0);
                near(image.origin[1], 274.0);
            }
        }
    }
}

#[test]
fn clamped_continuous_page_turn_keeps_current_frame_fit_zoom_and_layout_inputs() {
    for operation in ["fit", "zoom", "layout"] {
        let (mut worker, output, displayed) =
            document_worker([800, 1200], DocumentLayout::Continuous, Some(1.0));
        let image = displayed.image.as_ref().unwrap();
        assert_eq!(image.page, 1);
        assert_eq!(image.origin, [100.0, -88.0]);
        assert_eq!(image.zoom, 1.0);
        let before = worker.image.as_ref().unwrap().revision;
        worker.input(
            ViewerInput::DocumentPage {
                frame: displayed.revision,
                delta: 1,
            },
            &output,
        );
        assert_eq!(worker.image.as_ref().unwrap().revision, before);
        // No replacement frame is supplied between the clamped NextPage and this gesture.
        let input = match operation {
            "fit" => ViewerInput::Image {
                frame: displayed.revision,
                input: ViewerImageInput::Fit,
            },
            "zoom" => ViewerInput::Image {
                frame: displayed.revision,
                input: ViewerImageInput::ViewZoom {
                    zoom: ViewerZoomPreference::ScaleBasisPoints(20000),
                    source_anchor: [500.0, 512.0],
                    viewport_anchor: [400.0, 600.0],
                },
            },
            _ => ViewerInput::DocumentLayout {
                frame: displayed.revision,
                layout: DocumentLayout::Paged,
            },
        };
        worker.input(input, &output);
        let document = worker
            .image
            .as_ref()
            .unwrap()
            .document_view
            .as_ref()
            .unwrap();
        if operation == "layout" {
            assert_eq!(document.layout, DocumentLayout::Paged);
        } else {
            assert_eq!(
                document.geometry.scale, 2.0,
                "current-frame {operation} was rejected"
            );
            assert_eq!(document.geometry.fitted, operation == "fit");
        }
        assert!(worker.image.as_ref().unwrap().revision > before);
        let replacement = publish_document(&mut worker, &output);
        assert!(replacement.revision > displayed.revision);
    }
}

#[test]
fn real_page_turn_fences_old_coordinates_until_replacement_is_published() {
    let (mut worker, output, displayed) =
        document_worker([160, 120], DocumentLayout::Paged, Some(1.234567));
    worker.input(
        ViewerInput::DocumentPage {
            frame: displayed.revision,
            delta: 1,
        },
        &output,
    );
    worker.input(
        ViewerInput::Image {
            frame: displayed.revision,
            input: ViewerImageInput::ViewZoom {
                zoom: ViewerZoomPreference::ScaleBasisPoints(20000),
                source_anchor: [200.0, 300.0],
                viewport_anchor: [80.0, 60.0],
            },
        },
        &output,
    );
    worker.input(
        ViewerInput::DocumentLayout {
            frame: displayed.revision,
            layout: DocumentLayout::Continuous,
        },
        &output,
    );
    let image = worker.image.as_ref().unwrap();
    assert_eq!(image.page, 1);
    assert_eq!(image.geometry.as_ref().unwrap().scale, 1.234567);
    assert_eq!(
        image.document_view.as_ref().unwrap().layout,
        DocumentLayout::Paged
    );
    let replacement = publish_document(&mut worker, &output);
    worker.input(
        ViewerInput::Image {
            frame: replacement.revision,
            input: ViewerImageInput::ViewZoom {
                zoom: ViewerZoomPreference::ScaleBasisPoints(20000),
                source_anchor: [100.0, 100.0],
                viewport_anchor: [80.0, 60.0],
            },
        },
        &output,
    );
    assert_eq!(
        worker
            .image
            .as_ref()
            .unwrap()
            .geometry
            .as_ref()
            .unwrap()
            .scale,
        2.0
    );
}

#[test]
fn identical_document_resize_keeps_current_frame_layout_and_anchored_zoom() {
    for (layout, dpi_scale) in [
        (DocumentLayout::Paged, 1.0),
        (DocumentLayout::Continuous, 1.0),
        (DocumentLayout::Paged, 2.0),
        (DocumentLayout::Continuous, 2.0),
    ] {
        for change_layout in [false, true] {
            let (mut worker, output, displayed) = document_worker([160, 120], layout, Some(1.0));
            let before = worker.image.as_ref().unwrap().revision;
            worker.input(
                ViewerInput::Resize(ViewerGeometry {
                    width: 160,
                    height: 120,
                    scale: dpi_scale,
                }),
                &output,
            );
            assert_eq!(worker.geometry.scale, dpi_scale);
            assert_eq!(worker.image.as_ref().unwrap().revision, before);
            let target_layout = if layout == DocumentLayout::Paged {
                DocumentLayout::Continuous
            } else {
                DocumentLayout::Paged
            };
            let image = displayed.image.as_ref().unwrap();
            let input = if change_layout {
                ViewerInput::DocumentLayout {
                    frame: displayed.revision,
                    layout: target_layout,
                }
            } else {
                ViewerInput::Image {
                    frame: displayed.revision,
                    input: ViewerImageInput::ViewZoom {
                        zoom: ViewerZoomPreference::ScaleBasisPoints(20000),
                        source_anchor: [image.origin[0] + 80.0, image.origin[1] + 60.0],
                        viewport_anchor: [80.0, 60.0],
                    },
                }
            };
            // No replacement publication occurs between identical Resize and this gesture.
            worker.input(input, &output);
            let image = worker.image.as_ref().unwrap();
            if change_layout {
                assert_eq!(image.document_view.as_ref().unwrap().layout, target_layout);
            } else {
                assert_eq!(image.geometry.as_ref().unwrap().scale, 2.0);
            }
            assert!(image.revision > before);
            let replacement = publish_document(&mut worker, &output);
            assert!(replacement.revision > displayed.revision);
        }
    }
}

#[test]
fn actual_document_resize_fences_old_coordinates_until_replacement_is_published() {
    for layout in [DocumentLayout::Paged, DocumentLayout::Continuous] {
        let (mut worker, output, displayed) = document_worker([160, 120], layout, Some(1.0));
        worker.input(
            ViewerInput::Resize(ViewerGeometry {
                width: 320,
                height: 240,
                scale: 2.0,
            }),
            &output,
        );
        let target_layout = if layout == DocumentLayout::Paged {
            DocumentLayout::Continuous
        } else {
            DocumentLayout::Paged
        };
        worker.input(
            ViewerInput::DocumentLayout {
                frame: displayed.revision,
                layout: target_layout,
            },
            &output,
        );
        worker.input(
            ViewerInput::Image {
                frame: displayed.revision,
                input: ViewerImageInput::ViewZoom {
                    zoom: ViewerZoomPreference::ScaleBasisPoints(20000),
                    source_anchor: [200.0, 300.0],
                    viewport_anchor: [80.0, 60.0],
                },
            },
            &output,
        );
        let image = worker.image.as_ref().unwrap();
        assert_eq!(image.document_view.as_ref().unwrap().layout, layout);
        assert_eq!(image.geometry.as_ref().unwrap().scale, 1.0);
        assert_eq!(image.geometry.as_ref().unwrap().viewport, [320, 240]);
        let replacement = publish_document(&mut worker, &output);
        let image = replacement.image.as_ref().unwrap();
        worker.input(
            ViewerInput::Image {
                frame: replacement.revision,
                input: ViewerImageInput::ViewZoom {
                    zoom: ViewerZoomPreference::ScaleBasisPoints(20000),
                    source_anchor: [image.origin[0] + 160.0, image.origin[1] + 120.0],
                    viewport_anchor: [160.0, 120.0],
                },
            },
            &output,
        );
        assert_eq!(
            worker
                .image
                .as_ref()
                .unwrap()
                .geometry
                .as_ref()
                .unwrap()
                .scale,
            2.0,
        );
    }
}

/// The layout chosen last is a viewer preference: the next document opens in it, in Fit,
/// at the top of its first page.
#[test]
fn a_document_opens_in_the_layout_chosen_last_at_the_top_of_its_first_page() {
    for layout in [DocumentLayout::Paged, DocumentLayout::Continuous] {
        let (mut worker, output) = worker();
        worker.preferences = Arc::new(midnight_core::ViewerPreferences {
            document_layout: layout,
            ..Default::default()
        });
        worker.image_size = [160, 120];
        worker.geometry = ViewerGeometry {
            width: 160,
            height: 120,
            scale: 1.0,
        };
        worker.image = Some(crate::image_view::ImageView::new(
            crate::image_worker::ImageWorker::opened_for_test(
                vec![
                    Metadata {
                        width: 400,
                        height: 600,
                    },
                    Metadata {
                        width: 1000,
                        height: 400,
                    },
                ],
                true,
            ),
            worker.image_ids.clone(),
        ));
        worker.poll_image(&output);
        let frame = publish_document(&mut worker, &output);
        let image = frame.image.as_ref().unwrap();
        assert_eq!(image.document_layout, Some(layout));
        assert_eq!(image.page, 1);
        assert!(image.fitted);
        near(image.zoom, 0.2);
        match layout {
            DocumentLayout::Paged => assert_eq!(image.extent, [400.0, 600.0]),
            DocumentLayout::Continuous => {
                assert_eq!(image.extent, [1000.0, 1024.0]);
                assert_eq!(image.origin[1], 0.0, "the top of the first page");
                near(image.origin[0] + 80.0 / 0.2, 500.0);
            }
        }
        assert!(frame.tiles.is_some());
    }
}
