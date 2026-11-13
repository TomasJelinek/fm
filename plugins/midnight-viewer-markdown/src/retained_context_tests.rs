use super::*;
use crate::{decode, markdown};
use std::cell::Cell;

fn parse(input: &str) -> (DocumentPage, Vec<Anchor>) {
    assert!(input.len() <= 65536);
    let decoded = decode::decode(
        input.as_bytes(),
        0,
        decode::Encoding::Utf8,
        true,
        65536,
        usize::MAX,
        || false,
    )
    .unwrap();
    let page = markdown::render_context(&decoded, 3, true, || false)
        .unwrap()
        .0;
    (page, decoded.anchors)
}

fn paragraphs() -> (DocumentPage, Vec<Anchor>) {
    let input: String = ['a', 'b', 'c', 'd']
        .iter()
        .map(|letter| format!("{}\n\n", letter.to_string().repeat(16300)))
        .collect();
    let page = parse(&input);
    assert!(page_cache::size(&page.0) > MAX_BYTES);
    page
}

fn window(anchor: u64, backwards: bool, max_lines: usize) -> Window {
    Window {
        anchor,
        backwards,
        max_bytes: 65536,
        max_lines,
    }
}

#[test]
fn trimmed_context_keeps_complete_groups_and_original_parse_coverage_within_budget() {
    let (raw, anchors) = paragraphs();
    let retained = Retained::new(&raw, &window(16302, false, 1), &anchors, || false).unwrap();
    assert!(retained.bytes() <= MAX_BYTES);
    assert_eq!(retained.boundaries.as_ref().unwrap().len(), 8152);
    assert_eq!(retained.source, raw.source);
    assert!(!retained.has_after, "the original read reached EOF");
    assert_eq!(retained.page().blocks.len(), 3);
    assert!(
        retained.page().has_after,
        "retention does not make an artificial EOF"
    );
    assert!(retained.page().source.end().unwrap() < raw.source.end().unwrap());
    for (block, letter) in retained.page().blocks.iter().zip(['a', 'b', 'c']) {
        assert_eq!(block.text.text.len(), 16300);
        assert!(block.text.text.chars().all(|value| value == letter));
    }
    let request = window(16302, false, 1);
    let page = retained.crop(&request).unwrap();
    let uncached = markdown::crop_context(&raw, &request).unwrap();
    assert!(
        page == uncached,
        "retention must preserve the uncached wire fallback"
    );
    assert_eq!(page.blocks.len(), 1);
    // A complete 16,300-byte group fits retention, but its 16,301 five-item
    // anchors exceed the public 65,536 CBOR-item budget. Emission independently
    // retains the existing 8 KiB scalar fallback and exact source boundaries.
    let text = &page.blocks[0].text;
    assert_eq!(text.text.len(), 8192);
    assert!(text.text.bytes().all(|value| value == b'b'));
    assert_eq!(page.source.offset, 16302);
    assert_eq!(page.source.length, 8192);
    assert_eq!(text.anchors.len(), 8193);
    assert_eq!(text.anchors[0].source_offset, 16302);
    assert_eq!(text.anchors.last().unwrap().source_offset, 24494);
    assert!(page.has_before && page.has_after);
}

#[test]
fn forward_demand_past_a_retained_edge_requires_normal_reconstruction() {
    let (raw, anchors) = paragraphs();
    let retained = Retained::new(&raw, &window(16302, false, 1), &anchors, || false).unwrap();
    let request = window(32604, false, 2);
    assert_eq!(
        semantic_window::crop_ref(&raw, &request)
            .unwrap()
            .blocks
            .len(),
        2
    );
    assert!(
        retained.crop(&request).is_none(),
        "the missing final group is not EOF"
    );
}

#[test]
fn backward_demand_past_a_retained_edge_requires_normal_reconstruction() {
    let (raw, anchors) = paragraphs();
    let retained = Retained::new(&raw, &window(48906, false, 1), &anchors, || false).unwrap();
    assert!(retained.page().source.offset > raw.source.offset);
    let request = window(32604, true, 2);
    assert_eq!(
        semantic_window::crop_ref(&raw, &request)
            .unwrap()
            .blocks
            .len(),
        2
    );
    assert!(
        retained.crop(&request).is_none(),
        "the missing first group is not BOF"
    );
}

#[test]
fn table_above_retained_budget_is_not_split_to_make_it_cacheable() {
    let mut input = "| Column |\n| --- |\n".to_string();
    for letter in ['a', 'b', 'c', 'd'] {
        input.push_str(&format!("| {} |\n", letter.to_string().repeat(16300)));
    }
    let (raw, anchors) = parse(&input);
    assert!(raw.blocks.iter().all(|block| block.table.is_some()));
    assert!(page_cache::size(&raw) > MAX_BYTES);
    assert!(Retained::new(&raw, &window(100, false, 1), &anchors, || false).is_none());
}

#[test]
fn cancellation_during_retention_does_not_publish_a_partial_context() {
    let (raw, anchors) = paragraphs();
    let polls = Cell::new(0);
    assert!(
        Retained::new(&raw, &window(16302, false, 1), &anchors, || {
            polls.set(polls.get() + 1);
            polls.get() > 2
        })
        .is_none()
    );
}

#[test]
fn source_boundary_map_preserves_scalars_crlf_and_large_source_offsets() {
    let input = "é\r\n😀";
    let utf16: Vec<u8> = input.encode_utf16().flat_map(u16::to_le_bytes).collect();
    for (bytes, encoding, expected) in [
        (input.as_bytes(), decode::Encoding::Utf8, [0x15, 0x01]),
        (utf16.as_slice(), decode::Encoding::Le, [0x45, 0x04]),
    ] {
        let offset = u64::MAX - bytes.len() as u64;
        let decoded =
            decode::decode(bytes, offset, encoding, true, 65536, usize::MAX, || false).unwrap();
        let source = SourceRange {
            offset,
            length: bytes.len() as u64,
        };
        let map = source_boundaries(&source, &decoded.anchors, || false).unwrap();
        assert_eq!(map.as_ref(), &expected);
    }
}
