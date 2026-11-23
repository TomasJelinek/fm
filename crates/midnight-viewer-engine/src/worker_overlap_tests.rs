//! A refill reads only the rows past the shown page (`Worker::overlap`), and the page it
//! shows is the one a whole read gives.
use super::frontier::{Engine, Window, gesture, rest, rows};
use super::*;
use midnight_plugin_host::HostEvent;

fn geometry() -> ViewerGeometry {
    ViewerGeometry {
        width: 1312,
        height: 720,
        scale: 1.0,
    }
}

/// Plugin reads and text bytes of four PageDown and four PageUp from the middle of a
/// file of rows `width` bytes wide; every gesture moves its whole distance.
fn page_reads(mode: u16, width: usize) -> (usize, usize) {
    let mut engine = Engine::open(rows(3_000_000 / width, width), mode, geometry(), 1);
    let mut window = Window::default();
    rest(&mut engine, &mut window);
    window.seek(&mut engine, 0.5);
    rest(&mut engine, &mut window);
    let (reads, bytes) = (engine.reads, engine.bytes);
    let screen = f64::from(geometry().height);
    for page in [
        screen, screen, screen, screen, -screen, -screen, -screen, -screen,
    ] {
        let moved = gesture(&mut engine, &mut window, &[(page, true)], false);
        assert!(
            (moved - page).abs() < 1.0,
            "{width}-byte rows: a page key moved {moved} of {page}"
        );
    }
    (engine.reads - reads, engine.bytes - bytes)
}

/// A refill near the shown page's edge read every row around the camera again, in ranges
/// of at most 8 KiB: ten reads per PageDown over 500-byte rows. It now reads the new rows
/// only, in one range or a few, and no more ranges or bytes over short rows either. One
/// test per mode, so that they run in parallel.
mod page_keys_read_only_the_rows_past_the_shown_page {
    use super::*;

    #[test]
    fn text() {
        reads_only_the_rows_past_the_shown_page(viewer::MODE_TEXT);
    }

    #[test]
    fn source() {
        reads_only_the_rows_past_the_shown_page(viewer::MODE_SOURCE);
    }
}

fn reads_only_the_rows_past_the_shown_page(mode: u16) {
    let (reads, bytes) = page_reads(mode, 60);
    assert!(reads <= 16, "mode {mode}, 60-byte rows: {reads} reads");
    assert!(
        bytes <= 40 * 1024,
        "mode {mode}, 60-byte rows: {bytes} bytes read"
    );
    let (reads, bytes) = page_reads(mode, 500);
    assert!(reads <= 40, "mode {mode}, 500-byte rows: {reads} reads");
    assert!(
        bytes <= 300 * 1024,
        "mode {mode}, 500-byte rows: {bytes} bytes read"
    );
}

/// The worker showing `source` from `start` to `end`, with requests admitted from `first`.
fn shown(source: &str, start: usize, end: usize, first: u64) -> (Worker, Mailbox) {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_TEXT;
    worker.length = source.len() as u64;
    worker.geometry = geometry();
    worker.raster.enabled = true;
    worker.raster.legacy_pages = true;
    let mut page = super::text_position::edge_page(source, start, end);
    page.has_after = end < source.len();
    assert!(worker.set_page(Page::text(page, &worker.appearance), &output));
    unsent::take();
    unsent::take_admitted();
    unsent::admit_from(Some(first));
    (worker, output)
}

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

/// A forward request inside the shown page reads from the page's end, with the line budget
/// the rows it holds leave; the joined reply is exactly the page a whole read gives.
#[test]
fn a_request_inside_the_shown_page_reads_from_its_end_and_joins_the_held_rows() {
    let source = rows(400, 100);
    let row = |index: usize| index * 100;
    let (mut worker, output) = shown(&source, row(10), row(60), 1 << 30);
    let whole = worker.page_viewport(row(40) as u64, false);
    worker.request_page(row(40) as u64, false, &output);
    let admitted = unsent::take_admitted();
    let [(id, read)] = admitted.as_slice() else {
        panic!("one read: {admitted:?}");
    };
    assert_eq!(
        (read.source_anchor, read.backwards, read.max_lines),
        (row(60) as u64, false, whole.max_lines - 20),
        "only the rows past the shown page are read"
    );
    let expected = super::frontier::basic_reply(&source, &whole);
    reply(
        &mut worker,
        &output,
        *id,
        super::frontier::basic_reply(&source, read),
    );
    let page = worker.page.as_ref().expect("joined page");
    assert_eq!(
        (page.start, page.end),
        (
            expected.source.offset,
            expected.source.offset + expected.source.length
        )
    );
    assert_eq!(&*page.text, expected.text.as_str());
    assert_eq!(page.anchors, expected.anchors);
    assert_eq!(
        (page.before, page.after, page.partial_line),
        (
            expected.has_before,
            expected.has_after,
            expected.partial_line
        )
    );
    unsent::admit_from(None);
}

/// A backward request reads the rows before the shown page and keeps the ones it holds.
#[test]
fn a_backward_request_inside_the_shown_page_reads_before_its_start() {
    let source = rows(400, 100);
    let row = |index: usize| index * 100;
    let (mut worker, output) = shown(&source, row(200), row(250), 1 << 30);
    let whole = worker.page_viewport(row(220) as u64, true);
    worker.request_page(row(220) as u64, true, &output);
    let admitted = unsent::take_admitted();
    let [(id, read)] = admitted.as_slice() else {
        panic!("one read: {admitted:?}");
    };
    assert_eq!(
        (read.source_anchor, read.backwards, read.max_lines),
        (row(200) as u64, true, whole.max_lines - 20)
    );
    reply(
        &mut worker,
        &output,
        *id,
        super::frontier::basic_reply(&source, read),
    );
    let page = worker.page.as_ref().expect("joined page");
    let expected = super::frontier::basic_reply(&source, &whole);
    assert_eq!(page.end, row(220) as u64);
    assert_eq!(
        (page.start, &*page.text),
        (expected.source.offset, expected.text.as_str())
    );
    unsent::admit_from(None);
}

/// Rows that do not continue the held ones (as after a provider's realignment) are not
/// joined to them: the request is then read whole, and the page is still the right one.
#[test]
fn a_reply_that_does_not_continue_the_held_rows_reads_the_whole_request() {
    let source = rows(400, 100);
    let row = |index: usize| index * 100;
    let (mut worker, output) = shown(&source, row(10), row(60), 1 << 30);
    let whole = worker.page_viewport(row(40) as u64, false);
    worker.request_page(row(40) as u64, false, &output);
    let [(id, read)]: [(u64, messages::Viewport); 1] = unsent::take_admitted().try_into().unwrap();
    let mut shifted = read.clone();
    shifted.source_anchor += 1;
    reply(
        &mut worker,
        &output,
        id,
        super::frontier::basic_reply(&source, &shifted),
    );
    assert!(
        worker.page.as_ref().unwrap().start == row(10) as u64,
        "nothing joined yet"
    );
    let [(again, request)]: [(u64, messages::Viewport); 1] =
        unsent::take_admitted().try_into().unwrap();
    assert_eq!(request, whole, "the whole request is read");
    reply(
        &mut worker,
        &output,
        again,
        super::frontier::basic_reply(&source, &request),
    );
    let page = worker.page.as_ref().expect("whole page");
    let expected = super::frontier::basic_reply(&source, &whole);
    assert_eq!(
        (page.start, &*page.text),
        (expected.source.offset, expected.text.as_str())
    );
    assert_eq!(worker.viewport_request, None, "the request completed");
    unsent::admit_from(None);
}

/// A scrollbar position is read whole: its recovery checks the read it requested.
#[test]
fn scrollbar_positions_read_their_whole_request() {
    let source = rows(400, 100);
    let row = |index: usize| index * 100;
    let (mut worker, output) = shown(&source, row(10), row(60), 1 << 30);
    let whole = worker.page_viewport(row(40) as u64, false);
    worker.request_page_for(row(40) as u64, false, Some(1), &output);
    let admitted = unsent::take_admitted();
    assert_eq!(admitted.len(), 1);
    assert_eq!(admitted[0].1, whole);
    unsent::admit_from(None);
}
