use super::*;

#[path = "worker_file_navigation_tests.rs"]
mod file_navigation;

#[path = "worker_position_tests.rs"]
mod position;

#[path = "worker_raster_tests.rs"]
mod raster;
#[path = "worker_source_continuity_tests.rs"]
mod source_continuity;

#[test]
fn opening_acknowledges_a_missing_plugin_as_failure_instead_of_window_success() {
    let (mut worker, output) = worker();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("generated.txt");
    std::fs::write(&path, b"generated list\n").unwrap();
    let captured = midnight_plugin_host::CapturedFile::capture(&path).unwrap();
    worker.files = vec![(path, Some(Ok(captured)))];
    worker.basic = true;
    assert!(output.take_open_result().is_none());
    worker.open(&output);
    let failure = output
        .take_open_result()
        .expect("initial open must finish")
        .unwrap_err();
    assert!(failure.contains("No compatible viewer plugin"), "{failure}");
    assert!(
        output.take_open_result().is_none(),
        "one acknowledgement only"
    );
}

#[test]
fn opening_waits_for_rendered_content_and_cannot_be_completed_twice() {
    let (mut worker, output) = worker();
    output.put(ViewerOutput::Status("Opening…".into()));
    assert!(output.take_open_result().is_none());
    worker.viewport_request = Some(41);
    worker.event(
        41,
        EventKind::TextPage(decoded_page("generated list")),
        &output,
    );
    assert!(
        output.take_open_result().is_none(),
        "decoded content is not yet rendered"
    );
    worker.event(41, EventKind::Completed, &output);
    worker.paint(&output).unwrap();
    assert_eq!(output.take_open_result(), Some(Ok(())));
    worker.error("a later request failed".into(), &output);
    worker.paint(&output).unwrap();
    assert!(
        output.take_open_result().is_none(),
        "later activity cannot reopen completion"
    );
}

#[test]
fn opening_reports_plugin_open_and_initial_viewport_errors_without_a_frame() {
    for request in [1, 41] {
        let (mut worker, output) = worker();
        worker.viewport_request = Some(41);
        worker.event(
            request,
            EventKind::Failed(messages::Failure {
                code: 1,
                message: "plugin rejected the generated content".into(),
            }),
            &output,
        );
        assert_eq!(
            output.take_open_result(),
            Some(Err("plugin rejected the generated content".into()))
        );
        assert!(
            output
                .take()
                .iter()
                .all(|event| !matches!(event, ViewerOutput::Frame(_)))
        );
    }
}

fn worker() -> (Worker, Mailbox) {
    let root = tempfile::tempdir().unwrap();
    let runtime = Arc::new(Runtime::discover(root.path()).unwrap());
    let raster_budget = Arc::new(crate::raster_budget::Budget::default());
    let worker = Worker {
        wake: Arc::new(|| true),
        preference_state: Arc::new(crate::preferences::Preferences::new(Arc::new(
            ViewerPreferences::default(),
        ))),
        preference_revision: 1,
        preferences: Arc::new(ViewerPreferences::default()),
        runtime,
        images: Arc::from([]),
        image_startup: None,
        web_providers: Arc::from([]),
        web: None,
        closing_web: None,
        image_ids: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        image: None,
        closing_image: None,
        files: vec![],
        current: 0,
        basic: false,
        associations: Arc::default(),
        disabled_plugins: Arc::from([]),
        selected_mode: None,
        modes: vec![viewer::MODE_TEXT, viewer::MODE_BINARY],
        mode: viewer::MODE_TEXT,
        generation: 0,
        positions: Default::default(),
        position_page: None,
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
        scroll_map: None,
        text: Default::default(),
        raster: Default::default(),
        raster_owner: raster_budget.owner(),
        raster_budget,
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
        continuation: None,
        queued_scroll: [0.0; 2],
        caption: String::new(),
        status: String::new(),
        dirty: false,
        frames: Default::default(),
    };
    (worker, Mailbox::default())
}

#[test]
fn newly_opened_pdf_fits_window_instead_of_reusing_another_documents_manual_zoom() {
    for saved in [
        ViewerZoomPreference::ActualPixels,
        ViewerZoomPreference::ScaleBasisPoints(23_456),
    ] {
        let (mut worker, output) = worker();
        worker.preferences = Arc::new(ViewerPreferences {
            document_zoom: saved,
            ..Default::default()
        });
        worker.image = Some(crate::image_view::ImageView::new(
            crate::image_worker::ImageWorker::opened_for_test(
                vec![
                    midnight_image_viewer_host::Metadata {
                        width: 400,
                        height: 600,
                    },
                    midnight_image_viewer_host::Metadata {
                        width: 1000,
                        height: 400,
                    },
                ],
                true,
            ),
            worker.image_ids.clone(),
        ));
        worker.poll_image(&output);
        let view = worker.image.as_ref().unwrap().geometry.as_ref().unwrap();
        assert!(
            view.fitted,
            "a newly opened PDF must begin in Fit regardless of saved manual zoom"
        );
        assert!((view.scale - 1.0 / 6.0).abs() < 1e-12);
        assert_eq!(
            worker.preferences.document_zoom, saved,
            "opening must not rewrite application preferences"
        );
        worker
            .preference_state
            .restore(Arc::new(ViewerPreferences {
                document_zoom: ViewerZoomPreference::ScaleBasisPoints(50_000),
                ..Default::default()
            }))
            .unwrap();
        worker.sync_preferences(&output);
        assert!(
            worker
                .image
                .as_ref()
                .unwrap()
                .geometry
                .as_ref()
                .unwrap()
                .fitted,
            "late settings restoration must not replace a new PDF's Fit view"
        );

        let frame = worker.frames.painted().unwrap();
        worker.image.as_mut().unwrap().frames.painted(frame);
        assert!(worker.image_input(&ViewerInput::Image {
            frame,
            input: ViewerImageInput::ActualPixels
        }));
        let frame = worker.frames.painted().unwrap();
        worker.image.as_mut().unwrap().frames.painted(frame);
        assert!(worker.image_input(&ViewerInput::Image {
            frame,
            input: ViewerImageInput::Zoom {
                factor: 1.234567,
                anchor: None
            }
        }));
        let manual = worker
            .image
            .as_ref()
            .unwrap()
            .geometry
            .as_ref()
            .unwrap()
            .scale;
        worker
            .preference_state
            .restore(Arc::new(ViewerPreferences::default()))
            .unwrap();
        worker.sync_preferences(&output);
        for delta in [1, -1] {
            let frame = worker.frames.painted().unwrap();
            assert!(worker.image_input(&ViewerInput::DocumentPage { frame, delta }));
            let view = worker.image.as_ref().unwrap().geometry.as_ref().unwrap();
            assert!(!view.fitted);
            assert_eq!(
                view.scale, manual,
                "page turns must preserve exact runtime manual zoom"
            );
        }
    }
}

#[test]
fn pdf_fit_enlarges_small_pages_with_bounded_raster_while_image_fit_does_not() {
    for document in [true, false] {
        let (mut worker, output) = worker();
        worker.image_size = [1200, 1200];
        worker.image = Some(crate::image_view::ImageView::new(
            crate::image_worker::ImageWorker::opened_for_test(
                vec![midnight_image_viewer_host::Metadata {
                    width: 400,
                    height: 600,
                }],
                document,
            ),
            worker.image_ids.clone(),
        ));
        worker.poll_image(&output);
        let view = worker.image.as_mut().unwrap().geometry.as_mut().unwrap();
        assert_eq!(view.scale, if document { 2.0 } else { 1.0 });
        view.resize([2400, 1200]);
        view.change_page(1000, 400);
        assert!(view.fitted);
        assert_eq!(view.scale, if document { 2.4 } else { 1.0 });
        view.change_page(1, 1);
        view.resize([10_000, 10_000]);
        assert_eq!(view.scale, if document { 64.0 } else { 1.0 });
        let (raster, _) = view.raster_geometry();
        assert!(raster.iter().all(|dimension| *dimension <= 4096));
        assert!(u64::from(raster[0]) * u64::from(raster[1]) <= 4_194_304);
        view.actual();
        view.zoom(1.234567, view.center());
        view.change_page(400, 600);
        assert!(!view.fitted);
        assert_eq!(view.scale, 1.234567);
    }
}

#[test]
fn newly_opened_raster_image_still_restores_its_saved_zoom() {
    let (mut worker, output) = worker();
    worker.preferences = Arc::new(ViewerPreferences {
        image_zoom: ViewerZoomPreference::ScaleBasisPoints(23_456),
        ..Default::default()
    });
    worker.image = Some(crate::image_view::ImageView::new(
        crate::image_worker::ImageWorker::opened_for_test(
            vec![midnight_image_viewer_host::Metadata {
                width: 400,
                height: 600,
            }],
            false,
        ),
        worker.image_ids.clone(),
    ));
    worker.poll_image(&output);
    let view = worker.image.as_ref().unwrap().geometry.as_ref().unwrap();
    assert!(!view.fitted);
    assert!((view.scale - 2.3456).abs() < 1e-12);
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
        tables: vec![],
        document_blocks: Vec::new(),
        style_size: 14.0,
        before: true,
        after: true,
    }
}

fn decoded_page(text: &str) -> viewer::TextPage {
    viewer::TextPage {
        revision: 1,
        source: viewer::SourceRange {
            offset: 8000,
            length: text.len() as u64,
        },
        text: text.into(),
        spans: vec![],
        anchors: vec![
            viewer::Anchor {
                text_offset: 0,
                source_offset: 8000,
            },
            viewer::Anchor {
                text_offset: text.len() as u32,
                source_offset: 8000 + text.len() as u64,
            },
        ],
        has_before: true,
        has_after: true,
        partial_line: false,
    }
}

#[test]
fn markdown_presets_reflow_the_visible_document_without_another_plugin_page() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_DOCUMENT;
    worker.apply_font();
    let mut page =
        text_page("Paragraph with text.\n\nA second paragraph.\n\nThird paragraph.".into());
    page.before = false;
    page.after = false;
    assert!(worker.set_page(page, &output));
    let github = worker.shaped.as_ref().unwrap().height();
    worker
        .preference_state
        .change(&ViewerPreferenceChange::MarkdownPreset(
            MarkdownPreset::Compact,
        ))
        .unwrap();
    worker.sync_preferences(&output);
    let compact = worker.shaped.as_ref().unwrap().height();
    worker
        .preference_state
        .change(&ViewerPreferenceChange::MarkdownPreset(
            MarkdownPreset::Comfortable,
        ))
        .unwrap();
    worker.sync_preferences(&output);
    let comfortable = worker.shaped.as_ref().unwrap().height();
    assert!(
        compact < github && github < comfortable,
        "presets must change visible spacing: {compact}, {github}, {comfortable}"
    );
}

#[test]
fn source_line_numbers_change_pixels_without_changing_copied_text() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    let mut page = text_page("one\ntwo\nthree".into());
    page.start = 0;
    page.before = false;
    assert!(worker.set_page(page, &output));
    let frame = |output: &Mailbox| {
        output
            .take()
            .into_iter()
            .find_map(|event| match event {
                ViewerOutput::Frame(frame) => Some(frame),
                _ => None,
            })
            .unwrap()
    };
    worker.paint(&output).unwrap();
    let numbered = frame(&output);
    worker
        .preference_state
        .change(&ViewerPreferenceChange::LineNumbers(false))
        .unwrap();
    worker.sync_preferences(&output);
    worker.paint(&output).unwrap();
    let plain = frame(&output);
    assert!(
        numbered.pixels != plain.pixels,
        "line number toggle must change the rendered surface"
    );
    assert_eq!(numbered.accessible_text, plain.accessible_text);
    assert_eq!(
        worker
            .shaped
            .as_ref()
            .unwrap()
            .selected_text(0..13)
            .unwrap(),
        "one\ntwo\nthree"
    );
}

#[test]
fn source_coordinates_count_scalars_and_ignore_visual_wraps() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    worker.wrap = true;
    worker.geometry.width = 100;
    let source = "A\t文e\u{301}😀\nsecond logical line with wrapping";
    assert!(worker.set_page(text_page(source.into()), &output));
    worker.text.coordinate = Some(text::Coordinate {
        source_offset: 8000,
        line: 123,
        column: Some(50),
    });
    let newline = source.find('\n').unwrap();
    let at = worker
        .text
        .at(worker.page.as_ref().unwrap(), newline)
        .unwrap();
    assert_eq!((at.line, at.column), (123, Some(56)));
    let next = worker
        .text
        .at(worker.page.as_ref().unwrap(), newline + 1)
        .unwrap();
    assert_eq!((next.line, next.column), (124, Some(1)));
    assert!(worker.shaped.as_ref().unwrap().line_count() > 2);
    worker.prepare_gutter().unwrap();
    assert_eq!(
        worker.text.gutter.as_ref().unwrap().len(),
        2,
        "soft wrapping must not introduce logical line numbers"
    );
    worker.selection = Some((0, newline));
    assert!(
        worker
            .source_status(
                worker.page.as_ref().unwrap(),
                worker.shaped.as_ref().unwrap()
            )
            .starts_with("Ln 123, Col 56")
    );
}

#[test]
fn gutter_hit_testing_and_font_reflow_keep_selection_in_original_text() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    let mut page = text_page("first\nsecond".into());
    page.before = false;
    assert!(worker.set_page(page, &output));
    worker.paint(&output).unwrap();
    let revision = output
        .take()
        .into_iter()
        .find_map(|event| match event {
            ViewerOutput::Frame(frame) => Some(frame.revision),
            _ => None,
        })
        .unwrap();
    let caret = worker.shaped.as_ref().unwrap().caret_geometry(7).unwrap();
    worker.input(
        ViewerInput::Select {
            frame: revision,
            x: caret.x0 as f32 + worker.text_margin() + 0.1,
            y: ((caret.y0 + caret.y1) / 2.0) as f32,
            extend: false,
        },
        &output,
    );
    assert_eq!(worker.selection, Some((7, 7)));
    worker
        .preference_state
        .change(&ViewerPreferenceChange::FontSize(
            ViewerFontKind::Source,
            24,
        ))
        .unwrap();
    worker.sync_preferences(&output);
    assert_eq!(worker.selection, Some((7, 7)));
    assert_eq!(
        worker
            .shaped
            .as_ref()
            .unwrap()
            .selected_text(6..12)
            .unwrap(),
        "second"
    );
}

#[test]
fn locate_reply_updates_numbers_without_acknowledging_or_replacing_a_viewport() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    assert!(worker.set_page(text_page("visible".into()), &output));
    worker.text.locate_request = Some((12, 8000));
    let position = viewer::TextPosition {
        revision: 1,
        source_offset: 8000,
        line: 501,
        column: 8,
    };
    worker.event(11, EventKind::TextPosition(position), &output);
    assert!(
        worker.text.coordinate.is_none(),
        "stale locate result cannot replace current coordinates"
    );
    worker.event(12, EventKind::TextPosition(position), &output);
    worker.event(12, EventKind::Completed, &output);
    assert_eq!(worker.positions.acknowledged, [0; 2]);
    assert_eq!(worker.page.as_ref().unwrap().text, "visible");
    assert_eq!(worker.text.coordinate.unwrap().line, 501);
    assert!(!worker.text.unavailable);
    worker.text.locate_request = Some((13, 8000));
    worker.event(
        13,
        EventKind::Failed(messages::Failure {
            code: midnight_plugin_host::api::abi::UNSUPPORTED,
            message: "old plugin".into(),
        }),
        &output,
    );
    assert!(worker.text.unavailable);
    assert!(
        output
            .take()
            .iter()
            .all(|event| !matches!(event, ViewerOutput::Status(_)))
    );
}

#[test]
fn rounded_source_boundary_does_not_leave_a_completed_lookup_marked_indexing() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    assert!(worker.set_page(text_page("\nnext".into()), &output));
    worker.text.coordinate = Some(text::Coordinate {
        source_offset: 8001,
        line: 52,
        column: Some(1),
    });
    assert!(
        worker
            .source_status(
                worker.page.as_ref().unwrap(),
                worker.shaped.as_ref().unwrap()
            )
            .contains("unavailable")
    );
    worker.selection = Some((1, 2));
    assert!(
        worker
            .source_status(
                worker.page.as_ref().unwrap(),
                worker.shaped.as_ref().unwrap()
            )
            .starts_with("Ln 52, Col 2")
    );
}

#[test]
fn markdown_font_resize_scales_headings_and_retains_read_only_text() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_DOCUMENT;
    worker.apply_font();
    let mut page = text_page("Heading\n\nParagraph".into());
    page.style_size = 16.0;
    page.styles.push(midnight_viewer_render::StyleSpan {
        range: 0..7,
        weight: 700,
        italic: false,
        underline: false,
        strikethrough: false,
        monospace: false,
        size: 28.0,
    });
    assert!(worker.set_page(page, &output));
    let height = worker
        .shaped
        .as_ref()
        .unwrap()
        .caret_geometry(0)
        .unwrap()
        .y1;
    worker
        .preference_state
        .change(&ViewerPreferenceChange::FontSize(
            ViewerFontKind::Markdown,
            32,
        ))
        .unwrap();
    worker.sync_preferences(&output);
    let resized = worker.shaped.as_ref().unwrap();
    assert!(
        resized.caret_geometry(0).unwrap().y1 > height * 1.8,
        "semantic heading size must track the selected body size"
    );
    assert_eq!(
        resized.selected_text(0..18).unwrap(),
        "Heading\n\nParagraph"
    );
}

#[test]
fn scrollbar_ack_waits_for_the_matching_page_and_completed_event() {
    let (mut worker, output) = worker();
    worker.length = 20_000;
    worker.viewport_request = Some(42);
    assert!(worker.positions.accept(0, 9, false, 0.8));
    worker.positions.requested(42, 9);
    worker.event(41, EventKind::TextPage(decoded_page("obsolete")), &output);
    assert!(worker.page.is_none());
    worker.event(
        42,
        EventKind::TextPage(decoded_page("captured requested content")),
        &output,
    );
    worker.paint(&output).unwrap();
    assert!(
        output
            .take()
            .iter()
            .all(|event| !matches!(event, ViewerOutput::Frame(_))),
        "uncompleted page cannot be published with an older acknowledgement"
    );
    worker.event(41, EventKind::Completed, &output);
    assert_eq!(worker.positions.acknowledged, [0; 2]);
    worker.event(42, EventKind::Completed, &output);
    worker.paint(&output).unwrap();
    let ready = output
        .take()
        .into_iter()
        .find_map(|event| {
            if let ViewerOutput::Frame(frame) = event {
                Some(frame)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(ready.position_sequence, [9, 0]);
    worker.viewport_request = Some(43);
    worker.positions.requested(43, 10);
    worker.event(43, EventKind::Cancelled, &output);
    worker.paint(&output).unwrap();
    let cancelled = output
        .take()
        .into_iter()
        .find_map(|event| {
            if let ViewerOutput::Frame(frame) = event {
                Some(frame)
            } else {
                None
            }
        })
        .unwrap();
    assert!(cancelled.generation > ready.generation);
    assert_eq!(cancelled.position_sequence, [0; 2]);
}

#[test]
fn continuous_positions_keep_current_page_work_and_latest_deferred_intent_without_false_ack() {
    let (mut worker, output) = worker();
    worker.length = 20_000;
    worker.viewport_request = Some(42);
    worker.positions.accept(0, 1, false, 0.1);
    worker.positions.requested(42, 1);
    for sequence in 2..10_000 {
        worker.input(
            ViewerInput::Position {
                generation: 0,
                sequence,
                horizontal: false,
                position: sequence as f64 / 10_000.0,
            },
            &output,
        );
        assert_eq!(worker.viewport_request, Some(42));
    }
    assert_eq!(worker.positions.take_deferred(), Some((9999, 0.9999)));
    worker.event(
        42,
        EventKind::TextPage(decoded_page("first completed viewport")),
        &output,
    );
    worker.event(42, EventKind::Completed, &output);
    worker.paint(&output).unwrap();
    assert_eq!(worker.positions.acknowledged, [1, 0]);
    output.take();
    worker.viewport_request = Some(43);
    worker.positions.requested(43, 9999);
    // A new horizontal ack must not accidentally publish the new vertical page early.
    worker.event(
        43,
        EventKind::TextPage(decoded_page("later viewport ".repeat(60).as_str())),
        &output,
    );
    worker.input(
        ViewerInput::Position {
            generation: 0,
            sequence: 10_000,
            horizontal: true,
            position: 0.5,
        },
        &output,
    );
    worker.paint(&output).unwrap();
    assert!(
        output
            .take()
            .iter()
            .all(|event| !matches!(event, ViewerOutput::Frame(_)))
    );
    worker.event(43, EventKind::Completed, &output);
    worker.paint(&output).unwrap();
    let ready = output
        .take()
        .into_iter()
        .find_map(|event| {
            if let ViewerOutput::Frame(frame) = event {
                Some(frame)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(ready.position_sequence, [9999, 10_000]);
}

#[test]
fn no_match_search_publishes_new_context_and_next_scrollbar_input_is_accepted() {
    let (mut worker, output) = worker();
    worker.length = 20_000;
    worker.set_page(text_page("very long captured line ".repeat(60)), &output);
    worker.paint(&output).unwrap();
    output.take();
    worker.dirty = false;
    worker.input(
        ViewerInput::Search {
            query: "missing".into(),
            backwards: false,
            case_sensitive: false,
        },
        &output,
    );
    worker.search_request = Some(99);
    worker.status = "Searching…".into();
    worker.event(99, EventKind::Completed, &output);
    assert!(
        worker.dirty,
        "context reset must repaint even without a match"
    );
    worker.paint(&output).unwrap();
    let frame = output
        .take()
        .into_iter()
        .find_map(|event| {
            if let ViewerOutput::Frame(frame) = event {
                Some(frame)
            } else {
                None
            }
        })
        .unwrap();
    assert!(frame.generation > 0);
    worker.input(
        ViewerInput::Position {
            generation: frame.generation,
            sequence: 3,
            horizontal: true,
            position: 0.7,
        },
        &output,
    );
    assert_eq!(worker.positions.acknowledged, [0, 3]);
    let maximum = worker.shaped.as_ref().unwrap().width() - worker.geometry.width as f32;
    assert!((worker.origin[0] - maximum * 0.7).abs() < 0.001);
    worker.set_page(text_page("a longer captured line ".repeat(120)), &output);
    let maximum = worker.shaped.as_ref().unwrap().width() - worker.geometry.width as f32;
    assert!(
        (worker.origin[0] - maximum * 0.7).abs() < 0.001,
        "horizontal intent survives the matching page load"
    );
}

#[test]
fn newer_horizontal_wheel_clears_the_previous_absolute_percentage_before_the_next_page() {
    let (mut worker, output) = worker();
    worker.set_page(
        text_page("long line with many columns ".repeat(80)),
        &output,
    );
    worker.input(
        ViewerInput::Position {
            generation: 0,
            sequence: 1,
            horizontal: true,
            position: 0.4,
        },
        &output,
    );
    let before = worker.origin[0];
    worker.input(ViewerInput::Scroll { x: 19.5, y: 0.0 }, &output);
    assert!((worker.origin[0] - before - 19.5).abs() < 0.001);
    assert_eq!(worker.positions.horizontal, None);
    worker.set_page(text_page("replacement columns ".repeat(180)), &output);
    assert_eq!(
        worker.origin[0], 0.0,
        "an obsolete percentage must not be reapplied"
    );
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

#[test]
fn markdown_seek_lands_at_source_anchor_inside_an_intact_layout_block() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_DOCUMENT;
    let text = "Paragraph words continue without rebuilding their wrapped beginning. ".repeat(30);
    let mut page = text_page(text.clone());
    page.document_blocks
        .push(midnight_viewer_render::DocumentBlock {
            range: 0..text.len(),
            kind: midnight_viewer_render::DocumentKind::Paragraph,
            depth: 0,
        });
    let target = 500;
    worker.anchor = page.anchors[target].source_offset;
    let layout = worker.layout_page(&page).unwrap();
    let expected = layout.caret_geometry(target).unwrap().y0 as f32;
    assert!(expected > 0.0);
    assert!(worker.set_page(page, &output));
    assert!((worker.origin[1] - expected).abs() < 0.01);
}

#[test]
fn markdown_start_keeps_its_top_margin_before_and_after_returning() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_DOCUMENT;
    let mut page = text_page("Heading\n\nbody".repeat(40));
    for anchor in &mut page.anchors {
        anchor.source_offset -= 8000;
    }
    page.start = 0;
    page.end -= 8000;
    page.before = false;
    page.document_blocks
        .push(midnight_viewer_render::DocumentBlock {
            range: 0..page.text.len(),
            kind: midnight_viewer_render::DocumentKind::Paragraph,
            depth: 0,
        });
    worker.anchor = 0;
    assert!(worker.set_page(page, &output));
    assert_eq!(
        worker.origin[1], 0.0,
        "initial document padding must remain visible"
    );
}
