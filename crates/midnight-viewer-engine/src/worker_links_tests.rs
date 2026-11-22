//! Links of a shown Markdown page (`worker_links.rs`): hover, click, keyboard focus,
//! fragments, local targets and the link history, with a synthetic document page.
use super::*;
use midnight_core::ViewerRasterInfo;
use midnight_plugin_host::api::viewer::{Anchor, SourceRange, TextPage};

/// A block of `text` at source offset `source`, with `links` as (link text, target), and
/// exact anchors at its ends and at each link's ends.
fn block(kind: u16, level: u8, source: u64, text: &str, links: &[(&str, &str)]) -> document::Block {
    let mut offsets = vec![0, text.len()];
    let links: Vec<_> = links
        .iter()
        .map(|(shown, target)| {
            let start = text.find(shown).expect("link text in block");
            offsets.extend([start, start + shown.len()]);
            document::Link {
                start: start as u32,
                end: (start + shown.len()) as u32,
                target: (*target).into(),
            }
        })
        .collect();
    offsets.sort_unstable();
    offsets.dedup();
    document::Block {
        kind,
        depth: 0,
        heading_level: level,
        links,
        text: TextPage {
            revision: 1,
            source: SourceRange {
                offset: source,
                length: text.len() as u64,
            },
            text: text.into(),
            spans: vec![],
            anchors: offsets
                .into_iter()
                .map(|offset| Anchor {
                    text_offset: offset as u32,
                    source_offset: source + offset as u64,
                })
                .collect(),
            has_before: false,
            has_after: false,
            partial_line: false,
        },
        table: None,
        strikethrough: None,
    }
}

fn document(
    blocks: Vec<document::Block>,
    offset: u64,
    length: u64,
    after: bool,
) -> document::DocumentPage {
    document::DocumentPage {
        revision: 1,
        source: SourceRange { offset, length },
        blocks,
        has_before: offset > 0,
        has_after: after,
    }
}

fn links_page() -> document::DocumentPage {
    document(
        vec![
            block(document::BLOCK_HEADING, 1, 0, "Links", &[]),
            block(
                document::BLOCK_PARAGRAPH,
                0,
                100,
                "Read the guide, write mail, open notes, see second, run tool.",
                &[
                    ("guide", "https://example.com/guide"),
                    ("mail", "mailto:a@example.com?attach=/etc/passwd"),
                    ("notes", "notes.md"),
                    ("second", "#second-heading"),
                    ("tool", "javascript:alert(1)"),
                ],
            ),
            block(document::BLOCK_HEADING, 2, 300, "Second heading", &[]),
            block(
                document::BLOCK_PARAGRAPH,
                0,
                400,
                "Plain text without links.",
                &[],
            ),
        ],
        0,
        500,
        false,
    )
}

/// A Markdown document shown with retained raster, from a file in `folder`.
fn shown(folder: &Path, page: document::DocumentPage) -> (Worker, Mailbox) {
    let (mut worker, output) = crate::worker::tests::worker();
    worker.files = Arc::from([folder.join("a.md")]);
    worker.mode = viewer::MODE_DOCUMENT;
    worker.length = 500;
    worker.input(ViewerInput::RetainedRaster(true), &output);
    let page = Page::document(page, &worker.appearance);
    assert!(worker.set_page(page, &output));
    (worker, output)
}

fn paint(worker: &mut Worker, output: &Mailbox) -> ViewerFrame {
    worker.paint(output).unwrap();
    output
        .take()
        .into_iter()
        .find_map(|message| match message {
            ViewerOutput::Frame(frame) => Some(*frame),
            _ => None,
        })
        .expect("a frame")
}

/// A retained frame's body pixels: its prepared strips in order, or its own pixels.
fn body(frame: &ViewerFrame) -> Vec<u8> {
    let info = frame.raster.as_ref().expect("retained pixels");
    match &info.bands {
        Some(bands) => bands
            .list
            .iter()
            .flat_map(|band| band.pixels.iter().copied())
            .collect(),
        None => frame.pixels.to_vec(),
    }
}

/// The focus ring painted over a blank raster of link `index`'s first line: how many
/// pixels it colours around the line, and how many inside it.
fn ring_pixels(worker: &Worker, index: usize) -> (usize, usize) {
    let range = worker.page.as_ref().unwrap().links.links[index]
        .range
        .clone();
    let rect = worker
        .shaped
        .as_ref()
        .unwrap()
        .selection_geometry(range)
        .unwrap()[0];
    let (width, height) = (400, 200);
    let origin = [(rect.x0 - 20.0) as f32, (rect.y0 - 20.0) as f32];
    let mut pixels = vec![0; width * height * 4];
    worker.paint_link_focus(&mut pixels, width as u32, height as u32, origin);
    let inside = |x: usize, y: usize| {
        let (x, y) = (
            x as f64 + f64::from(origin[0]),
            y as f64 + f64::from(origin[1]),
        );
        x >= rect.x0 && x < rect.x1 && y >= rect.y0 && y < rect.y1
    };
    let mut counts = (0, 0);
    for y in 0..height {
        for x in 0..width {
            if pixels[(y * width + x) * 4 + 3] != 0 {
                if inside(x, y) {
                    counts.1 += 1;
                } else {
                    counts.0 += 1;
                }
            }
        }
    }
    counts
}

/// The center of link `index`'s first rectangle, in page layout coordinates.
fn center(worker: &Worker, index: usize) -> [f64; 2] {
    let range = worker.page.as_ref().unwrap().links.links[index]
        .range
        .clone();
    let rects = worker
        .shaped
        .as_ref()
        .unwrap()
        .selection_geometry(range)
        .unwrap();
    let rect = rects[0];
    [(rect.x0 + rect.x1) / 2.0, (rect.y0 + rect.y1) / 2.0]
}

/// Raster input coordinates of a layout point, for the displayed camera of `info`.
fn input_at(worker: &Worker, info: &ViewerRasterInfo, point: [f64; 2]) -> (f32, f32) {
    (
        (point[0] - info.origin[0]) as f32 + worker.text_margin(),
        (point[1] - info.origin[1]) as f32,
    )
}

fn hover(
    worker: &mut Worker,
    output: &Mailbox,
    info: &ViewerRasterInfo,
    point: [f64; 2],
) -> Option<String> {
    let (x, y) = input_at(worker, info, point);
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::Hover {
                pixel_id: info.pixel_id,
                origin: info.origin,
                x,
                y,
            },
        },
        output,
    );
    output
        .take()
        .into_iter()
        .find_map(|message| match message {
            ViewerOutput::LinkHover(status) => Some(status),
            _ => None,
        })
        .expect("every hover is answered")
}

fn press(
    worker: &mut Worker,
    output: &Mailbox,
    info: &ViewerRasterInfo,
    point: [f64; 2],
    extend: bool,
) {
    let (x, y) = input_at(worker, info, point);
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::Select {
                pixel_id: info.pixel_id,
                origin: info.origin,
                x,
                y,
                extend,
            },
        },
        output,
    );
}

fn release(worker: &mut Worker, output: &Mailbox, info: &ViewerRasterInfo, point: [f64; 2]) {
    let (x, y) = input_at(worker, info, point);
    worker.input(
        ViewerInput::Raster {
            surface: info.surface,
            input: ViewerRasterInput::Release {
                pixel_id: info.pixel_id,
                origin: info.origin,
                x,
                y,
            },
        },
        output,
    );
}

fn requests(output: &Mailbox) -> Vec<ViewerLinkRequest> {
    output
        .take()
        .into_iter()
        .filter_map(|message| match message {
            ViewerOutput::LinkRequest(request) => Some(request),
            _ => None,
        })
        .collect()
}

#[test]
fn hovering_a_link_names_where_it_leads_and_leaving_it_clears_the_status() {
    let folder = tempfile::tempdir().unwrap();
    let (mut worker, output) = shown(folder.path(), links_page());
    let frame = paint(&mut worker, &output);
    let info = frame.raster.as_ref().unwrap();
    let [guide, notes_link, tool] = [0, 2, 4].map(|index| center(&worker, index));
    assert_eq!(
        hover(&mut worker, &output, info, guide).as_deref(),
        Some("Link to https://example.com/guide")
    );
    let notes = folder.path().join("notes.md");
    assert_eq!(
        hover(&mut worker, &output, info, notes_link),
        Some(format!("Link to {}", notes.display()))
    );
    assert_eq!(
        hover(&mut worker, &output, info, tool).as_deref(),
        Some("Link to javascript:alert(1) · Links to javascript: addresses are not followed.")
    );
    // Plain text, and the pointer outside the text, are no link.
    let mut plain = guide;
    plain[0] = 1.0;
    assert_eq!(hover(&mut worker, &output, info, plain), None);
    assert_eq!(hover(&mut worker, &output, info, [f64::NAN, 0.0]), None);
}

/// The focus ring paints a frame just inside its rectangle, clipped to the raster, and
/// looks only at the pixels it paints: a link line far larger than a band costs no more
/// than the band's part of its edges.
#[test]
fn the_focus_ring_paints_only_its_edges_within_the_band() {
    let (width, height, ring, color) = (64_u32, 16_u32, 2, [1, 2, 3, 255]);
    let size = width as usize * height as usize * 4;
    for edges in [
        [4, 2, 30, 12],
        [-10, 5, 40, 100],
        [10, 3, 12, 5],
        [20, -1, 23, 30],
        [-10, -10, 100, 100],
        [-3000, -3000, 3000, 3000],
        [70, 0, 90, 10],
    ] {
        let mut pixels = vec![0; size];
        let visited = paint_ring(&mut pixels, [width, height], edges, ring, color);
        let [left, top, right, bottom] = edges;
        let mut expected = vec![0; size];
        for y in 0..i64::from(height) {
            for x in 0..i64::from(width) {
                let inside = x >= left && x < right && y >= top && y < bottom;
                let hole =
                    x >= left + ring && x < right - ring && y >= top + ring && y < bottom - ring;
                if inside && !hole {
                    let at = (y * i64::from(width) + x) as usize * 4;
                    expected[at..at + 4].copy_from_slice(&color);
                }
            }
        }
        assert!(pixels == expected, "{edges:?}");
        let painted = expected.chunks(4).filter(|pixel| pixel[3] != 0).count();
        assert_eq!(
            visited, painted,
            "{edges:?}: only painted pixels are looked at"
        );
    }
}

/// The window asks its next hover question only after an answer, so a hover that no shown
/// text reads is answered with no link, and the next one is read again.
#[test]
fn a_hover_no_shown_text_reads_is_answered_with_no_link() {
    let folder = tempfile::tempdir().unwrap();
    let (mut worker, output) = shown(folder.path(), links_page());
    let frame = paint(&mut worker, &output);
    let info = frame.raster.as_ref().unwrap();
    let guide = center(&worker, 0);
    let link = Some("Link to https://example.com/guide".to_owned());
    // While the layout handshake waits for the window.
    worker.layout = crate::layout_handshake::Handshake::new(true);
    worker.layout.select(viewer::MODE_DOCUMENT, 1).unwrap();
    assert_eq!(hover(&mut worker, &output, info, guide), None);
    worker.layout.invalidate();
    assert_eq!(hover(&mut worker, &output, info, guide), link);
    // While an image or a video is shown.
    worker.image = Some(crate::image_view::ImageView::new(
        crate::image_worker::ImageWorker::opened_for_test(
            vec![midnight_image_viewer_host::Metadata {
                width: 40,
                height: 60,
            }],
            false,
        ),
        worker.image_ids.clone(),
    ));
    assert_eq!(hover(&mut worker, &output, info, guide), None);
    worker.image = None;
    worker.mode = VIEWER_MODE_VIDEO;
    assert_eq!(hover(&mut worker, &output, info, guide), None);
    worker.mode = viewer::MODE_DOCUMENT;
    assert_eq!(hover(&mut worker, &output, info, guide), link);
}

#[test]
fn a_click_follows_a_link_and_a_drag_from_it_only_selects() {
    let folder = tempfile::tempdir().unwrap();
    let (mut worker, output) = shown(folder.path(), links_page());
    let frame = paint(&mut worker, &output);
    let info = frame.raster.as_ref().unwrap();
    let guide = center(&worker, 0);
    press(&mut worker, &output, info, guide, false);
    release(&mut worker, &output, info, guide);
    let [ViewerLinkRequest::OpenExternal(url)] = &requests(&output)[..] else {
        panic!("the click opens the web link");
    };
    assert_eq!(url.as_str(), "https://example.com/guide");
    assert_eq!(worker.status, "Opening in your browser…");
    // Pressed on the link, dragged over the next one, released there: a selection.
    let mail = center(&worker, 1);
    press(&mut worker, &output, info, guide, false);
    press(&mut worker, &output, info, mail, true);
    release(&mut worker, &output, info, mail);
    assert!(requests(&output).is_empty());
    assert!(worker.selection.is_some_and(|(start, end)| start != end));
    // A press on plain text released on a link follows nothing either.
    let mut plain = guide;
    plain[0] = 1.0;
    press(&mut worker, &output, info, plain, false);
    release(&mut worker, &output, info, guide);
    assert!(requests(&output).is_empty());
}

#[test]
fn mail_links_lose_attachments_and_other_schemes_are_refused_by_name() {
    let folder = tempfile::tempdir().unwrap();
    let (mut worker, output) = shown(folder.path(), links_page());
    output.take();
    worker.follow_link("mailto:a@example.com?attach=/etc/passwd", &output);
    let [ViewerLinkRequest::OpenExternal(url)] = &requests(&output)[..] else {
        panic!("the mail link opens");
    };
    assert_eq!(url.as_str(), "mailto:a@example.com");
    assert!(url.attachments_removed());
    assert_eq!(worker.status, "Opening in your mail program…");
    for (target, status) in [
        (
            "javascript:alert(1)",
            "Links to javascript: addresses are not followed.",
        ),
        (
            "data:text/html,x",
            "Links to data: addresses are not followed.",
        ),
        (
            "file://server/share/x.md",
            "Links to files on other computers are not followed.",
        ),
    ] {
        worker.follow_link(target, &output);
        assert_eq!(worker.status, status, "{target}");
        assert!(requests(&output).is_empty(), "{target}");
    }
}

#[test]
fn tab_rings_the_links_in_turn_shift_tab_leaves_them_and_enter_follows_the_focused_one() {
    let folder = tempfile::tempdir().unwrap();
    let (mut worker, output) = shown(folder.path(), links_page());
    let plain = paint(&mut worker, &output);
    let focus = |output: &Mailbox| {
        output.take().into_iter().find_map(|message| match message {
            ViewerOutput::LinkFocus { count, focused } => Some((count, focused)),
            _ => None,
        })
    };
    worker.input(ViewerInput::Link(ViewerLinkInput::Next), &output);
    assert_eq!(focus(&output), Some((5, Some(0))));
    assert_eq!(worker.status, "Link to https://example.com/guide");
    let ringed = paint(&mut worker, &output);
    assert_ne!(body(&plain), body(&ringed), "the focused link has a ring");
    let ring = ring_pixels(&worker, 0);
    assert!(
        ring.0 > 0 && ring.1 == 0,
        "around the link, not over its text: {ring:?}"
    );
    worker.input(ViewerInput::Link(ViewerLinkInput::Next), &output);
    assert_eq!(focus(&output), Some((5, Some(1))));
    worker.input(ViewerInput::Link(ViewerLinkInput::Follow), &output);
    let [ViewerLinkRequest::OpenExternal(url)] = &requests(&output)[..] else {
        panic!("Enter follows the focused mail link");
    };
    assert_eq!(url.as_str(), "mailto:a@example.com");
    for expected in [Some(0), None] {
        worker.input(ViewerInput::Link(ViewerLinkInput::Previous), &output);
        assert_eq!(focus(&output), Some((5, expected)));
    }
    assert_eq!(
        ring_pixels(&worker, 0),
        (0, 0),
        "the ring goes with the focus"
    );
    // Enter without a focused link follows nothing.
    worker.input(ViewerInput::Link(ViewerLinkInput::Follow), &output);
    assert!(requests(&output).is_empty());
}

#[test]
fn a_fragment_scrolls_to_its_github_heading_on_the_shown_page() {
    let folder = tempfile::tempdir().unwrap();
    let (mut worker, output) = shown(folder.path(), links_page());
    crate::worker::unsent::take();
    worker.follow_link("#second-heading", &output);
    let requested: Vec<_> = crate::worker::unsent::take()
        .iter()
        .map(|viewport| viewport.source_anchor)
        .collect();
    assert_eq!(
        requested,
        [300],
        "the heading's source goes to the top of the view"
    );
    worker.follow_link("#", &output);
    let requested: Vec<_> = crate::worker::unsent::take()
        .iter()
        .map(|viewport| viewport.source_anchor)
        .collect();
    assert_eq!(requested, [0], "an empty fragment is the top");
    worker.follow_link("#no-such-heading", &output);
    assert!(crate::worker::unsent::take().is_empty());
    assert_eq!(
        worker.status,
        "This document has no heading #no-such-heading."
    );
}

/// A fragment's percent-decoded controls do not reach the status line: they show as the
/// hover text shows them.
#[test]
fn decoded_controls_of_a_fragment_do_not_reach_the_status_line() {
    let folder = tempfile::tempdir().unwrap();
    let (mut worker, output) = shown(folder.path(), links_page());
    worker.follow_link("#a%0Ab%1B%E2%80%AEc", &output);
    assert_eq!(
        worker.status,
        "This document has no heading #a\u{fffd}b\u{fffd}\\u202Ec."
    );
    worker.mode = viewer::MODE_TEXT;
    worker.follow_link("#x%0Dy", &output);
    assert_eq!(
        worker.status,
        "This view has no headings, so #x\u{fffd}y is not shown."
    );
}

#[test]
fn a_heading_beyond_the_shown_page_is_found_by_a_bounded_scan_that_shows_progress() {
    let folder = tempfile::tempdir().unwrap();
    // The shown page begins the document but does not end it.
    let mut first = links_page();
    first.has_after = true;
    let (mut worker, output) = shown(folder.path(), first);
    worker.length = 100_000;
    crate::worker::unsent::take();
    crate::worker::unsent::admit_from(Some(70));
    worker.follow_link("#usage-1", &output);
    let scans: Vec<_> = crate::worker::unsent::take()
        .iter()
        .map(|viewport| viewport.source_anchor)
        .collect();
    assert_eq!(scans, [500], "the scan continues after the shown page");
    assert!(worker.links.scanning(70));
    // Another page without it: a slow scan says how far it got.
    worker.links.scan.as_mut().unwrap().started = Instant::now() - Duration::from_secs(1);
    worker.scanned_page(
        document(
            vec![block(document::BLOCK_HEADING, 2, 600, "Usage", &[])],
            500,
            9_500,
            true,
        ),
        &output,
    );
    assert!(
        worker
            .status
            .starts_with("Looking for #usage-1 in the document… "),
        "{}",
        worker.status
    );
    assert_eq!(
        crate::worker::unsent::take()
            .iter()
            .map(|viewport| viewport.source_anchor)
            .collect::<Vec<_>>(),
        [10_000]
    );
    // The second "Usage" heading is usage-1, counted across pages.
    worker.scanned_page(
        document(
            vec![block(document::BLOCK_HEADING, 2, 12_000, "Usage", &[])],
            10_000,
            5_000,
            true,
        ),
        &output,
    );
    assert!(worker.links.scan.is_none());
    assert_eq!(
        crate::worker::unsent::take()
            .iter()
            .map(|viewport| viewport.source_anchor)
            .collect::<Vec<_>>(),
        [12_000]
    );
    // Any other action ends a scan, and its bound ends one that finds nothing.
    worker.follow_link("#missing", &output);
    assert!(worker.links.scan.is_some());
    worker.input(ViewerInput::Scroll { x: 0.0, y: 40.0 }, &output);
    assert!(worker.links.scan.is_none(), "scrolling cancels the scan");
    let scan = Scan {
        request: 0,
        fragment: "missing".into(),
        slugs: Slugger::default(),
        pages: SCAN_PAGES,
        started: Instant::now(),
    };
    worker.request_scan(scan, 1_000, &output);
    assert!(
        worker
            .status
            .contains("was not found in the first 256 pages"),
        "{}",
        worker.status
    );
    crate::worker::unsent::admit_from(None);
}

/// A heading near the end of a large document (`#the-end` about 290 KB into a Markdown
/// file) is found by the scan: each page request is one the plugin protocol carries, and
/// the view goes to the heading.
#[test]
fn a_heading_near_the_end_of_a_large_document_is_found_and_shown() {
    const LENGTH: u64 = 295_000;
    const HEADING: u64 = 290_000;
    let folder = tempfile::tempdir().unwrap();
    let mut first = links_page();
    first.has_after = true;
    let (mut worker, output) = shown(folder.path(), first);
    worker.length = LENGTH;
    crate::worker::unsent::take();
    crate::worker::unsent::take_admitted();
    crate::worker::unsent::admit_from(Some(70));
    worker.follow_link("#the-end", &output);
    let mut pages = 0;
    while worker.links.scan.is_some() {
        let [(id, viewport)] = &crate::worker::unsent::take_admitted()[..] else {
            panic!("one admitted page request: {}", worker.status);
        };
        assert!(worker.links.scanning(*id));
        pages += 1;
        let from = viewport.source_anchor;
        let length = u64::from(viewport.max_bytes).min(LENGTH - from);
        let end = from + length;
        let block = if (from..end).contains(&HEADING) {
            block(document::BLOCK_HEADING, 2, HEADING, "The end", &[])
        } else {
            block(document::BLOCK_PARAGRAPH, 0, from, "Text.", &[])
        };
        crate::worker::unsent::take();
        worker.scanned_page(document(vec![block], from, length, end < LENGTH), &output);
    }
    assert!(
        !worker.status.starts_with("Cannot look for"),
        "{}",
        worker.status
    );
    assert!((2..=SCAN_PAGES).contains(&pages), "{pages} pages");
    assert_eq!(
        crate::worker::unsent::take()
            .iter()
            .map(|viewport| viewport.source_anchor)
            .collect::<Vec<_>>(),
        [HEADING],
        "the heading's source goes to the top of the view"
    );
    crate::worker::unsent::admit_from(None);
}

/// Follow `target` and wait for the look at the file system.
fn follow_local(worker: &mut Worker, output: &Mailbox, target: &str) {
    worker.follow_link(target, output);
    let deadline = Instant::now() + Duration::from_secs(10);
    while worker.links.resolving.is_some() {
        assert!(Instant::now() < deadline, "the look at {target} ends");
        std::thread::sleep(Duration::from_millis(2));
        worker.poll_links(output);
    }
}

/// An HTML page's pointer moves, the key-up of the Enter that followed a link and focus
/// changes are not actions that replace a pending look at a local target; a press is.
#[test]
fn only_what_the_user_does_ends_a_pending_look_at_a_local_target() {
    use midnight_core::viewer_web as api;
    let folder = tempfile::tempdir().unwrap();
    let (mut worker, output) = shown(folder.path(), links_page());
    let pointer = |kind| {
        api::CommandKind::Pointer(api::Pointer {
            kind,
            button: api::PointerButton::None,
            x: 5.0,
            y: 5.0,
            modifiers: Default::default(),
            click_count: 0,
        })
    };
    let enter = |pressed| {
        api::CommandKind::Key(api::Key {
            key: "Enter".into(),
            code: "Enter".into(),
            pressed,
            repeat: false,
            modifiers: Default::default(),
        })
    };
    let pending = Arc::new(Mutex::new(None));
    worker.links.resolving = Some(pending.clone());
    for input in [
        pointer(api::PointerKind::Move),
        pointer(api::PointerKind::Leave),
        enter(false),
        api::CommandKind::Focus(false),
    ] {
        let name = format!("{input:?}");
        worker.input(ViewerInput::Web { frame: 1, input }, &output);
        assert!(worker.links.resolving.is_some(), "{name}");
    }
    worker.input(
        ViewerInput::Web {
            frame: 1,
            input: enter(true),
        },
        &output,
    );
    assert!(worker.links.resolving.is_none(), "a key press is an action");
    worker.links.resolving = Some(pending);
    worker.input(
        ViewerInput::Web {
            frame: 1,
            input: pointer(api::PointerKind::Down),
        },
        &output,
    );
    assert!(
        worker.links.resolving.is_none(),
        "a pointer press is an action"
    );
}

#[test]
fn local_targets_open_in_the_window_reveal_files_no_viewer_shows_and_open_folders() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("a.md"), "# A\n").unwrap();
    std::fs::write(folder.path().join("notes.md"), "Notes\n").unwrap();
    std::fs::write(folder.path().join("tool.exe"), b"MZ\0\0program").unwrap();
    std::fs::create_dir(folder.path().join("sub")).unwrap();
    let (mut worker, output) = shown(folder.path(), links_page());
    output.take();
    follow_local(&mut worker, &output, "tool.exe");
    assert_eq!(
        requests(&output),
        [ViewerLinkRequest::Reveal(folder.path().join("tool.exe"))],
        "a program is revealed in the panel, never started"
    );
    follow_local(&mut worker, &output, "sub/");
    assert_eq!(
        requests(&output),
        [ViewerLinkRequest::OpenFolder(folder.path().join("sub"))]
    );
    follow_local(&mut worker, &output, "missing.md");
    assert!(
        worker.status.ends_with("missing.md does not exist."),
        "{}",
        worker.status
    );
    let before = worker.files.clone();
    follow_local(&mut worker, &output, "notes.md#top");
    assert_eq!(&worker.files[..], [folder.path().join("notes.md")]);
    assert!(
        requests(&output).is_empty(),
        "a file a viewer shows stays in the window"
    );
    assert!(
        matches!(&worker.links.arrival, Some(Arrival::Fragment(fragment)) if fragment == "top")
    );
    // Back and Forward through the documents opened from links; nothing beyond them.
    worker.input(ViewerInput::Link(ViewerLinkInput::Back), &output);
    assert_eq!(worker.files, before);
    assert!(matches!(worker.links.arrival, Some(Arrival::Position(0))));
    worker.input(ViewerInput::Link(ViewerLinkInput::Back), &output);
    assert_eq!(worker.files, before, "no history behind the first document");
    worker.input(ViewerInput::Link(ViewerLinkInput::Forward), &output);
    assert_eq!(&worker.files[..], [folder.path().join("notes.md")]);
    worker.input(ViewerInput::Link(ViewerLinkInput::Forward), &output);
    assert_eq!(&worker.files[..], [folder.path().join("notes.md")]);
}

#[test]
fn inside_an_archive_only_members_a_viewer_shows_below_the_document_open() {
    let folder = tempfile::tempdir().unwrap();
    let (mut worker, output) = shown(folder.path(), links_page());
    worker.links.on_disk = false;
    for (target, message) in [
        (
            "../up.md",
            "Links that lead above the document's folder inside an archive are not followed yet.",
        ),
        (
            ".",
            "Links to folders inside an archive are not followed yet.",
        ),
        (
            "/root.md",
            "Links to absolute paths are not followed from inside an archive.",
        ),
        (
            "notes.md",
            "This document cannot reach other files, so its links to files are not followed.",
        ),
    ] {
        worker.follow_link(target, &output);
        assert_eq!(worker.status, message, "{target}");
    }
}
