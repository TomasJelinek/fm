//! Published document layout, frame-scoped input and bounded latest intent.
use super::image_presentation_tests::present as present_outputs;
use super::*;

struct Fixture {
    manager: ViewerWindows,
    open: Rc<OpenWindow>,
    service: Service,
    inputs: Rc<RefCell<Vec<ViewerInput>>>,
    admitting: Rc<Cell<bool>>,
    // Field drops run in declaration order; backend controls must retire first.
    _runtime: image_presentation_tests::TestRuntime,
}

impl Fixture {
    fn new() -> Self {
        let (runtime, manager, open, service) = image_presentation_tests::open();
        open.timer.stop();
        let inputs = Rc::new(RefCell::new(Vec::new()));
        let admitting = Rc::new(Cell::new(true));
        let capture = inputs.clone();
        let admit = admitting.clone();
        *open.input.borrow_mut() = Rc::new(move |input| {
            capture.borrow_mut().push(input);
            admit.get()
        });
        Self {
            manager,
            open,
            service,
            inputs,
            admitting,
            _runtime: runtime,
        }
    }

    fn pdf(&self, generation: u64, revision: u64, layout: DocumentLayout) {
        self.deliver(vec![ViewerOutput::Frame(Box::new(document(
            generation, revision, layout,
        )))]);
    }

    fn deliver(&self, outputs: Vec<ViewerOutput>) {
        present_outputs(&self.open, &self.service, &|_| {}, outputs);
    }

    fn layouts(&self) -> Vec<(u64, DocumentLayout)> {
        self.inputs
            .borrow()
            .iter()
            .filter_map(|input| match input {
                ViewerInput::DocumentLayout { frame, layout } => Some((*frame, *layout)),
                _ => None,
            })
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.manager.close_all();
    }
}

fn document(generation: u64, revision: u64, layout: DocumentLayout) -> ViewerFrame {
    let mut frame = image_presentation_tests::frame();
    frame.generation = generation;
    frame.revision = revision;
    frame.tiles = None;
    frame.pixels = vec![255; 400 * 240 * 4].into();
    let image = frame.image.as_mut().unwrap();
    image.surface = ViewerSurfaceId(20 + revision);
    image.page = 1;
    image.page_count = 4;
    image.document_layout = Some(layout);
    frame
}

#[test]
fn layout_selection_waits_for_publication_and_retries_a_fresh_frame_generation() {
    let fixture = Fixture::new();
    fixture.pdf(1, 2, DocumentLayout::Paged);
    let preferences = fixture.open.session.preferences();
    fixture.open.window.invoke_change_document_layout(1);
    assert_eq!(fixture.layouts(), [(2, DocumentLayout::Continuous)]);
    assert_eq!(fixture.open.window.get_document_layout(), 0);
    assert_eq!(fixture.open.session.preferences(), preferences);

    fixture.deliver(Vec::new());
    assert_eq!(
        fixture.layouts().len(),
        1,
        "one command per published frame"
    );
    // Frame generations also advance for presentation context changes. They are
    // not a source identity and cannot erase a still-pending document choice.
    fixture.pdf(2, 3, DocumentLayout::Paged);
    assert_eq!(
        fixture.layouts(),
        [
            (2, DocumentLayout::Continuous),
            (3, DocumentLayout::Continuous)
        ]
    );
    assert_eq!(fixture.open.window.get_document_layout(), 0);
    fixture.pdf(3, 4, DocumentLayout::Continuous);
    assert_eq!(fixture.open.window.get_document_layout(), 1);
    fixture.deliver(Vec::new());
    assert_eq!(fixture.layouts().len(), 2);

    fixture.pdf(3, 3, DocumentLayout::Paged);
    assert_eq!(fixture.open.frame.get(), 4);
    assert_eq!(fixture.open.window.get_document_layout(), 1);
    assert_eq!(
        fixture.layouts().len(),
        2,
        "a stale frame cannot replay intent"
    );
}

#[test]
fn latest_layout_choice_replaces_pending_switch_even_when_it_matches_displayed_layout() {
    let fixture = Fixture::new();
    fixture.pdf(1, 2, DocumentLayout::Paged);
    fixture.open.window.invoke_change_document_layout(1);
    fixture.open.window.invoke_change_document_layout(0);
    assert_eq!(
        fixture.layouts(),
        [(2, DocumentLayout::Continuous), (2, DocumentLayout::Paged)]
    );
    assert_eq!(fixture.open.window.get_document_layout(), 0);
    fixture.pdf(2, 3, DocumentLayout::Continuous);
    assert_eq!(fixture.open.window.get_document_layout(), 1);
    assert_eq!(fixture.layouts().last(), Some(&(3, DocumentLayout::Paged)));
    fixture.pdf(3, 4, DocumentLayout::Paged);
    fixture.deliver(Vec::new());
    assert_eq!(fixture.open.window.get_document_layout(), 0);
    assert_eq!(
        fixture.layouts().len(),
        3,
        "only the latest intent survives"
    );
}

#[test]
fn refused_layout_admission_retries_without_optimistic_selection_or_stale_acknowledgment() {
    let fixture = Fixture::new();
    fixture.pdf(1, 2, DocumentLayout::Paged);
    fixture.admitting.set(false);
    fixture.open.window.invoke_change_document_layout(1);
    assert_eq!(fixture.open.window.get_document_layout(), 0);
    fixture.admitting.set(true);
    fixture.deliver(Vec::new());
    assert_eq!(
        fixture.layouts(),
        [
            (2, DocumentLayout::Continuous),
            (2, DocumentLayout::Continuous)
        ]
    );
    fixture.pdf(1, 2, DocumentLayout::Continuous);
    assert_eq!(fixture.open.window.get_document_layout(), 0);
    fixture.deliver(Vec::new());
    assert_eq!(fixture.layouts().len(), 2);
    fixture.pdf(2, 3, DocumentLayout::Continuous);
    fixture.deliver(Vec::new());
    assert_eq!(fixture.open.window.get_document_layout(), 1);
    assert_eq!(fixture.layouts().len(), 2);
}

#[test]
fn file_cycle_retires_pending_layout_before_the_next_documents_frame() {
    let fixture = Fixture::new();
    fixture.pdf(1, 2, DocumentLayout::Paged);
    fixture.open.window.invoke_change_document_layout(1);
    assert!(dispatch_key(&fixture.open, "viewer.next-file"));
    fixture.pdf(2, 3, DocumentLayout::Paged);
    assert_eq!(fixture.open.window.get_document_layout(), 0);
    assert_eq!(fixture.layouts().len(), 1);
}

#[test]
fn new_source_layout_handshake_retires_the_former_documents_choice() {
    let fixture = Fixture::new();
    fixture.pdf(1, 2, DocumentLayout::Paged);
    fixture.open.window.invoke_change_document_layout(1);
    fixture.deliver(vec![ViewerOutput::LayoutSelected {
        token: 7,
        mode: VIEWER_MODE_IMAGE,
        content_generation: 2,
    }]);
    fixture.open.window.invoke_change_document_layout(1);
    assert_eq!(
        fixture.layouts().len(),
        1,
        "opening has no published document authority"
    );
    fixture.pdf(2, 3, DocumentLayout::Paged);
    assert_eq!(fixture.open.window.get_document_layout(), 0);
    assert_eq!(fixture.layouts().len(), 1);
}

#[test]
fn source_unavailable_clears_the_pending_layout_choice() {
    let fixture = Fixture::new();
    fixture.pdf(1, 2, DocumentLayout::Paged);
    fixture.open.window.invoke_change_document_layout(1);
    fixture.deliver(vec![ViewerOutput::ImageUnavailable {
        generation: 2,
        surface: ViewerSurfaceId(22),
        message: "Source changed".into(),
    }]);
    fixture.pdf(3, 3, DocumentLayout::Paged);
    assert_eq!(fixture.layouts().len(), 1);
    assert_eq!(fixture.open.window.get_document_layout(), 0);
}

#[test]
fn ordinary_images_and_stale_document_frames_cannot_submit_layout_changes() {
    let fixture = Fixture::new();
    fixture.open.window.invoke_change_document_layout(1);
    assert_eq!(fixture.open.window.get_document_layout(), -1);
    assert!(fixture.layouts().is_empty());
    fixture.pdf(1, 2, DocumentLayout::Paged);
    assert!(!fixture.open.enqueue(ViewerInput::DocumentLayout {
        frame: 1,
        layout: DocumentLayout::Continuous,
    }));
    assert!(fixture.layouts().is_empty());
    fixture.open.window.invoke_change_document_layout(0);
    assert!(
        fixture.layouts().is_empty(),
        "an already acknowledged choice needs no command"
    );
}

#[test]
fn untiled_pdf_wheel_and_scrollbar_input_stays_on_the_backend_in_both_layouts() {
    let fixture = Fixture::new();
    fixture
        .open
        .window
        .window()
        .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
    for (revision, layout) in [(2, DocumentLayout::Paged), (3, DocumentLayout::Continuous)] {
        fixture.pdf(1, revision, layout);
        assert!(!fixture.open.window.get_tiled());
        fixture.inputs.borrow_mut().clear();
        fixture.open.window.invoke_scroll(3.5, 7.25);
        fixture.open.window.invoke_seek(0.75);
        let inputs = fixture.inputs.borrow();
        let [ViewerInput::Scroll { x, y }, ViewerInput::Seek(position)] = inputs.as_slice() else {
            panic!("untiled documents must route wheel and seek to the backend: {inputs:?}");
        };
        assert_eq!([*x, *y], [7.0, 14.5]);
        assert_eq!(*position, 0.75);
    }
}

#[test]
fn pending_zoom_does_not_cross_the_document_layout_boundary() {
    let frame = document(1, 2, DocumentLayout::Paged);
    let mut info = frame.image.unwrap();
    let viewport = [400.0, 240.0];
    let mut intent = image_zoom::Intent::default();
    intent.accepted(
        1,
        &info,
        viewport,
        ViewerZoomPreference::ScaleBasisPoints(20_000),
    );
    assert_eq!(intent.scale(1, &info, viewport), 2.0);
    info.document_layout = Some(DocumentLayout::Continuous);
    assert_eq!(intent.scale(1, &info, viewport), 1.0);
    intent.receive(1, Some(&info));
    info.document_layout = Some(DocumentLayout::Paged);
    assert_eq!(intent.scale(1, &info, viewport), 1.0);
}
