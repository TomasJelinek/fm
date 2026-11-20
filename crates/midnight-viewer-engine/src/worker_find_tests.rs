//! Regression coverage through the worker's real SearchMatch and viewport entrypoints.
use super::super::tests::worker;
use super::*;

fn page(text: &str, offset: u64) -> Page {
    let mut anchors: Vec<_> = text
        .char_indices()
        .map(|(at, _)| viewer::Anchor {
            text_offset: at as u32,
            source_offset: offset + at as u64,
        })
        .collect();
    anchors.push(viewer::Anchor {
        text_offset: text.len() as u32,
        source_offset: offset + text.len() as u64,
    });
    Page::text(
        viewer::TextPage {
            revision: 1,
            source: viewer::SourceRange {
                offset,
                length: text.len() as u64,
            },
            text: text.into(),
            spans: vec![],
            anchors,
            has_before: offset != 0,
            has_after: false,
            partial_line: false,
        },
        &worker().0.appearance,
    )
}

fn range(text: &str, query: &str, offset: u64) -> viewer::SourceRange {
    viewer::SourceRange {
        offset: offset + text.find(query).unwrap() as u64,
        length: query.len() as u64,
    }
}

#[test]
fn found_text_reuses_retained_page_and_centers_actual_indented_unicode_position() {
    for mode in [
        viewer::MODE_TEXT,
        viewer::MODE_SOURCE,
        viewer::MODE_DOCUMENT,
    ] {
        for retained in [false, true] {
            let (mut worker, output) = worker();
            worker.mode = mode;
            worker.raster.enabled = retained;
            worker.geometry.width = 600;
            let text = format!(
                "{}    α😀 target value\n{}",
                "row\n".repeat(40),
                "row\n".repeat(40)
            );
            let source = range(&text, "target", 0);
            worker.set_page(page(&text, 0), &output);
            let original = worker.page.as_ref().unwrap().text.clone();
            worker.search_cursor.configure(Request {
                query: "target".into(),
                sensitive: true,
                backwards: false,
                whole_words: true,
            });
            worker.search_request = Some(45);
            worker.event(
                44,
                EventKind::SearchMatch(messages::SearchMatch {
                    source: source.clone(),
                    preview: "target".into(),
                }),
                &output,
            );
            assert!(worker.selection.is_none(), "stale request moved the camera");
            worker.event(
                45,
                EventKind::SearchMatch(messages::SearchMatch {
                    source,
                    preview: "target".into(),
                }),
                &output,
            );
            assert!(Arc::ptr_eq(&original, &worker.page.as_ref().unwrap().text));
            let selected = worker.selection.unwrap();
            let shaped = worker.shaped.as_ref().unwrap();
            let caret = shaped.caret_geometry(selected.0).unwrap();
            assert!(caret.x0 > 20.0, "indentation disappeared in mode {mode}");
            assert_eq!(worker.origin[0], 0.0);
            let center = ((caret.y0 + caret.y1) as f32) / 2.0 - worker.origin[1];
            assert!((center - worker.geometry.height as f32 / 2.0).abs() < 0.01);
            assert_eq!(
                &worker.page.as_ref().unwrap().text[selected.0..selected.1],
                "target"
            );
            assert!(worker.search_cursor.candidate.is_none());
        }
    }
}

#[test]
fn unloaded_find_context_starts_at_real_row_and_preserves_indentation() {
    let (mut worker, output) = worker();
    worker.geometry.width = 600;
    worker.set_page(page("old content", 0), &output);
    worker.search_cursor.configure(Request {
        query: "target".into(),
        sensitive: true,
        backwards: false,
        whole_words: true,
    });
    let prefix = page("partial old row\nprevious row\n    α😀 ", 5000);
    let offset = prefix.end;
    let anchor = context_start(&prefix, offset, 1);
    assert_eq!(anchor, 5000 + "partial old row\n".len() as u64);
    worker.search_cursor.candidate = Some(Candidate {
        source: viewer::SourceRange { offset, length: 6 },
        context: Context::Content,
    });
    let text = format!(
        "previous row\n    α😀 target value\n{}",
        "trailing row\n".repeat(40)
    );
    worker.received_page(42, page(&text, anchor), &output);
    assert!(
        worker
            .page
            .as_ref()
            .unwrap()
            .text
            .starts_with("previous row\n    α😀 target")
    );
    let selected = worker.selection.unwrap();
    let caret = worker
        .shaped
        .as_ref()
        .unwrap()
        .caret_geometry(selected.0)
        .unwrap();
    assert!(caret.x0 > 20.0);
    assert!(worker.search_cursor.candidate.is_none());
}

#[test]
fn whole_word_filter_respects_marks_connectors_unicode_and_casefold_scalars() {
    let (mut worker, _) = worker();
    worker.search_cursor.configure(Request {
        query: "cat".into(),
        sensitive: false,
        backwards: false,
        whole_words: true,
    });
    for (text, query, expected) in [
        ("a cat!", "cat", true),
        ("bobcat", "cat", false),
        ("cat_name", "cat", false),
        ("cat\u{301}", "cat", false),
        ("cat\u{203f}tail", "cat", false),
        ("cat\u{200c}tail", "cat", false),
        (" α猫 ", "猫", false),
        (" 猫!", "猫", true),
        ("Straße", "Straße", true),
    ] {
        worker.search_cursor.request.as_mut().unwrap().query = query.into();
        let page = page(text, 0);
        let selected = selection(&page, &range(text, query, 0)).unwrap();
        assert_eq!(
            worker.valid_find(&page, selected),
            Some(expected),
            "{text:?}"
        );
    }
    let page = page("ß", 0);
    worker.search_cursor.request.as_mut().unwrap().query = "s".into();
    assert_eq!(
        worker.valid_find(&page, (0, 2)),
        Some(false),
        "partial full fold accepted"
    );
    worker.search_cursor.request.as_mut().unwrap().query = "SS".into();
    assert_eq!(worker.valid_find(&page, (0, 2)), Some(true));
}

#[test]
fn unknown_word_boundary_is_never_accepted_and_clear_retires_late_context() {
    let (mut worker, output) = worker();
    let mut fragment = page("cat", 20);
    fragment.after = true;
    assert_eq!(word_match(&fragment, (0, 3)), None);
    worker.set_page(fragment, &output);
    worker.selection = Some((0, 3));
    worker.search_cursor.candidate = Some(Candidate {
        source: viewer::SourceRange {
            offset: 20,
            length: 3,
        },
        context: Context::Content,
    });
    worker.viewport_request = Some(42);
    worker.search_request = Some(41);
    worker.input(
        ViewerInput::Search {
            query: String::new(),
            backwards: false,
            case_sensitive: false,
            whole_words: true,
        },
        &output,
    );
    assert!(worker.selection.is_none());
    assert!(worker.found.is_none());
    assert!(worker.search_request.is_none());
    assert!(worker.viewport_request.is_none());
    assert!(worker.search_cursor.candidate.is_none());
    worker.event(
        41,
        EventKind::SearchMatch(messages::SearchMatch {
            source: viewer::SourceRange {
                offset: 20,
                length: 3,
            },
            preview: "cat".into(),
        }),
        &output,
    );
    worker.event(
        42,
        EventKind::TextPage(super::super::tests::decoded_page("late context")),
        &output,
    );
    assert_eq!(&*worker.page.as_ref().unwrap().text, "cat");
    assert!(worker.selection.is_none());
}

#[test]
fn repeated_nonadvancing_provider_match_is_rejected_without_unbounded_retries() {
    let (mut worker, output) = worker();
    worker.set_page(page("bobcat and cat_name", 0), &output);
    worker.search_cursor.configure(Request {
        query: "cat".into(),
        sensitive: false,
        backwards: false,
        whole_words: true,
    });
    let source = viewer::SourceRange {
        offset: 3,
        length: 3,
    };
    worker.found_match(source.clone(), &output);
    assert!(worker.selection.is_none());
    worker.found_match(source, &output);
    assert!(worker.status.contains("without advancing"));
    assert!(worker.search_cursor.candidate.is_none());
}

#[test]
fn rejected_candidate_context_finds_next_valid_word_without_provider_roundtrip() {
    let (mut worker, output) = worker();
    worker.set_page(page("visible content", 0), &output);
    worker.search_cursor.configure(Request {
        query: "cat".into(),
        sensitive: true,
        backwards: false,
        whole_words: true,
    });
    worker.search_cursor.candidate = Some(Candidate {
        source: viewer::SourceRange {
            offset: 5003,
            length: 3,
        },
        context: Context::Content,
    });
    let context = page("bobcat and cat_name and cat!", 5000);
    assert!(worker.find_page(&context, &output));
    assert!(worker.selection.is_some());
    assert_eq!(&*worker.page.as_ref().unwrap().text, &*context.text);
    assert_eq!(worker.found.as_ref().unwrap().offset, 5024);
    assert_eq!(worker.search_cursor.submissions, 0);
}

#[test]
fn matches_at_either_fragment_boundary_reload_adjacent_scalars_before_deciding() {
    for (text, offset, source_offset) in [("cat ", 5000, 5000), (" cat", 5000, 5001)] {
        let (mut worker, output) = worker();
        worker.set_page(page("visible content", 0), &output);
        worker.search_cursor.configure(Request {
            query: "cat".into(),
            sensitive: true,
            backwards: false,
            whole_words: true,
        });
        worker.search_cursor.candidate = Some(Candidate {
            source: viewer::SourceRange {
                offset: source_offset,
                length: 3,
            },
            context: Context::Content,
        });
        let mut fragment = page(text, offset);
        fragment.after = true;
        assert!(worker.find_page(&fragment, &output));
        assert!(matches!(
            worker
                .search_cursor
                .candidate
                .as_ref()
                .map(|candidate| candidate.context),
            Some(Context::Neighbor)
        ));
        assert!(
            worker.status.is_empty(),
            "boundary uncertainty became an error before retry"
        );
        worker.received_page(43, page(" cat ", source_offset - 1), &output);
        assert!(worker.search_cursor.candidate.is_none());
        assert!(worker.selection.is_some());
        assert_eq!(&*worker.page.as_ref().unwrap().text, " cat ");
    }
}

#[test]
fn find_on_huge_row_retries_near_match_without_requiring_unbounded_prefix() {
    for whole_words in [false, true] {
        let (mut worker, output) = worker();
        worker.set_page(page("visible content", 0), &output);
        worker.search_cursor.configure(Request {
            query: "target".into(),
            sensitive: true,
            backwards: false,
            whole_words,
        });
        worker.search_cursor.candidate = Some(Candidate {
            source: viewer::SourceRange {
                offset: 1_000_000,
                length: 6,
            },
            context: Context::Content,
        });
        let mut distant_prefix = page(&"x".repeat(8192), 980_000);
        distant_prefix.after = true;
        assert!(worker.find_page(&distant_prefix, &output));
        assert!(matches!(
            worker
                .search_cursor
                .candidate
                .as_ref()
                .map(|candidate| candidate.context),
            Some(Context::Neighbor)
        ));
        worker.received_page(43, page("suffix  target tail", 999_992), &output);
        assert!(worker.selection.is_some());
        assert!(worker.search_cursor.candidate.is_none());
        let selected = worker.selection.unwrap();
        assert_eq!(
            &worker.page.as_ref().unwrap().text[selected.0..selected.1],
            "target"
        );
    }
}

#[test]
fn context_terminal_without_page_finishes_find_with_visible_error() {
    for terminal in [EventKind::Completed, EventKind::Cancelled] {
        let (mut worker, output) = worker();
        worker.viewport_request = Some(45);
        worker.search_cursor.candidate = Some(Candidate {
            source: viewer::SourceRange {
                offset: 30,
                length: 3,
            },
            context: Context::Content,
        });
        worker.status = SEARCHING.into();
        worker.event(45, terminal, &output);
        assert!(worker.search_cursor.candidate.is_none());
        assert!(worker.viewport_request.is_none());
        assert!(worker.status.contains("Find context was not completed"));
    }
}

#[test]
fn dense_substring_page_uses_bounded_provider_searches_in_both_directions() {
    let text = format!(
        "_{}_",
        "hotdog dog_name dog\u{301} dog\u{203f}tail ".repeat(1024)
    );
    for backwards in [false, true] {
        let (mut worker, output) = worker();
        let mut context = page(&text, 5000);
        context.after = true;
        worker.length = context.end + 5000;
        let end = context.end;
        worker.set_page(context, &output);
        worker.anchor = if backwards { end } else { 5000 };
        worker.start_find("dog".into(), backwards, false, true, &output);
        for _ in 0..20 {
            let anchor = worker.search_cursor.submitted_anchor.unwrap();
            let found = if backwards {
                text.rmatch_indices("dog")
                    .find(|(at, _)| 5000 + *at as u64 + 3 <= anchor)
            } else {
                text.match_indices("dog")
                    .find(|(at, _)| 5000 + *at as u64 >= anchor)
            };
            let Some((at, _)) = found else {
                break;
            };
            worker.found_match(
                viewer::SourceRange {
                    offset: 5000 + at as u64,
                    length: 3,
                },
                &output,
            );
            assert!(worker.selection.is_none());
            assert!(worker.search_cursor.candidate.is_none());
        }
        assert!(
            worker.search_cursor.submissions <= 6,
            "{} provider scans for a page of rejected substrings, backwards={backwards}",
            worker.search_cursor.submissions
        );
        let anchor = worker.search_cursor.submitted_anchor.unwrap();
        assert!(if backwards {
            anchor < 5020
        } else {
            anchor > end - 20
        });
    }
}

#[test]
fn local_word_scan_preserves_full_unicode_fold_and_nearest_direction() {
    let text = "bobSTRASSE Straße straße_ STRASSE";
    let page = page(text, 0);
    let mut request = Request {
        query: "STRASSE".into(),
        sensitive: false,
        backwards: false,
        whole_words: true,
    };
    let Some(LocalWords::Found(forward)) = local_words(&page, &request, 3) else {
        panic!("next full Unicode word expected");
    };
    assert_eq!(forward.offset, text.find("Straße").unwrap() as u64);
    request.backwards = true;
    let Some(LocalWords::Found(backward)) =
        local_words(&page, &request, text.rfind("STRASSE").unwrap() as u64)
    else {
        panic!("previous full Unicode word expected");
    };
    assert_eq!(backward, forward);
}

#[test]
fn painted_match_is_centered_and_keeps_the_real_indented_x_position() {
    let (mut worker, output) = worker();
    worker.appearance.background = [20, 20, 20, 255];
    worker.appearance.foreground = [240, 240, 240, 255];
    worker.appearance.selection = [17, 190, 90, 255];
    worker.geometry.width = 600;
    worker.geometry.height = 180;
    let text = format!(
        "{}    α😀 target value\n{}",
        "row\n".repeat(40),
        "row\n".repeat(40)
    );
    worker.length = text.len() as u64;
    worker.set_page(page(&text, 0), &output);
    worker.search_cursor.configure(Request {
        query: "target".into(),
        sensitive: true,
        backwards: false,
        whole_words: true,
    });
    worker.found_match(range(&text, "target", 0), &output);
    worker.paint(&output).unwrap();
    let frame = output
        .take()
        .into_iter()
        .find_map(|event| match event {
            ViewerOutput::Frame(frame) => Some(frame),
            _ => None,
        })
        .unwrap();
    let mut bounds = [frame.width, frame.height, 0, 0];
    for (index, pixel) in frame.pixels.as_chunks::<4>().0.iter().enumerate() {
        if *pixel == [17, 190, 90, 255] {
            let x = index as u32 % frame.width;
            let y = index as u32 / frame.width;
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].min(y);
            bounds[2] = bounds[2].max(x + 1);
            bounds[3] = bounds[3].max(y + 1);
        }
    }
    assert!(
        bounds[0] > 20 && bounds[0] < bounds[2],
        "painted match lost its column: {bounds:?}"
    );
    assert!(bounds[1] < bounds[3]);
    let center = (bounds[1] + bounds[3]) as f32 / 2.0;
    assert!(
        (center - frame.height as f32 / 2.0).abs() <= 2.0,
        "painted match outside center: {bounds:?}"
    );
}

fn painted(worker: &mut Worker, output: &Mailbox) -> ViewerFrame {
    worker.paint(output).unwrap();
    output
        .take()
        .into_iter()
        .find_map(|event| match event {
            ViewerOutput::Frame(frame) => Some(*frame),
            _ => None,
        })
        .expect("a painted frame")
}

fn copied(worker: &mut Worker, output: &Mailbox, copy: ViewerInput) -> Option<String> {
    worker.input(copy, output);
    output.take().into_iter().find_map(|event| match event {
        ViewerOutput::CopyText(text) => Some(text),
        _ => None,
    })
}

/// A scrollbar position refined within the loaded page: the camera moves, the page
/// and its selection stay.
fn refine(worker: &mut Worker, output: &Mailbox, shown: &ViewerFrame, sequence: u64, at: f64) {
    worker.input(
        ViewerInput::Position {
            generation: shown.generation,
            sequence,
            horizontal: false,
            position: at,
        },
        output,
    );
    assert_eq!(
        worker.positions.acknowledged[0], sequence,
        "a loaded position"
    );
}

/// A fully loaded page with a match 40 rows down.
fn match_page(worker: &mut Worker, output: &Mailbox) -> String {
    worker.geometry.width = 600;
    let text = format!(
        "{}    α😀 target value\n{}",
        "row\n".repeat(40),
        "row\n".repeat(40)
    );
    worker.length = text.len() as u64;
    assert!(worker.set_page(page(&text, 0), output));
    text
}

fn find_target(worker: &mut Worker, output: &Mailbox, text: &str) {
    worker.search_cursor.configure(Request {
        query: "target".into(),
        sensitive: true,
        backwards: false,
        whole_words: true,
    });
    worker.found_match(range(text, "target", 0), output);
    assert!(worker.selection.is_some());
}

#[test]
fn find_copy_outlives_a_later_refinement_but_not_a_later_selection() {
    let (mut worker, output) = worker();
    let text = match_page(&mut worker, &output);
    let before = painted(&mut worker, &output);
    find_target(&mut worker, &output, &text);
    let found = painted(&mut worker, &output);
    refine(&mut worker, &output, &found, 1, 0.0);
    let refined = painted(&mut worker, &output);
    assert!(
        !worker.frames.accepts(found.revision),
        "the refinement retired the coordinates of the find frame"
    );
    let copy = |frame: &ViewerFrame| ViewerInput::Copy {
        frame: frame.revision,
    };
    assert_eq!(
        copied(&mut worker, &output, copy(&found)).as_deref(),
        Some("target")
    );
    assert_eq!(
        copied(&mut worker, &output, copy(&before)),
        None,
        "that frame showed no match"
    );

    // Ctrl+A names the shown frame, which can copy the rows before showing them.
    worker.input(
        ViewerInput::SelectPage {
            frame: refined.revision,
        },
        &output,
    );
    let rows = copied(&mut worker, &output, copy(&refined)).unwrap();
    assert!(rows.starts_with("row\n") && !rows.contains("target"));
    assert_eq!(
        copied(&mut worker, &output, copy(&found)),
        None,
        "it showed the match, not the rows"
    );
    let selected = painted(&mut worker, &output);
    refine(&mut worker, &output, &selected, 2, 1.0);
    painted(&mut worker, &output);
    assert_eq!(
        copied(&mut worker, &output, copy(&selected)).as_deref(),
        Some(rows.as_str())
    );
    assert_eq!(
        copied(&mut worker, &output, copy(&refined)),
        None,
        "once its camera moved, only frames that showed the rows copy them"
    );
}

#[test]
fn retained_find_copy_names_a_replaced_surface_that_showed_the_match() {
    let (mut worker, output) = worker();
    worker.input(ViewerInput::RetainedRaster(true), &output);
    let text = match_page(&mut worker, &output);
    // Retained frames share a bounded snapshot budget: keep identities, not pixels.
    let surface = |frame: ViewerFrame| frame.raster.expect("a retained frame").surface;
    let copy = |surface| ViewerInput::Raster {
        surface,
        input: ViewerRasterInput::Copy,
    };
    let before = surface(painted(&mut worker, &output));
    find_target(&mut worker, &output, &text);
    let found = painted(&mut worker, &output);
    refine(&mut worker, &output, &found, 1, 0.0);
    let found = surface(found);
    let refined = painted(&mut worker, &output);
    let info = refined.raster.clone().unwrap();
    assert_ne!(info.surface, found, "the refinement replaced the surface");
    assert_eq!(
        copied(&mut worker, &output, copy(found)).as_deref(),
        Some("target")
    );
    assert_eq!(
        copied(&mut worker, &output, copy(before)),
        None,
        "that surface showed no match"
    );

    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::SelectPage {
                pixel_id: info.pixel_id,
                origin: info.origin,
            },
        },
        &output,
    );
    drop(info);
    let rows = copied(&mut worker, &output, copy(surface(refined))).unwrap();
    assert!(rows.starts_with("row\n") && !rows.contains("target"));
    assert_eq!(
        copied(&mut worker, &output, copy(found)),
        None,
        "it showed the match, not the rows"
    );
    let selected = painted(&mut worker, &output);
    refine(&mut worker, &output, &selected, 2, 1.0);
    let selected = surface(selected);
    let moved = surface(painted(&mut worker, &output));
    assert_ne!(moved, selected);
    assert_eq!(
        copied(&mut worker, &output, copy(moved)).as_deref(),
        Some(rows.as_str())
    );
    assert_eq!(
        copied(&mut worker, &output, copy(selected)),
        None,
        "an earlier frame of that surface showed the match"
    );
}

#[test]
fn a_page_that_selects_the_same_source_keeps_earlier_find_frames_copyable() {
    let (mut worker, output) = worker();
    let text = match_page(&mut worker, &output);
    find_target(&mut worker, &output, &text);
    let found = painted(&mut worker, &output);
    // A reloaded page that starts one row before the match still selects its source.
    let reloaded = 4 * 39;
    assert!(worker.set_page(page(&text[reloaded..], reloaded as u64), &output));
    assert_ne!(worker.selection, None);
    painted(&mut worker, &output);
    assert!(!worker.frames.accepts(found.revision));
    assert_eq!(
        copied(
            &mut worker,
            &output,
            ViewerInput::Copy {
                frame: found.revision
            }
        )
        .as_deref(),
        Some("target")
    );
    // A page without the match selects nothing: earlier frames showed something else.
    assert!(worker.set_page(page(&text[..reloaded], 0), &output));
    assert_eq!(worker.selection, None);
    assert!(!worker.frames.accepts_copy(found.revision));
}

/// Rows around one long row with a match. A fragment of that row alone is shorter
/// than the window, which the retained path cannot refill: its frames carry no raster.
fn long_row_file() -> (String, std::ops::Range<usize>) {
    let rows = "row\n".repeat(40);
    let long = format!("{} target {}\n", "x".repeat(300), "x".repeat(100));
    let text = format!("{rows}{long}{rows}");
    (text, rows.len()..rows.len() + long.len())
}

fn fragment(text: &str, range: std::ops::Range<usize>) -> Page {
    let mut fragment = page(&text[range.clone()], range.start as u64);
    fragment.after = range.end < text.len();
    fragment
}

#[test]
fn fallback_frames_with_retained_raster_enabled_copy_the_shown_find_result() {
    let (mut worker, output) = worker();
    worker.input(ViewerInput::RetainedRaster(true), &output);
    let (text, long_row) = long_row_file();
    worker.length = text.len() as u64;
    assert!(worker.set_page(fragment(&text, long_row), &output));
    find_target(&mut worker, &output, &text);
    let found = painted(&mut worker, &output);
    assert!(worker.raster.enabled && !worker.retained_active());
    assert!(found.raster.is_none(), "a frame without retained raster");
    assert_eq!(
        copied(
            &mut worker,
            &output,
            ViewerInput::Copy {
                frame: found.revision
            }
        )
        .as_deref(),
        Some("target")
    );
}

#[test]
fn a_stale_retained_surface_copies_the_shown_selection_after_the_page_falls_back() {
    let (mut worker, output) = worker();
    worker.input(ViewerInput::RetainedRaster(true), &output);
    let (text, long_row) = long_row_file();
    worker.length = text.len() as u64;
    assert!(worker.set_page(fragment(&text, 0..long_row.end + 80), &output));
    find_target(&mut worker, &output, &text);
    let surface = painted(&mut worker, &output)
        .raster
        .expect("a retained frame")
        .surface;
    // A seek shows the long row alone; the UI still presents the retained surface.
    assert!(worker.set_page(fragment(&text, long_row), &output));
    assert!(!worker.retained_active());
    let copy = || ViewerInput::Raster {
        surface,
        input: ViewerRasterInput::Copy,
    };
    assert_eq!(
        copied(&mut worker, &output, copy()).as_deref(),
        Some("target")
    );
    // Ctrl+A on the frame without raster: that surface showed another selection.
    let shown = painted(&mut worker, &output);
    assert!(shown.raster.is_none());
    worker.input(
        ViewerInput::SelectPage {
            frame: shown.revision,
        },
        &output,
    );
    assert_eq!(copied(&mut worker, &output, copy()), None);
}

#[test]
fn a_rejected_retained_selection_copies_nothing_even_when_a_frame_names_it() {
    let (mut worker, output) = worker();
    worker.input(ViewerInput::RetainedRaster(true), &output);
    let text = match_page(&mut worker, &output);
    find_target(&mut worker, &output, &text);
    let info = painted(&mut worker, &output)
        .raster
        .expect("a retained frame");
    // A selection the worker cannot place invalidates the retained Copy. The match
    // stays shown, but Copy must not fall back to it.
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::Select {
                pixel_id: info.pixel_id,
                origin: info.origin,
                x: f32::NAN,
                y: 0.0,
                extend: false,
            },
        },
        &output,
    );
    assert!(worker.selection.is_some() && worker.retained_active());
    let shown = painted(&mut worker, &output);
    assert!(shown.raster.is_some());
    assert_eq!(
        copied(
            &mut worker,
            &output,
            ViewerInput::Copy {
                frame: shown.revision
            }
        ),
        None
    );
}

#[test]
fn synthetic_or_ambiguous_word_anchors_fall_back_without_claiming_a_local_match() {
    let request = Request {
        query: "cat".into(),
        sensitive: true,
        backwards: false,
        whole_words: true,
    };
    for zero_length in [false, true] {
        let mut page = page("bobcat cat", 0);
        for anchor in &mut page.anchors {
            if anchor.text_offset >= 7 {
                anchor.source_offset = if zero_length {
                    7
                } else {
                    u64::from(anchor.text_offset - 1)
                };
            }
        }
        page.end = page.anchors.last().unwrap().source_offset;
        assert!(
            local_words(&page, &request, 3).is_none(),
            "synthetic mapping became a source match"
        );
    }
}

#[test]
fn rtl_match_wider_than_viewport_centers_its_actual_shaped_horizontal_extent() {
    let (mut worker, output) = worker();
    worker.geometry.width = 80;
    let text = "שלום".repeat(12);
    worker.length = text.len() as u64;
    worker.set_page(page(&text, 0), &output);
    let shaped = worker.shaped.as_ref().unwrap();
    let first = shaped.caret_geometry(0).unwrap();
    let last = shaped.caret_geometry(text.len()).unwrap();
    assert!(
        first.x0 > last.x0,
        "fixture must shape right to left: {first:?}, {last:?}"
    );
    assert!(
        first.x0 > f64::from(worker.geometry.width) && last.x1 < f64::from(worker.geometry.width),
        "RTL match must cross the viewport edge: {first:?}, {last:?}"
    );
    worker.search_cursor.configure(Request {
        query: text.clone(),
        sensitive: true,
        backwards: false,
        whole_words: false,
    });
    worker.found_match(
        viewer::SourceRange {
            offset: 0,
            length: text.len() as u64,
        },
        &output,
    );
    let center = ((first.x0.min(last.x0) + first.x1.max(last.x1)) / 2.0) as f32 - worker.origin[0];
    assert!(
        (center - worker.geometry.width as f32 / 2.0).abs() < 1.0,
        "RTL highlight must center its visual extent, origin={:?}",
        worker.origin
    );
}

#[test]
fn binary_find_defensively_ignores_word_policy_and_selects_literal_source_bytes() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_BINARY;
    worker.binary = true;
    worker.length = 7;
    worker.set_page(
        Page::binary(
            viewer::BinaryPage {
                revision: 1,
                source: viewer::SourceRange {
                    offset: 0,
                    length: 7,
                },
                has_before: false,
                has_after: false,
            },
            b"cat dog",
        ),
        &output,
    );
    worker.start_find("cat".into(), false, true, true, &output);
    assert!(!worker.search_cursor.request.as_ref().unwrap().whole_words);
    worker.found_match(
        viewer::SourceRange {
            offset: 0,
            length: 3,
        },
        &output,
    );
    let selected = worker
        .selection
        .expect("literal binary result must remain selectable");
    assert_eq!(
        worker.page.as_ref().unwrap().text[selected.0..selected.1].trim(),
        "63 61 74"
    );
}

#[test]
fn clearing_find_preserves_a_delayed_initial_viewport_until_it_is_published() {
    let (mut worker, output) = worker();
    worker.viewport_request = Some(41);
    worker.start_find(String::new(), false, false, false, &output);
    assert_eq!(worker.viewport_request, Some(41));
    worker.event(
        41,
        EventKind::TextPage(super::super::tests::decoded_page("initial content")),
        &output,
    );
    worker.event(41, EventKind::Completed, &output);
    assert_eq!(&*worker.page.as_ref().unwrap().text, "initial content");
    assert!(worker.viewport_request.is_none());
    worker.paint(&output).unwrap();
    assert!(output.take().into_iter().any(|event| matches!(event, ViewerOutput::Frame(frame) if &*frame.accessible_text == "initial content")));
}

#[test]
fn no_match_and_clear_preserve_a_delayed_logical_seek_and_its_acknowledgement() {
    for clear in [false, true] {
        let (mut worker, output) = worker();
        worker.length = 20_000;
        worker.set_page(page("old view", 0), &output);
        worker.viewport_request = Some(41);
        worker.positions.requested(41, 7);
        let generation = worker.positions.generation;
        worker.start_find(
            if clear {
                String::new()
            } else {
                "missing".into()
            },
            false,
            false,
            false,
            &output,
        );
        assert_eq!(worker.viewport_request, Some(41));
        assert!(worker.positions.has_pending());
        assert_eq!(worker.positions.generation, generation);
        if !clear {
            worker.search_request = Some(42);
            worker.status = SEARCHING.into();
            worker.event(42, EventKind::Completed, &output);
            assert!(worker.status.contains("No further match"));
        }
        worker.event(
            41,
            EventKind::TextPage(super::super::tests::decoded_page("seek destination")),
            &output,
        );
        assert_eq!(
            &*worker.page.as_ref().unwrap().text,
            "old view",
            "seek page cannot publish before terminal"
        );
        worker.event(41, EventKind::Completed, &output);
        assert_eq!(&*worker.page.as_ref().unwrap().text, "seek destination");
        assert!(!worker.positions.has_pending());
        assert_eq!(worker.positions.acknowledged[0], 7);
        assert!(worker.viewport_request.is_none());
    }
}

#[test]
fn result_during_initial_load_waits_for_that_viewport_before_selecting_match() {
    let (mut worker, output) = worker();
    worker.length = 9000;
    worker.viewport_request = Some(41);
    worker.start_find("target".into(), false, true, false, &output);
    worker.search_request = Some(42);
    worker.event(
        42,
        EventKind::SearchMatch(messages::SearchMatch {
            source: viewer::SourceRange {
                offset: 8000,
                length: 6,
            },
            preview: "target".into(),
        }),
        &output,
    );
    assert_eq!(worker.viewport_request, Some(41));
    assert!(worker.search_cursor.waiting.is_some());
    assert!(worker.selection.is_none());
    worker.event(
        41,
        EventKind::TextPage(super::super::tests::decoded_page("target")),
        &output,
    );
    worker.event(41, EventKind::Completed, &output);
    assert!(worker.search_cursor.waiting.is_none());
    assert!(worker.selection.is_some());
    assert_eq!(worker.found.as_ref().unwrap().offset, 8000);
}

#[test]
fn unrelated_viewport_publication_preserves_an_inflight_search_status_until_exhaustion() {
    let (mut worker, output) = worker();
    worker.viewport_request = Some(41);
    worker.start_find("missing".into(), false, false, false, &output);
    worker.search_request = Some(42);
    worker.status = SEARCHING.into();
    worker.event(
        41,
        EventKind::TextPage(super::super::tests::decoded_page("initial view")),
        &output,
    );
    worker.event(41, EventKind::Completed, &output);
    assert_eq!(&*worker.page.as_ref().unwrap().text, "initial view");
    assert_eq!(worker.status, SEARCHING);
    worker.event(42, EventKind::Completed, &output);
    assert!(worker.status.contains("No further match"));
}
