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
            display_name: "large-table.md".into(),
        },
    );
    assert!(
        host.finish()
            .iter()
            .any(|event| event.kind == abi::EVENT_OPENED)
    );
    host
}

fn viewport(host: &mut Host, anchor: u64, backwards: bool, max_bytes: u32) -> DocumentPage {
    host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: anchor,
            backwards,
            max_bytes,
            max_lines: 40,
        },
    );
    let events = host.finish();
    codec::decode(
        &events
            .iter()
            .find(|event| event.kind == abi::EVENT_DOCUMENT_PAGE)
            .unwrap_or_else(|| panic!("document missing: {events:?}"))
            .payload,
    )
    .unwrap()
}

fn table(repeats: usize) -> String {
    format!(
        "| ID | Description |\n| --- | --- |\n| id | {} |\n| tail | final |\n\n",
        "a later cell with many words ".repeat(repeats)
    )
}

#[test]
fn a_large_complete_table_preserves_columns_for_both_scroll_directions() {
    let table = table(350);
    assert!(table.len() > 8192 && table.len() < 65536);
    let input = format!("{table}{}", "Following paragraph.\n\n".repeat(10_000));
    let mut host = open(&input);
    let first = viewport(&mut host, 0, false, 65536);
    let rows: Vec<_> = first
        .blocks
        .iter()
        .filter(|block| block.table.is_some())
        .collect();
    assert_eq!(rows.len(), 3);
    assert!(rows[2].text.text.contains("final"));
    for backwards in [false, true] {
        let page = viewport(&mut host, 5_000, backwards, 65536);
        let actual: Vec<_> = page
            .blocks
            .iter()
            .filter(|block| block.table.is_some())
            .collect();
        assert_eq!(
            actual, rows,
            "all column-measuring rows must remain identical"
        );
    }
    assert!(host.reads().iter().all(|read| read.length <= 65536));
    assert!(
        host.reads()
            .iter()
            .all(|read| read.offset + u64::from(read.length) < input.len() as u64)
    );
}

#[test]
fn a_table_over_the_raw_budget_uses_bounded_prose_and_keeps_advancing() {
    let table = table(5000);
    let input = format!("{table}{}", "After table.\n\n".repeat(10_000));
    let mut host = open(&input);
    let first = viewport(&mut host, 0, false, 65536);
    assert!(first.blocks.iter().all(|block| block.table.is_none()));
    assert!(first.has_after);
    let end = first.source.end().unwrap();
    assert!(end <= 65536);
    let next = viewport(&mut host, end, false, 65536);
    assert!(next.source.end().unwrap() > end);
    assert!(next.blocks.iter().all(|block| block.table.is_none()));
    assert!(host.reads().iter().all(|read| read.length <= 65536));
}

#[test]
fn a_caller_with_a_smaller_budget_gets_prose_instead_of_a_partial_table() {
    let input = table(1400);
    let mut host = open(&input);
    let first = viewport(&mut host, 0, false, 8192);
    assert!(first.blocks.iter().all(|block| block.table.is_none()));
    assert!(
        first
            .blocks
            .iter()
            .map(|block| block.text.text.len())
            .sum::<usize>()
            <= 8192
    );
    let next = viewport(&mut host, first.source.end().unwrap(), false, 8192);
    assert!(next.source.end().unwrap() > first.source.end().unwrap());
}

#[test]
fn a_complete_table_over_the_wire_anchor_budget_uses_bounded_prose() {
    let input = table(1400);
    assert!(input.len() < 65536);
    let mut host = open(&input);
    let first = viewport(&mut host, 0, false, 65536);
    assert!(first.blocks.iter().all(|block| block.table.is_none()));
    assert!(
        first
            .blocks
            .iter()
            .map(|block| block.text.text.len())
            .sum::<usize>()
            <= 8192
    );
    let end = first.source.end().unwrap();
    let next = viewport(&mut host, end, false, 65536);
    assert!(next.source.end().unwrap() > end);
    let previous = viewport(&mut host, end, true, 65536);
    assert!(previous.source.offset < next.source.offset);
    assert!(previous.blocks.iter().all(|block| block.table.is_none()));
}

#[test]
fn a_large_budget_backward_seek_keeps_the_anchor_inside_the_bounded_read() {
    let input = "word ".repeat(80_000);
    let mut host = open(&input);
    let page = viewport(&mut host, 300_000, true, 65536);
    assert!(page.source.end().unwrap() <= 300_000);
    assert!(page.source.end().unwrap() > 290_000);
    assert!(
        page.blocks
            .iter()
            .map(|block| block.text.text.len())
            .sum::<usize>()
            <= 8192
    );
    assert!(host.reads().iter().all(|read| read.length <= 65536));
}
