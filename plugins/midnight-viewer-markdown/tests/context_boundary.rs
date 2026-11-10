//! Complete semantic groups must survive bounded backward read-window planning.
#[allow(dead_code)]
mod support;

use midnight_plugin_sdk::api::{abi, codec, document::DocumentPage, messages::*, viewer::*};
use midnight_viewer_markdown::midnight_plugin_entry_v1 as entry;
use support::Host;

fn viewport(host: &mut Host, anchor: u64, backwards: bool) -> DocumentPage {
    host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: anchor,
            backwards,
            max_bytes: 8192,
            max_lines: 64,
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

#[test]
fn trusted_checkpoint_inside_backward_prefetch_preserves_complete_table() {
    let mut input = "\n".repeat(32_760);
    input.push_str("# Seed\n\n");
    input.push_str(&"\n".repeat(65_000 - input.len()));
    input.push_str("# Heading\n\n");
    let table_start = input.len() as u64;
    input.push_str("| Name | Value |\n| --- | --- |\n| first | one |\n| second | ");
    input.push_str(&"long ".repeat(180));
    input.push_str("|\n| last | final witness |\n\nAfter.\n");
    assert!(input.len() > 65_536);

    let mut host = Host::new(input.as_bytes(), entry);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 3,
            length: Some(input.len() as u64),
            mode: MODE_DOCUMENT,
            display_name: "context-boundary.md".into(),
        },
    );
    assert!(
        host.finish()
            .iter()
            .any(|event| event.kind == abi::EVENT_OPENED)
    );

    // The first bounded read records the completed Seed heading near byte32767.
    // A backward read at65520 must prefer this trusted boundary over byte0:
    // the latter leaves only16 bytes after the anchor in the64KiB read cap.
    assert_eq!(viewport(&mut host, 0, false).blocks[0].text.text, "Seed");
    let backwards = viewport(&mut host, 65_520, true);
    let forwards = viewport(&mut host, table_start, false);
    let expected: Vec<_> = forwards
        .blocks
        .iter()
        .filter(|block| block.table.is_some())
        .collect();
    let actual: Vec<_> = backwards
        .blocks
        .iter()
        .filter(|block| block.table.is_some())
        .collect();
    assert_eq!(expected.len(), 4);
    assert!(expected.last().unwrap().text.text.contains("final witness"));
    assert_eq!(
        actual, expected,
        "the backward raw-window cap must not truncate a small table"
    );
}
