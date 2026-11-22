//! Whole-file selection and its streamed Copy (`worker_whole.rs`), against a simulated
//! plugin session that answers the worker's page requests.
use super::*;
use std::time::Instant;

/// A decoded text page of `text` at source `offset`, with an exact anchor at every
/// character boundary, as the Basic and Source plugins send them.
fn text_page(offset: u64, text: &str, after: bool) -> viewer::TextPage {
    let anchors = text
        .char_indices()
        .map(|(at, _)| at)
        .chain(std::iter::once(text.len()))
        .map(|at| viewer::Anchor {
            text_offset: at as u32,
            source_offset: offset + at as u64,
        })
        .collect();
    viewer::TextPage {
        revision: 1,
        source: viewer::SourceRange {
            offset,
            length: text.len() as u64,
        },
        text: text.into(),
        spans: vec![],
        anchors,
        has_before: offset > 0,
        has_after: after,
        partial_line: !text.ends_with('\n'),
    }
}

/// `text` cut into pages of at most `page` bytes at character boundaries.
fn pages(text: &str, page: usize) -> Vec<viewer::TextPage> {
    let mut pages = Vec::new();
    let mut start = 0;
    while start < text.len() || pages.is_empty() {
        let mut end = (start + page).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        pages.push(text_page(start as u64, &text[start..end], end < text.len()));
        start = end;
    }
    pages
}

/// A Text view of `text` showing its first page, painted once.
fn shown(text: &str) -> (Worker, Mailbox, u64) {
    let (mut worker, output) = crate::worker::tests::worker();
    worker.length = text.len() as u64;
    let first = pages(text, 4096).remove(0);
    assert!(worker.set_page(Page::text(first, &worker.appearance), &output));
    worker.paint(&output).unwrap();
    output.take();
    let frame = worker.frames.latest_revision();
    (worker, output, frame)
}

/// Answer every page request the worker makes from `text`, cut as a plugin would at the
/// requested size, until it asks for no more. Returns the requests' anchors.
fn answer(worker: &mut Worker, output: &Mailbox, text: &str, page: usize) -> Vec<u64> {
    let mut anchors = Vec::new();
    while let [(id, viewport)] = &crate::worker::unsent::take_admitted()[..] {
        assert!(!viewport.backwards);
        assert_eq!(
            (viewport.max_bytes, viewport.max_lines),
            (256 * 1024, 4096),
            "the protocol's largest page"
        );
        anchors.push(viewport.source_anchor);
        let from = viewport.source_anchor as usize;
        let mut reply = pages(&text[from..], page.min(viewport.max_bytes as usize)).remove(0);
        reply.source.offset += from as u64;
        for anchor in &mut reply.anchors {
            anchor.source_offset += from as u64;
        }
        reply.has_before = from > 0;
        worker.event(*id, EventKind::TextPage(reply), output);
        worker.event(*id, EventKind::Completed, output);
        assert!(anchors.len() < 10_000, "the copy ends");
    }
    anchors
}

fn copied(output: &Mailbox) -> Vec<String> {
    output
        .take()
        .into_iter()
        .filter_map(|output| match output {
            ViewerOutput::CopyText(text) => Some(text),
            _ => None,
        })
        .collect()
}

/// Ctrl+A then Copy in a Text view of a file of several pages: the worker reads the file
/// from its start in the protocol's largest pages, one request at a time, and hands the
/// whole decoded text to the clipboard once, after the last page.
#[test]
fn ctrl_a_then_copy_reads_every_page_and_copies_the_whole_text() {
    let text: String = (0..3000)
        .map(|line| format!("line {line:04}: Žluťoučký kůň 🦀 tab\there\n"))
        .collect();
    let (mut worker, output, frame) = shown(&text);
    crate::worker::unsent::take();
    crate::worker::unsent::take_admitted();
    crate::worker::unsent::admit_from(Some(500));
    worker.input(ViewerInput::SelectPage { frame }, &output);
    worker.input(ViewerInput::Copy { frame }, &output);
    let anchors = answer(&mut worker, &output, &text, 8 * 1024);
    crate::worker::unsent::admit_from(None);
    assert!(anchors.len() > 10, "{} page requests", anchors.len());
    assert_eq!(anchors[0], 0);
    assert_eq!(copied(&output), [text]);
}

fn copying(outputs: &[ViewerOutput]) -> Vec<bool> {
    outputs
        .iter()
        .filter_map(|output| match output {
            ViewerOutput::Copying(running) => Some(*running),
            _ => None,
        })
        .collect()
}

/// The one page request a copy has outstanding.
fn request() -> (u64, messages::Viewport) {
    match &crate::worker::unsent::take_admitted()[..] {
        [request] => request.clone(),
        other => panic!("one page request, not {}", other.len()),
    }
}

fn rows(count: usize) -> String {
    (0..count)
        .map(|row| format!("row {row:05} of the whole file\n"))
        .collect()
}

fn click(worker: &mut Worker, output: &Mailbox) {
    worker.paint(output).unwrap();
    output.take();
    let frame = worker.frames.latest_revision();
    worker.input(
        ViewerInput::Select {
            frame,
            x: 30.0,
            y: 30.0,
            extend: false,
        },
        output,
    );
}

#[test]
fn the_whole_selection_shows_on_every_page_and_in_the_status_until_a_click_replaces_it() {
    let text = rows(1000);
    let (mut worker, output, frame) = shown(&text);
    worker.input(ViewerInput::SelectPage { frame }, &output);
    let first = worker.shaped.as_ref().unwrap().text().len();
    assert_eq!(worker.shown_selection(), Some((0, first)));
    assert!(
        worker.selection.is_none(),
        "no page range stands for the file"
    );
    worker.paint(&output).unwrap();
    let status = output
        .take()
        .into_iter()
        .find_map(|output| match output {
            ViewerOutput::Frame(frame) => Some(frame.status),
            _ => None,
        })
        .unwrap();
    let selected = worker.frames.latest_revision();
    let size = format!("({:.1} KiB)", text.len() as f64 / 1024.0);
    assert!(
        status.ends_with(&format!("Whole file selected {size}")),
        "{status}"
    );
    // Another page of the file, as scrolling shows it: all of it selected, and Copy still
    // accepts the frame that showed the selection first.
    let second = pages(&text, 4096).remove(3);
    assert!(worker.set_page(Page::text(second, &worker.appearance), &output));
    let shown = worker.shaped.as_ref().unwrap().text().len();
    assert_eq!(worker.shown_selection(), Some((0, shown)));
    assert!(worker.frames.accepts_copy(selected));
    assert!(
        !worker.frames.accepts_copy(frame),
        "that frame showed no selection"
    );
    // The renderer gets the rows a raster shows, as for Ctrl+A's rows before.
    let page = worker.shaped.as_ref().unwrap();
    let painted = whole::painted(true, None, page, 40.0, 100.0).unwrap();
    assert!(painted.start > 0 && painted.end < shown, "{painted:?}");
    // A click replaces it.
    click(&mut worker, &output);
    assert!(!worker.whole.selected());
    let (start, end) = worker.shown_selection().unwrap();
    assert_eq!(start, end, "a click selects nothing yet");
    assert!(!worker.status_note().contains("Whole file"));
}

#[test]
fn escape_another_selection_or_another_file_stops_a_copy_that_copies_nothing() {
    let text = rows(2000);
    let (mut worker, output, frame) = shown(&text);
    crate::worker::unsent::take_admitted();
    crate::worker::unsent::admit_from(Some(900));
    worker.input(ViewerInput::SelectPage { frame }, &output);
    worker.input(ViewerInput::Copy { frame }, &output);
    assert_eq!(copying(&output.take()), [true]);
    let (id, viewport) = request();
    assert_eq!(viewport.source_anchor, 0);
    // A slow copy shows how far it got.
    worker.whole.copy.as_mut().unwrap().started = Instant::now() - Duration::from_secs(1);
    worker.event(
        id,
        EventKind::TextPage(pages(&text, 8192).remove(0)),
        &output,
    );
    worker.event(id, EventKind::Completed, &output);
    let percent = format!("Copying the whole file… {}% (", 8192 * 100 / text.len());
    assert!(worker.status.starts_with(&percent), "{}", worker.status);
    // Copy again while it runs keeps the one copy.
    let (id, viewport) = request();
    assert_eq!(viewport.source_anchor, 8192);
    worker.input(ViewerInput::Copy { frame }, &output);
    assert!(crate::worker::unsent::take_admitted().is_empty());
    // Escape: no text, the window learns the copy ended, the selection stays.
    worker.input(ViewerInput::CancelCopy, &output);
    let outputs = output.take();
    assert_eq!(copying(&outputs), [false]);
    assert!(
        !outputs
            .iter()
            .any(|output| matches!(output, ViewerOutput::CopyText(_)))
    );
    assert!(worker.status.contains("stopped; nothing was copied"));
    assert!(worker.whole.selected());
    // The cancelled request's late page copies nothing.
    worker.event(
        id,
        EventKind::TextPage(pages(&text, 8192).remove(1)),
        &output,
    );
    worker.event(id, EventKind::Completed, &output);
    assert!(copied(&output).is_empty());
    // Escape without a copy still answers, so the window's next Escape closes it.
    worker.input(ViewerInput::CancelCopy, &output);
    assert_eq!(copying(&output.take()), [false]);
    // Copy starts over from the start; a new selection stops it.
    worker.input(ViewerInput::Copy { frame }, &output);
    assert_eq!(request().1.source_anchor, 0);
    click(&mut worker, &output);
    worker.poll_whole(&output);
    assert_eq!(copying(&output.take()), [false]);
    assert!(worker.whole.copy.is_none());
    // So does another file or mode.
    let latest = worker.frames.latest_revision();
    worker.input(ViewerInput::SelectPage { frame: latest }, &output);
    worker.input(ViewerInput::Copy { frame: latest }, &output);
    request();
    worker.reset_whole_file(&output);
    assert_eq!(
        copying(&output.take()),
        [false],
        "the latest state replaces the start"
    );
    assert!(!worker.whole.selected() && worker.whole.copy.is_none());
    crate::worker::unsent::admit_from(None);
}

#[test]
fn a_file_too_large_for_the_clipboard_copies_nothing_and_points_to_copying_the_file() {
    let (mut worker, output, frame) = shown(&rows(100));
    crate::worker::unsent::take_admitted();
    crate::worker::unsent::admit_from(Some(900));
    worker.input(ViewerInput::SelectPage { frame }, &output);
    // Text decoders give at least a byte of text per four of the file: refused unread.
    worker.length = 4 * crate::whole_copy::LIMIT as u64 + 1;
    worker.input(ViewerInput::Copy { frame }, &output);
    assert!(crate::worker::unsent::take_admitted().is_empty());
    assert!(
        worker
            .status
            .contains("too large to copy as text (more than 64.0 MiB)")
            && worker.status.contains("F5"),
        "{}",
        worker.status
    );
    assert!(copied(&output).is_empty());
    crate::worker::unsent::admit_from(None);
}

#[test]
fn a_read_that_fails_or_a_changed_file_fails_the_copy_and_copies_nothing() {
    let text = rows(1000);
    let (mut worker, output, frame) = shown(&text);
    crate::worker::unsent::take_admitted();
    crate::worker::unsent::admit_from(Some(900));
    worker.input(ViewerInput::SelectPage { frame }, &output);
    worker.input(ViewerInput::Copy { frame }, &output);
    let (id, _) = request();
    worker.event(
        id,
        EventKind::TextPage(pages(&text, 8192).remove(0)),
        &output,
    );
    worker.event(id, EventKind::Completed, &output);
    // The session reads the captured revision: after the file changed, a read fails.
    let (id, _) = request();
    worker.event(
        id,
        EventKind::Failed(messages::Failure {
            code: midnight_plugin_host::api::abi::INVALID,
            message: "Source changed after capture".into(),
        }),
        &output,
    );
    let outputs = output.take();
    assert_eq!(copying(&outputs), [false]);
    assert!(
        !outputs
            .iter()
            .any(|output| matches!(output, ViewerOutput::CopyText(_)))
    );
    assert_eq!(
        worker.status,
        "Cannot copy the whole file: Source changed after capture. Nothing was copied."
    );
    // A page of another revision fails it too.
    worker.input(ViewerInput::Copy { frame }, &output);
    let (id, _) = request();
    worker.event(
        id,
        EventKind::TextPage(pages(&text, 8192).remove(0)),
        &output,
    );
    worker.event(id, EventKind::Completed, &output);
    let (id, _) = request();
    let mut changed = pages(&text, 8192).remove(1);
    changed.revision = 2;
    worker.event(id, EventKind::TextPage(changed), &output);
    worker.event(id, EventKind::Completed, &output);
    assert!(
        worker.status.contains("the file changed while it was read"),
        "{}",
        worker.status
    );
    assert!(copied(&output).is_empty());
    crate::worker::unsent::admit_from(None);
}

/// Markdown: Ctrl+A then Copy joins the document's pages as the view shows them.
#[test]
fn a_markdown_document_copies_its_shown_text_across_pages() {
    use midnight_plugin_host::api::document;
    let block = |kind, source: u64, text: &str| {
        let mut page = text_page(source, text, false);
        page.has_before = false;
        document::Block {
            kind,
            depth: 0,
            heading_level: u8::from(kind == document::BLOCK_HEADING),
            text: page,
            links: vec![],
            table: None,
            strikethrough: None,
        }
    };
    let page = |offset, length, blocks, after| document::DocumentPage {
        revision: 1,
        source: viewer::SourceRange { offset, length },
        blocks,
        has_before: offset > 0,
        has_after: after,
    };
    let (mut worker, output) = crate::worker::tests::worker();
    worker.mode = viewer::MODE_DOCUMENT;
    worker.length = 60;
    let first = page(
        0,
        30,
        vec![
            block(document::BLOCK_HEADING, 2, "Notes"),
            block(document::BLOCK_LIST_ITEM, 12, "first item"),
        ],
        true,
    );
    let shown = Page::document(first.clone(), &worker.appearance);
    assert!(worker.set_page(shown, &output));
    worker.paint(&output).unwrap();
    output.take();
    let frame = worker.frames.latest_revision();
    crate::worker::unsent::take_admitted();
    crate::worker::unsent::admit_from(Some(900));
    worker.input(ViewerInput::SelectPage { frame }, &output);
    worker.input(ViewerInput::Copy { frame }, &output);
    let (id, _) = request();
    worker.event(id, EventKind::DocumentPage(first), &output);
    worker.event(id, EventKind::Completed, &output);
    let (id, viewport) = request();
    assert_eq!(viewport.source_anchor, 30);
    let last = page(
        30,
        30,
        vec![block(document::BLOCK_PARAGRAPH, 40, "Closing words.")],
        false,
    );
    worker.event(id, EventKind::DocumentPage(last), &output);
    worker.event(id, EventKind::Completed, &output);
    let whole = "Notes\n\n• first item\n\nClosing words.";
    assert_eq!(copied(&output), [whole]);
    assert_eq!(
        worker.status,
        format!("Copied the whole file: {} bytes of text.", whole.len())
    );
    crate::worker::unsent::admit_from(None);
}
