//! Reuse complete layout units when a bounded partial parse exceeds context retention.
#[allow(dead_code)]
mod support;

use midnight_plugin_sdk::api::{abi, codec, document::*, messages::*, viewer::*};
use midnight_viewer_markdown::midnight_plugin_entry_v1 as entry;
use support::Host;

fn open(input: &str) -> Host {
    let mut host = Host::new(input.as_bytes(), entry);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 3,
            length: Some(input.len() as u64),
            mode: MODE_DOCUMENT,
            display_name: "partial-context.md".into(),
        },
    );
    assert!(
        host.finish()
            .iter()
            .any(|event| event.kind == abi::EVENT_OPENED)
    );
    host
}

fn viewport(host: &mut Host, anchor: u64, backwards: bool, lines: u32) -> DocumentPage {
    host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: anchor,
            backwards,
            max_bytes: 65536,
            max_lines: lines,
        },
    );
    let events = host.finish();
    codec::decode(
        &events
            .iter()
            .find(|event| event.kind == abi::EVENT_DOCUMENT_PAGE)
            .unwrap_or_else(|| panic!("missing document: {events:?}"))
            .payload,
    )
    .unwrap()
}

fn paragraph(index: usize) -> String {
    format!("Paragraph {index:04}: {}end.\n\n", "word ".repeat(20))
}

fn long_document() -> String {
    (0..1500).map(paragraph).collect()
}

#[test]
fn partial_context_reuses_new_forward_and_backward_anchors_without_rereads() {
    let input = long_document();
    assert!(input.len() > 65536 * 2);
    let mut host = open(&input);
    let first = viewport(&mut host, 0, false, 4);
    assert!(first.has_after);
    assert_eq!(first.blocks[0].text.text, paragraph(0).trim());
    let reads = host.reads().len();
    for index in 20..40 {
        let anchor = input.find(&format!("Paragraph {index:04}:")).unwrap() as u64;
        for backwards in [false, true] {
            let page = viewport(&mut host, anchor, backwards, 4);
            let expected = if backwards { index - 4 } else { index };
            assert_eq!(page.blocks.len(), 4);
            assert_eq!(page.blocks[0].text.text, paragraph(expected).trim());
            assert!(page.has_before && page.has_after);
            assert_eq!(page.revision, 3);
        }
    }
    assert!(host.reads().iter().all(|read| read.length <= 65536));
    assert_eq!(
        host.reads().len(),
        reads,
        "nearby partial-page requests already have complete parsed layout units"
    );
}

#[test]
fn long_table_in_a_partial_context_keeps_every_row_without_rereads() {
    let prefix: String = (0..20).map(paragraph).collect();
    let input = format!(
        "{prefix}| ID | Description |\n| --- | ---: |\n| wide | {} |\n\
         | tail | final witness |\n\n{}",
        "long cell ".repeat(950),
        long_document()
    );
    let mut host = open(&input);
    viewport(&mut host, 0, false, 4);
    let reads = host.reads().len();
    for needle in ["long cell long cell", "final witness"] {
        let anchor = input.find(needle).unwrap() as u64;
        for backwards in [false, true] {
            let page = viewport(&mut host, anchor, backwards, 1);
            let rows: Vec<_> = page
                .blocks
                .iter()
                .filter(|block| block.table.is_some())
                .collect();
            assert_eq!(rows.len(), 3);
            assert_eq!(rows[0].text.text, "ID\tDescription");
            assert_eq!(
                rows[1].text.text,
                format!("wide\t{}", "long cell ".repeat(950).trim())
            );
            assert_eq!(rows[2].text.text, "tail\tfinal witness");
            assert!(rows[0].table.as_ref().unwrap().header);
            assert!(rows.iter().all(|row| {
                let cells = &row.table.as_ref().unwrap().cells;
                cells.len() == 2 && cells[1].alignment == TABLE_ALIGN_RIGHT
            }));
        }
    }
    assert_eq!(
        host.reads().len(),
        reads,
        "retaining a table must keep all column-measuring rows as one unit"
    );
}

#[test]
fn partial_reuse_preserves_backward_limits_inside_a_multibyte_scalar() {
    let prefix: String = (0..20).map(paragraph).collect();
    let input = format!(
        "{prefix}é boundary with 中文 and 😀.\n\n{}",
        long_document()
    );
    let anchor = input.find("é boundary").unwrap() as u64 + 1;
    let mut warm = open(&input);
    viewport(&mut warm, 0, false, 4);
    let actual = viewport(&mut warm, anchor, true, 4);
    let mut cold = open(&input);
    let expected = viewport(&mut cold, anchor, true, 4);
    assert_eq!(
        actual, expected,
        "a cache must not round a raw source limit forward"
    );
    assert!(
        actual
            .blocks
            .iter()
            .all(|block| !block.text.text.contains('�'))
    );
    assert!(actual.source.end().unwrap() <= anchor);
}

#[test]
fn full_bom_context_reuses_zero_anchor_with_a_different_line_budget() {
    let input = "\u{feff}# BOM heading\n\nA paragraph with **emphasis**.\n";
    let mut host = open(input);
    let first = viewport(&mut host, 0, false, 1);
    let reads = host.reads().len();
    let expanded = viewport(&mut host, 0, false, 2);
    assert_eq!(
        host.reads().len(),
        reads,
        "the BOM is not an artificial retained edge"
    );
    assert_eq!(expanded.blocks[0], first.blocks[0]);
    assert_eq!(expanded.source.offset, 3);
    assert!(!expanded.has_before);
    assert_eq!(expanded.blocks.len(), 2);
}
