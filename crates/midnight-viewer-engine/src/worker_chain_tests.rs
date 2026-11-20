//! Byte-capped Text replies joined to the line budget of their request (`worker_chain.rs`).
//! Replies follow the basic viewer's selection: at most 8176 bytes, cut inside a row,
//! and ended earlier at the request's line budget.
use super::super::chain::{JOIN_BYTES, adjacent, join, trim_cut_start};
use super::text_position::edge_page;
use super::*;
use midnight_plugin_host::HostEvent;

/// The basic viewer's page budget (`READ_BYTES - 16`).
const PROVIDER_BYTES: usize = 8176;

/// Rows like those of the acceptance fixture `inventory.csv` (100 to 260 bytes, 180 on
/// average), or `width` bytes each.
fn rows(count: usize, width: Option<usize>) -> String {
    (0..count)
        .map(|row| {
            let prefix = format!("r{row:05},component-{},MIT,", row % 89);
            let width = width.unwrap_or(100 + row * 37 % 161);
            format!("{prefix}{}\n", "x".repeat(width - prefix.len() - 1))
        })
        .collect()
}

fn lines(text: &str) -> u32 {
    text.bytes().filter(|byte| *byte == b'\n').count() as u32
}

/// What the basic viewer answers to `viewport` over `source`.
fn reply_to(source: &str, viewport: &messages::Viewport) -> viewer::TextPage {
    let anchor = viewport.source_anchor as usize;
    let max_lines = viewport.max_lines;
    let (start, end) = if viewport.backwards {
        let mut start = anchor.saturating_sub(PROVIDER_BYTES);
        let mut counted = 0;
        for (index, byte) in source.as_bytes()[start..anchor].iter().enumerate().rev() {
            if *byte == b'\n' && start + index + 1 < anchor {
                counted += 1;
                if counted >= max_lines {
                    start += index + 1;
                    break;
                }
            }
        }
        (start, anchor)
    } else {
        let mut end = (anchor + PROVIDER_BYTES).min(source.len());
        let mut counted = 0;
        for (index, byte) in source.as_bytes()[anchor..end].iter().enumerate() {
            if *byte == b'\n' {
                counted += 1;
                if counted >= max_lines {
                    end = anchor + index + 1;
                    break;
                }
            }
        }
        (anchor, end)
    };
    let mut page = edge_page(source, start, end);
    // The basic viewer's flag covers either edge (`window::select`).
    page.partial_line = !(start == 0 || source[..start].ends_with('\n'))
        || (page.has_after && !page.text.ends_with('\n'));
    page
}

/// A test worker showing Text in the basic viewer's 1312 x 748 content area.
fn text_worker(source: &str) -> (Worker, Mailbox) {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_TEXT;
    worker.wrap = false;
    worker.raster.enabled = true;
    worker.length = source.len() as u64;
    worker.geometry = ViewerGeometry {
        width: 1312,
        height: 748,
        scale: 1.0,
    };
    (worker, output)
}

/// Admit `viewport` as foreground request `id`, as `request_viewport` does with a session.
fn admit(worker: &mut Worker, id: u64, viewport: &messages::Viewport) {
    worker.viewport_request = Some(id);
    worker.anchor = viewport.source_anchor;
    worker.land_at_bottom = viewport.backwards;
    worker.chain_requested(id, viewport);
    unsent::take();
}

/// The plugin's events for `id` as the worker drains them from its session.
fn reply(worker: &mut Worker, output: &Mailbox, id: u64, page: viewer::TextPage) {
    let generation = worker.generation;
    worker.session_events(
        [EventKind::TextPage(page), EventKind::Completed]
            .into_iter()
            .map(|kind| HostEvent {
                request_id: id,
                generation,
                kind,
                fatal: false,
            })
            .collect(),
        output,
    );
}

/// The one range a chain asked for since the last call, and the request to answer.
fn chained(worker: &Worker, logical: u64) -> (messages::Viewport, u64) {
    let requests = unsent::take();
    assert_eq!(requests.len(), 1, "one more range is read: {requests:?}");
    let physical = worker.chained_request(logical).expect("chain");
    assert_ne!(physical, logical);
    (requests[0].clone(), physical)
}

/// Camera travel the published page leaves around the viewport.
fn travel(worker: &Worker) -> f32 {
    let page = worker.page.as_ref().unwrap();
    let shaped = worker.shaped.as_ref().unwrap();
    let height = worker.geometry.height as f32;
    crate::viewport::available_height(page, shaped, height) - height
}

/// A forward page of 8 KiB of these rows is about as tall as the viewport: it left a few
/// pixels of travel, no room for the retained guards, and every arrow press needed a
/// refill that could not pass the page's edge. The reply now waits for the range after
/// it, and one page holding the request's whole line budget is published.
#[test]
fn a_byte_capped_forward_reply_is_joined_to_its_line_budget() {
    let source = rows(1500, None);
    let (mut worker, output) = text_worker(&source);
    let viewport = worker.page_viewport(0, false);
    assert!(viewport.max_lines > 60, "{viewport:?}");
    admit(&mut worker, 91, &viewport);
    let first = reply_to(&source, &viewport);
    assert!(lines(&first.text) < 50 && first.has_after, "cut by bytes");
    reply(&mut worker, &output, 91, first.clone());
    assert!(worker.page.is_none(), "the short reply is not shown alone");
    assert_eq!(worker.viewport_request, Some(91));
    let (next, physical) = chained(&worker, 91);
    assert_eq!(
        (next.source_anchor, next.backwards, next.max_lines),
        (
            first.source.offset + first.source.length,
            false,
            viewport.max_lines - lines(&first.text)
        )
    );
    reply(&mut worker, &output, physical, reply_to(&source, &next));
    let page = worker.page.as_ref().expect("joined page");
    assert_eq!(page.start, 0);
    assert_eq!(lines(&page.text), viewport.max_lines);
    assert!(
        page.text.ends_with('\n'),
        "the line budget ends it after a whole row"
    );
    assert_eq!(page.anchors.last().unwrap().source_offset, page.end);
    assert!(worker.viewport_request.is_none(), "the request completed");
    let guard = super::super::raster::guard(worker.geometry, false)[1] as f32;
    assert!(
        travel(&worker) >= 2.0 * guard,
        "{} px of travel",
        travel(&worker)
    );
    assert!(unsent::take().is_empty());
}

/// A backward page of 8 KiB started inside a row and showed that row's tail as a line of
/// its own at the top. The range before it is read too, and the joined page starts at a
/// row: the first row of the range read last is dropped, as it ends inside a row and its
/// edge flag cannot tell whether that row is whole.
#[test]
fn a_byte_capped_backward_reply_is_joined_and_starts_at_a_row() {
    let source = rows(1500, None);
    let anchor = source[..source.len() / 2].rfind('\n').unwrap() + 1;
    let (mut worker, output) = text_worker(&source);
    let viewport = worker.page_viewport(anchor as u64, true);
    admit(&mut worker, 91, &viewport);
    let first = reply_to(&source, &viewport);
    let cut = first.source.offset as usize;
    assert!(
        !source[..cut].ends_with('\n'),
        "the byte budget ends inside a row"
    );
    reply(&mut worker, &output, 91, first.clone());
    assert!(worker.page.is_none());
    let (next, physical) = chained(&worker, 91);
    assert_eq!(
        (next.source_anchor, next.backwards),
        (first.source.offset, true)
    );
    reply(&mut worker, &output, physical, reply_to(&source, &next));
    let page = worker.page.as_ref().expect("joined page");
    assert_eq!(page.end, anchor as u64);
    assert!(
        source[..page.start as usize].ends_with('\n'),
        "a whole first row"
    );
    assert!(page.text.starts_with('r') && !page.partial_line);
    // As a provider's backward page, one newline less than the budget (the last row runs
    // up to the anchor), and its first row dropped.
    assert_eq!(lines(&page.text), viewport.max_lines - 2);
    let guard = super::super::raster::guard(worker.geometry, false)[1] as f32;
    assert!(travel(&worker) >= 2.0 * guard);
    assert!(unsent::take().is_empty(), "no further read");
}

/// Rows longer than the provider's budget fill a joined page before its line budget; the
/// last backward read then starts inside a row, which is dropped, never shown as a line.
#[test]
fn a_bounded_backward_join_drops_its_cut_first_row() {
    let source = rows(400, Some(3000));
    let anchor = source[..source.len() / 2].rfind('\n').unwrap() + 1;
    let (mut worker, output) = text_worker(&source);
    let viewport = worker.page_viewport(anchor as u64, true);
    admit(&mut worker, 91, &viewport);
    let mut reads = 0;
    let mut id = 91;
    let mut request = viewport.clone();
    while worker.page.is_none() {
        reply(&mut worker, &output, id, reply_to(&source, &request));
        reads += 1;
        if worker.page.is_none() {
            (request, id) = chained(&worker, 91);
        }
    }
    let page = worker.page.as_ref().unwrap();
    assert!(
        reads > 1 && page.text.len() >= JOIN_BYTES - 3000,
        "{reads} reads"
    );
    assert!(page.text.len() <= JOIN_BYTES + PROVIDER_BYTES);
    assert!(
        source[..page.start as usize].ends_with('\n'),
        "the cut row is dropped"
    );
    assert!(page.text.starts_with('r') && !page.partial_line);
    assert_eq!(page.end, anchor as u64);
}

/// A chain whose request was replaced reads no further and shows nothing.
#[test]
fn a_superseded_chain_shows_nothing() {
    let source = rows(1500, None);
    let (mut worker, output) = text_worker(&source);
    let viewport = worker.page_viewport(0, false);
    admit(&mut worker, 91, &viewport);
    reply(&mut worker, &output, 91, reply_to(&source, &viewport));
    let (next, physical) = chained(&worker, 91);
    // A newer request replaces the one being joined.
    let newer = worker.page_viewport(50_000, false);
    admit(&mut worker, 95, &newer);
    reply(&mut worker, &output, physical, reply_to(&source, &next));
    assert!(worker.page.is_none(), "no page of the replaced request");
    assert_eq!(worker.viewport_request, Some(95));
    assert!(worker.chained_request(91).is_none());
    assert!(unsent::take().is_empty());
}

/// Adjacent pages join with shifted anchors and spans; others are returned unchanged.
#[test]
fn adjacent_text_pages_join_and_a_cut_first_row_is_dropped() {
    let source = "alpha\nbravo charlie\ndelta\necho foxtrot\n";
    let mut first = edge_page(source, 2, 15);
    first.spans = vec![viewer::Span {
        start: 2,
        end: 9,
        role: viewer::STYLE_KEYWORD,
        flags: 1,
    }];
    first.partial_line = true;
    let mut second = edge_page(source, 15, 26);
    second.spans = vec![viewer::Span {
        start: 1,
        end: 4,
        role: viewer::STYLE_STRING,
        flags: 0,
    }];
    assert!(adjacent(&first, &second));
    let joined = join(first.clone(), second.clone());
    midnight_plugin_host::api::codec::Model::validate(&joined).unwrap();
    assert_eq!(joined.text, &source[2..26]);
    assert_eq!((joined.source.offset, joined.source.length), (2, 24));
    assert_eq!((joined.spans[1].start, joined.spans[1].end), (14, 17));
    assert!(joined.partial_line, "its first row may still be cut");
    assert!(
        !adjacent(&second, &first),
        "the second does not continue the first"
    );
    let (trimmed, cut) = trim_cut_start(joined);
    assert!(cut);
    midnight_plugin_host::api::codec::Model::validate(&trimmed).unwrap();
    assert_eq!(trimmed.text, &source[6..26]);
    assert_eq!(trimmed.source.offset, 6);
    assert_eq!(
        (trimmed.spans[0].start, trimmed.spans[0].end),
        (0, 5),
        "a span over the cut keeps its rest"
    );
    // An enormous line has no whole row to keep.
    let line = edge_page("x".repeat(64).as_str(), 3, 40);
    let (kept, cut) = trim_cut_start(line.clone());
    assert!(!cut && kept.text == line.text);
}
