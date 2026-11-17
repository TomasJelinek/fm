use super::*;
use crate::decode::Encoding;
use midnight_plugin_sdk::api::codec::Model;

fn raw_page(text: &str) -> DocumentPage {
    let decoded = decode::decode(
        text.as_bytes(),
        100,
        Encoding::Utf8,
        true,
        65536,
        usize::MAX,
        || false,
    )
    .unwrap();
    plain(&decoded, 7, true)
}

fn full_window(page: &DocumentPage, backwards: bool) -> Window {
    Window {
        anchor: if backwards {
            page.source.end().unwrap()
        } else {
            page.source.offset
        },
        backwards,
        max_bytes: 65536,
        max_lines: 100,
    }
}

fn decorate(page: &mut DocumentPage) {
    let block = &mut page.blocks[0];
    block.text.spans = (0..MAX_SPANS)
        .map(|at| Span {
            start: (at * 2) as u32,
            end: (at * 2 + 1) as u32,
            role: STYLE_KEYWORD,
            flags: 1,
        })
        .collect();
    block.links.push(Link {
        start: 0,
        end: 1,
        target: "https://example.org".into(),
    });
    block.strikethrough = Some(vec![TextRange { start: 2, end: 3 }]);
    block.table = Some(TableRow {
        cells: vec![TableCell {
            start: 0,
            end: block.text.text.len() as u32,
            alignment: TABLE_ALIGN_LEFT,
        }],
        header: true,
    });
}

#[test]
fn a_successful_context_check_preserves_the_exact_rich_page_in_both_directions() {
    let input = "# Café\n\nText **bold** ~~old~~ [link](https://example.org).\n\n\
                 | Left | Right |\n| --- | --- |\n| one | two |\n";
    let decoded = decode::decode(
        input.as_bytes(),
        100,
        Encoding::Utf8,
        true,
        65536,
        usize::MAX,
        || false,
    )
    .unwrap();
    let (raw, _) = render_context(&decoded, 7, true, || false).unwrap();
    assert!(raw.blocks.iter().any(|block| !block.text.spans.is_empty()));
    assert!(raw.blocks.iter().any(|block| !block.links.is_empty()));
    assert!(raw.blocks.iter().any(|block| block.strikethrough.is_some()));
    assert!(raw.blocks.iter().any(|block| block.table.is_some()));
    let encoded = codec::encode(&raw).unwrap();
    for backwards in [false, true] {
        let actual = crop_context(&raw, &full_window(&raw, backwards)).unwrap();
        assert_eq!(
            actual, raw,
            "text, exact anchors and all metadata must survive"
        );
        assert_eq!(codec::encode(&actual).unwrap(), encoded);
    }
}

#[test]
fn a_check_that_fits_after_removing_decorations_returns_that_mutated_page() {
    let mut raw = raw_page(&"x".repeat(8192));
    decorate(&mut raw);
    raw.validate().unwrap();
    assert_eq!(codec::encode(&raw), Err(WireError::Limit));
    let original = raw.clone();
    let mut expected = raw.clone();
    expected.blocks[0].text.spans.clear();
    expected.blocks[0].links.clear();
    expected.blocks[0].strikethrough = None;
    codec::encode(&expected).unwrap();
    for backwards in [false, true] {
        let actual = crop_context(&raw, &full_window(&raw, backwards)).unwrap();
        assert_eq!(
            actual, expected,
            "successful mutation must not be replaced by the raw page"
        );
        assert_eq!(
            actual.blocks[0].text.anchors,
            original.blocks[0].text.anchors
        );
        assert!(
            actual.blocks[0].table.is_some(),
            "the successful check must retain table metadata"
        );
    }
    assert_eq!(raw, original, "the borrowed parse context stays unchanged");
}

#[test]
fn a_context_still_over_the_wire_limit_keeps_its_directional_scalar_fallback() {
    let mut raw = raw_page(&"x".repeat(14000));
    decorate(&mut raw);
    raw.validate().unwrap();
    let original = raw.clone();
    let mut undecorated = raw.clone();
    undecorated.blocks[0].text.spans.clear();
    undecorated.blocks[0].links.clear();
    undecorated.blocks[0].strikethrough = None;
    assert_eq!(codec::encode(&undecorated), Err(WireError::Limit));
    for backwards in [false, true] {
        let actual = crop_context(&raw, &full_window(&raw, backwards)).unwrap();
        let selected = if backwards {
            14000 - 8192..14000
        } else {
            0..8192
        };
        let block = &actual.blocks[0];
        assert_eq!(actual.blocks.len(), 1);
        assert_eq!(
            block.text.text.as_str(),
            &raw.blocks[0].text.text[selected.clone()]
        );
        let expected_anchors: Vec<_> = original.blocks[0].text.anchors
            [selected.start..=selected.end]
            .iter()
            .map(|anchor| Anchor {
                text_offset: anchor.text_offset - selected.start as u32,
                source_offset: anchor.source_offset,
            })
            .collect();
        assert_eq!(block.text.anchors, expected_anchors);
        assert_eq!(
            actual.source,
            SourceRange {
                offset: 100 + selected.start as u64,
                length: 8192,
            }
        );
        assert_eq!(block.text.source, actual.source);
        assert!(block.text.spans.is_empty() && block.links.is_empty());
        assert!(block.strikethrough.is_none() && block.table.is_none());
        assert!(actual.has_before);
        assert_eq!(actual.has_after, !backwards);
        assert!(codec::encode(&actual).is_ok());
    }
    assert_eq!(raw, original);
}

#[test]
fn a_malformed_context_remains_rejected_after_the_fallback_check() {
    let mut raw = raw_page("ordinary text");
    raw.blocks[0].text.revision += 1;
    let original = raw.clone();
    assert_eq!(codec::encode(&raw), Err(WireError::Malformed));
    for backwards in [false, true] {
        assert!(crop_context(&raw, &full_window(&raw, backwards)).is_none());
    }
    assert_eq!(raw, original);
}
