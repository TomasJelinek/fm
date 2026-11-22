use super::*;

fn styled_worker(retained: bool) -> (Worker, Mailbox) {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    worker.positions.reset().unwrap();
    let mut page = decoded_page("return selected_value; // context\n");
    page.has_before = false;
    page.has_after = false;
    page.spans = vec![viewer::Span {
        start: 0,
        end: 6,
        role: viewer::STYLE_KEYWORD,
        flags: 1,
    }];
    worker.length = page.source.offset + page.source.length;
    assert!(worker.set_page(Page::text(page, &worker.appearance), &output));
    worker.selection = Some((7, 12));
    worker.raster.enabled = retained;
    output.take();
    (worker, output)
}

fn painted(worker: &mut Worker, output: &Mailbox) -> (u64, u64, u64, Arc<[u8]>) {
    worker.paint(output).unwrap();
    let events = output.take();
    let (index, frame) = events
        .iter()
        .enumerate()
        .find_map(|(index, event)| match event {
            ViewerOutput::Frame(frame) => Some((index, frame)),
            _ => None,
        })
        .expect("a complete test page must paint");
    let (ack_index, revision, generation, floor) = events
        .iter()
        .enumerate()
        .find_map(|(index, event)| match event {
            ViewerOutput::PaletteApplied {
                revision,
                generation,
                minimum_frame_revision: Some(floor),
            } => Some((index, *revision, *generation, *floor)),
            _ => None,
        })
        .expect("a new palette/frame generation needs a concrete fence");
    assert!(ack_index < index);
    assert_eq!(generation, frame.generation);
    assert_eq!(floor, frame.revision);
    // A retained frame's pixels are its prepared strips, stacked.
    let pixels: Arc<[u8]> = if frame.raster.is_some() {
        super::raster::stacked(frame).1.into()
    } else {
        frame.pixels.clone()
    };
    (revision, generation, floor, pixels)
}

#[test]
fn latest_palette_recolors_active_and_cached_pages_without_source_or_camera_reset() {
    let (mut worker, output) = styled_worker(false);
    let (_, generation, _, old_pixels) = painted(&mut worker, &output);
    let page = worker.page.as_ref().unwrap().clone();
    worker.position_page = Some((0, page.clone()));
    let selection = worker.selection;
    let origin = worker.origin;
    let preferences = worker.preferences.clone();
    let plugin_generation = worker.generation;
    let viewport_request = worker.viewport_request;
    let mut palette = worker.appearance.clone();
    palette.family = "UI typography must not be applied".into();
    palette.font_size = 77.0;
    palette.foreground = [30, 31, 32, 255];
    palette.background = [250, 249, 248, 255];
    palette.selection = [80, 100, 120, 255];
    palette.accent = [170, 30, 50, 255];
    assert_eq!(
        worker.palette_state.restore(&palette, || Ok(())).unwrap(),
        2
    );
    let family = worker.appearance.family.clone();
    let size = worker.appearance.font_size;
    worker.sync_palette(&output);
    assert_eq!(worker.appearance.family, family);
    assert_eq!(worker.appearance.font_size, size);
    assert_eq!(worker.origin, origin);
    assert_eq!(worker.selection, selection);
    assert!(Arc::ptr_eq(&worker.preferences, &preferences));
    assert_eq!(worker.generation, plugin_generation);
    assert_eq!(worker.viewport_request, viewport_request);
    assert_eq!(worker.page.as_ref().unwrap().text, page.text);
    assert_eq!(worker.page.as_ref().unwrap().roles, page.roles);
    assert_eq!(
        worker.position_page.as_ref().unwrap().1.spans[0].color,
        worker.page.as_ref().unwrap().spans[0].color
    );
    let (revision, current_generation, _, pixels) = painted(&mut worker, &output);
    assert_eq!(revision, 2);
    assert_eq!(generation, current_generation);
    assert_ne!(pixels, old_pixels);
}

#[test]
fn a_new_selection_text_color_repaints_the_selected_text_and_nothing_else() {
    // Retained pixels: a palette that changes only the selection pair is not laid out
    // again, but its retained raster must still be painted again.
    let (mut worker, output) = styled_worker(true);
    let (_, generation, first, own_colors) = painted(&mut worker, &output);
    let spans = worker.page.as_ref().unwrap().spans.clone();
    let mut yellow = worker.appearance.clone();
    yellow.selection_text = [255, 255, 0, 255];
    assert_eq!(worker.palette_state.restore(&yellow, || Ok(())).unwrap(), 2);
    worker.sync_palette(&output);
    let (revision, current_generation, floor, selected_yellow) = painted(&mut worker, &output);
    assert_eq!((revision, current_generation), (2, generation));
    assert!(floor > first, "a new real frame fences the new color");
    assert_ne!(
        selected_yellow, own_colors,
        "selected text is painted in the selection text color"
    );
    assert_eq!(
        worker.page.as_ref().unwrap().spans[0].color,
        spans[0].color,
        "text colors are unchanged"
    );
    // Without a selection the two palettes paint the same pixels.
    let (mut worker, output) = styled_worker(false);
    worker.selection = None;
    let (_, _, _, unselected_own) = painted(&mut worker, &output);
    let mut yellow = worker.appearance.clone();
    yellow.selection_text = [255, 255, 0, 255];
    worker.palette_state.restore(&yellow, || Ok(())).unwrap();
    worker.sync_palette(&output);
    let (_, _, _, unselected_yellow) = painted(&mut worker, &output);
    assert_eq!(unselected_own, unselected_yellow);
}

#[test]
fn skipped_b_then_a_still_publishes_a_new_real_frame_floor() {
    let (mut worker, output) = styled_worker(false);
    let (_, generation, first, original_pixels) = painted(&mut worker, &output);
    let a = worker.appearance.clone();
    let mut b = a.clone();
    b.background = [240; 4];
    assert_eq!(worker.palette_state.restore(&b, || Ok(())).unwrap(), 2);
    assert_eq!(worker.palette_state.restore(&a, || Ok(())).unwrap(), 3);
    worker.sync_palette(&output);
    let (revision, current_generation, floor, pixels) = painted(&mut worker, &output);
    assert_eq!((revision, current_generation), (3, generation));
    assert!(floor > first);
    assert_eq!(pixels, original_pixels);
}

#[test]
fn rollback_discards_a_prepared_b_retained_raster_without_changing_generation() {
    let (mut worker, output) = styled_worker(true);
    let (_, generation, _, original_pixels) = painted(&mut worker, &output);
    let a = worker.appearance.clone();
    let mut b = a.clone();
    b.background = [240; 4];
    worker.palette_state.restore(&b, || Ok(())).unwrap();
    worker.sync_palette(&output);
    let prepared_b = worker.retained_pixels().unwrap().unwrap();
    let bands = prepared_b.info.bands.as_ref().expect("prepared strips");
    let prepared_b_pixels: Vec<u8> = bands
        .list
        .iter()
        .flat_map(|band| band.pixels.iter().copied())
        .collect();
    assert_ne!(*prepared_b_pixels, *original_pixels);
    worker.palette_state.restore(&a, || Ok(())).unwrap();
    worker.sync_palette(&output);
    let (revision, current_generation, _, pixels) = painted(&mut worker, &output);
    assert_eq!((revision, current_generation), (3, generation));
    assert_eq!(pixels, original_pixels);
}

#[test]
fn unchanged_palette_is_reannounced_after_same_route_position_context_change() {
    let (mut worker, output) = styled_worker(false);
    let (request, generation, _, _) = painted(&mut worker, &output);
    worker.position_context(&output);
    let (revision, next_generation, _, _) = painted(&mut worker, &output);
    assert_eq!(revision, request);
    assert_ne!(next_generation, generation);
}

#[test]
fn author_image_none_fences_are_generation_bound_and_do_not_claim_text_pixels() {
    let (mut worker, output) = worker();
    worker.positions.reset().unwrap();
    worker.image = Some(crate::image_view::ImageView::new(
        crate::image_worker::ImageWorker::opened_for_test(
            vec![midnight_image_viewer_host::Metadata {
                width: 1,
                height: 1,
            }],
            false,
        ),
        worker.image_ids.clone(),
    ));
    worker.announce_content_palette(&output);
    let first = worker.positions.generation;
    assert!(
        matches!(output.take().as_slice(), [ViewerOutput::PaletteApplied { revision: 1, generation, minimum_frame_revision: None }] if *generation == first)
    );
    worker.position_context(&output);
    worker.announce_content_palette(&output);
    assert!(
        matches!(output.take().as_slice(), [ViewerOutput::PaletteApplied { revision: 1, generation, minimum_frame_revision: None }] if *generation != first)
    );
    assert!(output.take_open_result().is_none());
}

#[test]
fn plain_binary_and_markdown_repaint_existing_pages_with_the_new_palette() {
    use midnight_plugin_host::api::document;
    for mode in [
        viewer::MODE_TEXT,
        viewer::MODE_BINARY,
        viewer::MODE_DOCUMENT,
    ] {
        let (mut worker, output) = worker();
        worker.positions.reset().unwrap();
        worker.mode = mode;
        worker.binary = mode == viewer::MODE_BINARY;
        worker.apply_font();
        let page = match mode {
            viewer::MODE_BINARY => Page::binary(
                viewer::BinaryPage {
                    revision: 1,
                    source: viewer::SourceRange {
                        offset: 0,
                        length: 4,
                    },
                    has_before: false,
                    has_after: false,
                },
                b"text",
            ),
            viewer::MODE_DOCUMENT => Page::document(
                document::DocumentPage {
                    revision: 1,
                    source: viewer::SourceRange {
                        offset: 0,
                        length: 7,
                    },
                    has_before: false,
                    has_after: false,
                    blocks: vec![document::Block {
                        kind: document::BLOCK_HEADING,
                        heading_level: 2,
                        depth: 0,
                        links: vec![],
                        table: None,
                        strikethrough: None,
                        text: viewer::TextPage {
                            revision: 1,
                            source: viewer::SourceRange {
                                offset: 0,
                                length: 7,
                            },
                            text: "Heading".into(),
                            spans: vec![],
                            anchors: vec![viewer::Anchor {
                                text_offset: 0,
                                source_offset: 0,
                            }],
                            has_before: false,
                            has_after: false,
                            partial_line: false,
                        },
                    }],
                },
                &worker.appearance,
            ),
            _ => text_page("plain page".into()),
        };
        let raw = (page.start, page.end);
        let text = page.text.clone();
        worker.length = page.end;
        assert!(worker.set_page(page, &output));
        let (_, generation, _, first) = painted(&mut worker, &output);
        let font = (
            worker.appearance.family.clone(),
            worker.appearance.font_size,
        );
        let mut colors = worker.appearance.clone();
        colors.background = [250; 4];
        colors.foreground = [20, 21, 22, 255];
        colors.accent = [170, 30, 50, 255];
        colors.family = "application font is separate".into();
        colors.font_size = 75.0;
        worker.palette_state.restore(&colors, || Ok(())).unwrap();
        worker.sync_palette(&output);
        let (revision, next_generation, _, second) = painted(&mut worker, &output);
        assert_eq!((revision, next_generation), (2, generation));
        assert_ne!(first, second);
        assert_eq!(
            (
                worker.appearance.family.clone(),
                worker.appearance.font_size
            ),
            font
        );
        let page = worker.page.as_ref().unwrap();
        assert_eq!(page.text, text);
        assert_eq!((page.start, page.end), raw);
    }
}
