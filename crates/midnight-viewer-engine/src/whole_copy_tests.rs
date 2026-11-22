//! Joining forward pages into a whole file's text (`whole_copy.rs`).
use super::*;
use midnight_plugin_host::api::{
    document::{BLOCK_HEADING, BLOCK_LIST_ITEM, BLOCK_PARAGRAPH, BLOCK_RULE, Block, DocumentPage},
    viewer::{Anchor, SourceRange, TextPage},
};

/// A decoded page of `text` at source `offset` whose characters each consumed `width`
/// source bytes (2 for UTF-16 text), with an exact anchor at every character boundary.
fn decoded(offset: u64, text: &str, width: u64, after: bool) -> TextPage {
    let mut anchors = vec![Anchor {
        text_offset: 0,
        source_offset: offset,
    }];
    let mut source = offset;
    for (at, character) in text.char_indices() {
        source += width;
        anchors.push(Anchor {
            text_offset: (at + character.len_utf8()) as u32,
            source_offset: source,
        });
    }
    TextPage {
        revision: 7,
        source: SourceRange {
            offset,
            length: source - offset,
        },
        text: text.into(),
        spans: vec![],
        anchors,
        has_before: offset > 0,
        has_after: after,
        partial_line: !text.ends_with('\n'),
    }
}

/// Join `pages` until the last; the read offsets asked for on the way.
fn join(pages: &[TextPage], limit: usize) -> Result<(String, Vec<u64>), JoinError> {
    let mut joined = Joined::new(limit);
    let mut reads = Vec::new();
    for page in pages {
        match joined.text_page(page)? {
            Next::Read(offset) => reads.push(offset),
            Next::Done => return Ok((joined.finish(), reads)),
        }
    }
    panic!("no last page")
}

/// Each page's source end: where the next forward read starts.
fn end(page: &TextPage) -> u64 {
    page.source.offset + page.source.length
}

#[test]
fn text_pages_join_end_to_end_without_adding_breaks() {
    // Line breaks as the decoders show them (a CRLF file's are LF), multi-byte
    // characters on both sides of a page boundary and a wrapped line, which layout
    // alone wraps. The first page starts after a byte-order mark.
    let first = decoded(3, "first line\nsecond é", 1, true);
    let second = decoded(end(&first), "🦀 rest of it\n", 1, true);
    let last = decoded(end(&second), "last line without break", 1, false);
    let reads = [end(&first), end(&second)];
    let (text, read) = join(&[first, second, last], LIMIT).unwrap();
    assert_eq!(
        text,
        "first line\nsecond é🦀 rest of it\nlast line without break"
    );
    assert_eq!(read, reads, "each read starts where the page before ended");
}

#[test]
fn a_page_from_before_the_copied_end_continues_at_its_exact_anchor() {
    // UTF-16 text: two source bytes per character, so text offsets are not source ones.
    let first = decoded(2, "abc\ndéf", 2, true);
    assert_eq!(end(&first), 16);
    // A decoder resynchronizing before the anchor sends "déf" (source 10..16) again.
    let again = decoded(10, "déf\nghi\n", 2, false);
    let (text, _) = join(&[first.clone(), again], LIMIT).unwrap();
    assert_eq!(text, "abc\ndéf\nghi\n");
    // A page that starts after the copied end, or has no anchor at it, ends the copy.
    let gap = decoded(18, "ghi", 2, false);
    assert_eq!(
        join(&[first.clone(), gap], LIMIT),
        Err(JoinError::Discontinuous)
    );
    let unaligned = decoded(15, "xghi", 2, false);
    assert_eq!(
        join(&[first, unaligned], LIMIT),
        Err(JoinError::Discontinuous)
    );
}

#[test]
fn a_long_line_in_partial_fragments_stays_one_line() {
    // The 200,000-character line of the large-file fixtures, in 8 KiB fragments.
    let line = "x".repeat(200_000);
    let mut pages = Vec::new();
    let mut offset = 0;
    for chunk in line.as_bytes().chunks(8 * 1024) {
        let end = offset + chunk.len();
        pages.push(decoded(
            offset as u64,
            std::str::from_utf8(chunk).unwrap(),
            1,
            true,
        ));
        offset = end;
    }
    pages.push(decoded(offset as u64, "\nnext\n", 1, false));
    let (text, reads) = join(&pages, LIMIT).unwrap();
    assert_eq!(text, format!("{line}\nnext\n"));
    assert_eq!(reads.len(), pages.len() - 1);
}

#[test]
fn an_empty_file_copies_empty_text() {
    assert_eq!(
        join(&[decoded(0, "", 1, false)], LIMIT).unwrap().0,
        "",
        "an empty file (or one of a BOM alone) is one empty last page"
    );
    assert_eq!(join(&[decoded(3, "", 1, false)], LIMIT).unwrap().0, "");
}

#[test]
fn the_limit_bounds_the_text_and_its_buffer() {
    let pages = [
        decoded(0, "0123456789", 1, true),
        decoded(10, "0123456789", 1, false),
    ];
    assert_eq!(
        join(&pages, 20).unwrap().0.len(),
        20,
        "exactly at the limit"
    );
    assert_eq!(join(&pages, 19), Err(JoinError::Limit));
    let mut joined = Joined::new(25);
    for page in &pages {
        joined.text_page(page).unwrap();
    }
    assert!(joined.text.capacity() <= 25, "{}", joined.text.capacity());
}

#[test]
fn pages_of_another_revision_or_without_progress_end_the_copy() {
    let mut changed = decoded(4, "efgh", 1, false);
    changed.revision = 8;
    assert_eq!(
        join(&[decoded(0, "abcd", 1, true), changed], LIMIT),
        Err(JoinError::Changed),
        "the file changed while it was read"
    );
    assert_eq!(
        join(
            &[decoded(0, "abcd", 1, true), decoded(4, "", 1, true)],
            LIMIT
        ),
        Err(JoinError::Discontinuous),
        "an empty page that claims more would repeat forever"
    );
}

/// A Markdown block of `text` at `source`, its source as long as its text.
fn block(kind: u16, source: u64, text: &str) -> Block {
    let mut page = decoded(source, text, 1, false);
    page.has_before = false;
    Block {
        kind,
        depth: 0,
        heading_level: u8::from(kind == BLOCK_HEADING),
        text: page,
        links: vec![],
        table: None,
        strikethrough: None,
    }
}

fn document(offset: u64, length: u64, blocks: Vec<Block>, after: bool) -> DocumentPage {
    DocumentPage {
        revision: 7,
        source: SourceRange { offset, length },
        blocks,
        has_before: offset > 0,
        has_after: after,
    }
}

#[test]
fn document_pages_join_blocks_as_a_page_shows_them() {
    let mut joined = Joined::new(LIMIT);
    // A heading, then a paragraph the first page cuts at its end (the page ends where
    // its last block does) and the second goes on with.
    let first = document(
        0,
        22,
        vec![
            block(BLOCK_HEADING, 2, "Title"),
            block(BLOCK_PARAGRAPH, 9, "First paragra"),
        ],
        true,
    );
    assert_eq!(joined.document_page(&first), Ok(Next::Read(22)));
    let second = document(
        22,
        28,
        vec![
            block(BLOCK_PARAGRAPH, 22, "ph that goes on"),
            block(BLOCK_LIST_ITEM, 40, "item"),
            block(BLOCK_RULE, 50, ""),
        ],
        true,
    );
    assert_eq!(joined.document_page(&second), Ok(Next::Read(50)));
    // The next page starts before the copied end and shows the list item and the rule
    // at the page boundary again: neither is copied twice.
    let third = document(
        40,
        30,
        vec![
            block(BLOCK_LIST_ITEM, 40, "item"),
            block(BLOCK_RULE, 50, ""),
            block(BLOCK_PARAGRAPH, 60, "Last."),
        ],
        false,
    );
    assert_eq!(joined.document_page(&third), Ok(Next::Done));
    assert_eq!(
        joined.finish(),
        "Title\n\nFirst paragraph that goes on\n\n• item\n\n────────────\n\nLast."
    );
}

#[test]
fn a_document_block_shown_again_whole_continues_at_its_exact_anchor() {
    let mut joined = Joined::new(LIMIT);
    let row = |source, text| block(BLOCK_PARAGRAPH, source, text);
    // The first page cuts the second table row; the next shows that row whole again.
    let first = document(0, 103, vec![row(0, "a\tb\tc"), row(100, "a\tb")], true);
    assert_eq!(joined.document_page(&first), Ok(Next::Read(103)));
    let second = document(
        100,
        110,
        vec![row(100, "a\tb\tc"), row(200, "a\tb\tc")],
        false,
    );
    assert_eq!(joined.document_page(&second), Ok(Next::Done));
    assert_eq!(joined.finish(), "a\tb\tc\n\na\tb\tc\n\na\tb\tc");
}
