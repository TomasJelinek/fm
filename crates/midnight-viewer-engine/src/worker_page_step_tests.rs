//! Relative document page steps (Ctrl+PageDown/Up, the page buttons) made faster than
//! pages are published: every step counts, clamped to the document, while inputs with
//! coordinates of an outdated page stay refused.
use super::pdf_continuous::{publish_document, try_publish_document};
use super::*;
use midnight_core::{DocumentLayout, ViewerFrame};
use midnight_image_viewer_host::Metadata;

const PAGES: usize = 6;

fn document(pages: usize) -> crate::image_view::ImageView {
    crate::image_view::ImageView::new(
        crate::image_worker::ImageWorker::opened_for_test(
            vec![
                Metadata {
                    width: 400,
                    height: 600,
                };
                pages
            ],
            true,
        ),
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
    )
}

/// A six-page document in the Paged layout, its first page published.
fn six_pages() -> (Worker, Mailbox, ViewerFrame) {
    let (mut worker, output) = worker();
    worker.image_size = [160, 120];
    worker.geometry = ViewerGeometry {
        width: 160,
        height: 120,
        scale: 1.0,
    };
    worker.preferences = Arc::new(ViewerPreferences {
        document_layout: DocumentLayout::Paged,
        ..Default::default()
    });
    worker.image = Some(document(PAGES));
    worker.poll_image(&output);
    let shown = publish_document(&mut worker, &output);
    assert_eq!(shown.image.as_ref().unwrap().page, 1);
    (worker, output, shown)
}

/// The band of the latest camera is in flight, as `paint_image` leaves it after a camera
/// change; the test actor has no thread to render it.
fn rendering(worker: &mut Worker) {
    let image = worker.image.as_mut().unwrap();
    image.pending = Some(image.revision);
}

fn step(worker: &mut Worker, output: &Mailbox, frame: &ViewerFrame, delta: i32) {
    worker.input(
        ViewerInput::DocumentPage {
            frame: frame.revision,
            delta,
        },
        output,
    );
}

fn page(frame: &ViewerFrame) -> u32 {
    frame.image.as_ref().unwrap().page
}

#[test]
fn page_steps_made_before_the_previous_page_is_shown_all_count() {
    for render_between in [false, true] {
        let (mut worker, output, shown) = six_pages();
        // Six presses while the window still shows the first page, as a quick typist or
        // a held key makes them: each one carries that page's frame.
        for _ in 0..PAGES {
            step(&mut worker, &output, &shown, 1);
            if render_between {
                rendering(&mut worker);
            }
        }
        let mut pages = Vec::new();
        while let Some(frame) = try_publish_document(&mut worker, &output) {
            pages.push(page(&frame));
            assert!(pages.len() <= PAGES, "pages shown: {pages:?}");
        }
        assert_eq!(
            pages.last(),
            Some(&(PAGES as u32)),
            "six steps from page 1 of 6 reach the last page (render between: \
             {render_between}); pages shown: {pages:?}"
        );
        assert!(pages.windows(2).all(|pair| pair[0] < pair[1]), "{pages:?}");
        // The last page shown is the last page: one more step changes nothing.
        let last = publish_document_after(&mut worker, &output, &shown, 1);
        assert!(
            last.is_none(),
            "a step past the last page published {last:?}"
        );
    }
}

fn publish_document_after(
    worker: &mut Worker,
    output: &Mailbox,
    frame: &ViewerFrame,
    delta: i32,
) -> Option<u32> {
    step(worker, output, frame, delta);
    try_publish_document(worker, output).map(|frame| page(&frame))
}

#[test]
fn a_step_during_rendering_waits_for_that_page_and_a_held_key_moves_steadily() {
    let (mut worker, output, first) = six_pages();
    step(&mut worker, &output, &first, 1);
    assert_eq!(worker.image.as_ref().unwrap().page, 1, "applied at once");
    rendering(&mut worker);
    let camera = worker.image.as_ref().unwrap().revision;
    step(&mut worker, &output, &first, 1);
    step(&mut worker, &output, &first, 1);
    let image = worker.image.as_ref().unwrap();
    assert_eq!(
        (image.page, image.revision),
        (1, camera),
        "steps during rendering do not chase the camera in progress"
    );
    let second = publish_document(&mut worker, &output);
    assert_eq!(page(&second), 2, "the page in progress is shown");
    assert_eq!(page(&publish_document(&mut worker, &output)), 4);

    // A held key: auto-repeat outpaces rendering, three presses per published page, each
    // carrying the frame the window shows. Every publication moves on, never past the
    // last page, and nothing waits once the key is released.
    let (mut worker, output, mut shown) = six_pages();
    let mut pages = vec![page(&shown)];
    for press in 0..30 {
        step(&mut worker, &output, &shown, 1);
        rendering(&mut worker);
        if press % 3 == 2
            && let Some(frame) = try_publish_document(&mut worker, &output)
        {
            pages.push(page(&frame));
            shown = frame;
            // The steps that waited for it moved the camera: its band is in flight.
            rendering(&mut worker);
        }
    }
    // Released: the steps that waited show at once, then nothing more moves.
    while let Some(frame) = try_publish_document(&mut worker, &output) {
        pages.push(page(&frame));
        assert!(pages.len() <= PAGES + 1, "{pages:?}");
    }
    assert_eq!(pages.last(), Some(&(PAGES as u32)), "{pages:?}");
    assert!(pages.windows(2).all(|pair| pair[0] < pair[1]), "{pages:?}");
    assert!(
        pages.len() >= 3,
        "the held key showed pages on the way: {pages:?}"
    );

    // Back again: steps down clamp at the first page.
    for _ in 0..20 {
        step(&mut worker, &output, &shown, -1);
        rendering(&mut worker);
    }
    let mut last = None;
    while let Some(frame) = try_publish_document(&mut worker, &output) {
        last = Some(page(&frame));
    }
    assert_eq!(last, Some(1));
}

#[test]
fn camera_inputs_after_waiting_steps_apply_to_the_page_the_steps_reach() {
    let (mut worker, output, first) = six_pages();
    step(&mut worker, &output, &first, 1);
    rendering(&mut worker);
    step(&mut worker, &output, &first, 1);
    assert_eq!(worker.image.as_ref().unwrap().page, 1);
    // End of the page, pressed after the step: it scrolls the page the step reaches.
    worker.input(ViewerInput::Seek(1.0), &output);
    assert_eq!(worker.image.as_ref().unwrap().page, 2);
    assert_eq!(page(&publish_document(&mut worker, &output)), 3);
    assert!(try_publish_document(&mut worker, &output).is_none());
}

#[test]
fn fast_page_steps_keep_refusing_outdated_coordinates_and_other_documents() {
    let (mut worker, output, first) = six_pages();
    step(&mut worker, &output, &first, 1);
    rendering(&mut worker);
    step(&mut worker, &output, &first, 1);
    let scale = worker
        .image
        .as_ref()
        .unwrap()
        .geometry
        .as_ref()
        .unwrap()
        .scale;
    // A zoom anchored on the first page and a layout switch made against it: their
    // coordinates are of a page no longer requested.
    worker.input(
        ViewerInput::Image {
            frame: first.revision,
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
            frame: first.revision,
            layout: DocumentLayout::Continuous,
        },
        &output,
    );
    let image = worker.image.as_ref().unwrap();
    assert_eq!(image.page, 2, "the waiting step came first");
    assert_eq!(image.geometry.as_ref().unwrap().scale, scale);
    assert_eq!(
        image.document_view.as_ref().unwrap().layout,
        DocumentLayout::Paged
    );
    let third = publish_document(&mut worker, &output);
    assert_eq!(page(&third), 3);
    // A step never published: refused.
    worker.input(
        ViewerInput::DocumentPage {
            frame: third.revision + 1,
            delta: 1,
        },
        &output,
    );
    assert_eq!(worker.image.as_ref().unwrap().page, 2);

    // Another document opens: a step made on the previous one does not turn its pages.
    worker.frames.invalidate();
    worker.image = Some(document(3));
    worker.poll_image(&output);
    let opened = publish_document(&mut worker, &output);
    assert_eq!(page(&opened), 1);
    step(&mut worker, &output, &third, 1);
    assert_eq!(worker.image.as_ref().unwrap().page, 0);
    assert!(try_publish_document(&mut worker, &output).is_none());
    step(&mut worker, &output, &opened, 1);
    assert_eq!(worker.image.as_ref().unwrap().page, 1);
}
