use super::*;
use std::sync::Mutex;

fn worker() -> (Worker, Mailbox) {
    let root = tempfile::tempdir().unwrap();
    let runtime = Arc::new(Runtime::discover(root.path()).unwrap());
    let worker = Worker {
        runtime,
        images: Arc::from([]),
        image: None,
        closing_image: None,
        files: vec![],
        current: 0,
        basic: false,
        disabled_plugins: Arc::from([]),
        selected_mode: None,
        modes: vec![viewer::MODE_TEXT, viewer::MODE_BINARY],
        mode: viewer::MODE_TEXT,
        generation: 0,
        session: None,
        closing: None,
        pending_open: false,
        viewport_request: None,
        search_request: None,
        search_cursor: Default::default(),
        found: None,
        renderer: TextRenderer::default(),
        page: None,
        shaped: None,
        appearance: ViewerAppearance {
            family: "monospace".into(),
            font_size: 14.0,
            foreground: [220; 4],
            background: [20; 4],
            selection: [80; 4],
            accent: [180; 4],
        },
        geometry: ViewerGeometry {
            width: 240,
            height: 100,
            scale: 1.0,
        },
        origin: [0.0; 2],
        image_size: [240, 100],
        selection: None,
        length: 0,
        anchor: 0,
        binary: false,
        wrap: false,
        land_at_bottom: false,
        caption: String::new(),
        status: String::new(),
        dirty: false,
        frames: Default::default(),
    };
    (
        worker,
        Mailbox {
            output: Mutex::new(Vec::new()),
        },
    )
}

fn text_page(text: String) -> Page {
    Page {
        anchors: text
            .char_indices()
            .map(|(at, _)| viewer::Anchor {
                text_offset: at as u32,
                source_offset: 8000 + at as u64,
            })
            .collect(),
        start: 8000,
        end: 8000 + text.len() as u64,
        text,
        spans: vec![],
        styles: vec![],
        before: true,
        after: true,
    }
}

#[test]
fn wrapping_and_resize_preserve_page_anchor_selection_and_visible_text() {
    let (mut worker, output) = worker();
    let text = "alpha beta café 世界 delta epsilon zeta eta theta\n".repeat(50);
    let selected = text.find("café").unwrap();
    let visible = text.match_indices("alpha").nth(20).unwrap().0;
    worker.length = 20_000;
    worker.set_page(text_page(text.clone()), &output);
    worker.selection = Some((selected, selected + "café".len()));
    worker.origin[1] = worker
        .shaped
        .as_ref()
        .unwrap()
        .selection_geometry(visible..visible + 1)
        .unwrap()[0]
        .y0 as f32;
    let original_height = worker.shaped.as_ref().unwrap().height();
    worker.input(ViewerInput::Wrap(true), &output);
    let shaped = worker.shaped.as_ref().unwrap();
    assert!(shaped.height() > original_height);
    assert!(shaped.width() <= 241.0);
    assert_eq!(
        shaped.hit_test(0.0, worker.origin[1] + 1.0).unwrap(),
        visible
    );
    assert_eq!(worker.selection, Some((selected, selected + "café".len())));
    assert_eq!(worker.anchor, 8000);
    assert_eq!(worker.page.as_ref().unwrap().text, text);
    worker.input(
        ViewerInput::Resize(ViewerGeometry {
            width: 140,
            height: 100,
            scale: 1.0,
        }),
        &output,
    );
    let shaped = worker.shaped.as_ref().unwrap();
    assert!(shaped.width() <= 141.0);
    assert_eq!(
        shaped.hit_test(0.0, worker.origin[1] + 1.0).unwrap(),
        visible
    );
    assert_eq!(worker.selection, Some((selected, selected + "café".len())));
    worker.paint(&output).unwrap();
    let frame = output
        .take()
        .into_iter()
        .find_map(|output| {
            if let ViewerOutput::Frame(frame) = output {
                Some(frame)
            } else {
                None
            }
        })
        .unwrap();
    assert!(frame.wrap_enabled && frame.wrap_available);
    assert_eq!(frame.horizontal_extent, 1.0);
}

#[test]
fn source_wraps_but_binary_keeps_fixed_columns_and_printable_bytes() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    worker.set_page(text_page("long source identifier ".repeat(30)), &output);
    worker.input(ViewerInput::Wrap(true), &output);
    assert!(worker.shaped.as_ref().unwrap().line_count() > 1);
    worker.binary = true;
    worker.mode = viewer::MODE_BINARY;
    worker.set_page(
        Page::binary(
            viewer::BinaryPage {
                revision: 1,
                source: viewer::SourceRange {
                    offset: 1 << 35,
                    length: 5,
                },
                has_before: true,
                has_after: false,
            },
            &[0, 1, 255, b'A', b'Z'],
        ),
        &output,
    );
    worker.input(ViewerInput::Wrap(false), &output);
    assert!(
        worker.wrap,
        "fixed-column mode must not change the remembered text setting"
    );
    let page = worker.shaped.as_ref().unwrap();
    assert!(page.width() > 240.0);
    assert!(page.text().starts_with("0000000800000000  00 01 FF 41 5A"));
    assert!(page.text().ends_with(" │ ···AZ\n"));
}
