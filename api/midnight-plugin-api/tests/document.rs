use midnight_plugin_api::{
    codec::{self, Model},
    document::*,
    viewer::*,
};

fn document() -> DocumentPage {
    let text = TextPage {
        revision: 1,
        source: SourceRange {
            offset: 100,
            length: 6,
        },
        text: "文档".into(),
        spans: vec![],
        anchors: vec![
            Anchor {
                text_offset: 0,
                source_offset: 100,
            },
            Anchor {
                text_offset: 6,
                source_offset: 106,
            },
        ],
        has_before: false,
        has_after: false,
        partial_line: false,
    };
    DocumentPage {
        revision: 1,
        source: SourceRange {
            offset: 100,
            length: 6,
        },
        blocks: vec![Block {
            table: None,
            strikethrough: None,
            kind: BLOCK_HEADING,
            depth: 0,
            heading_level: 2,
            text,
            links: vec![Link {
                start: 0,
                end: 6,
                target: "https://example.org/".into(),
            }],
        }],
        has_before: true,
        has_after: false,
    }
}

#[test]
fn document_blocks_keep_valid_source_revisions_and_utf8_link_ranges() {
    let valid = document();
    assert_eq!(
        codec::decode::<DocumentPage>(&codec::encode(&valid).unwrap()).unwrap(),
        valid
    );
    let mut invalid = document();
    invalid.blocks[0].text.revision = 2;
    assert!(invalid.validate().is_err());
    let mut invalid = document();
    invalid.blocks[0].links[0].start = 1;
    assert!(invalid.validate().is_err());
    let mut invalid = document();
    invalid.blocks[0].links[0].target.push('\0');
    assert!(invalid.validate().is_err());
    let mut invalid = document();
    invalid.source.length = 5;
    assert!(invalid.validate().is_err());
}

#[test]
fn document_depth_blocks_and_combined_text_are_bounded() {
    let mut invalid = document();
    invalid.blocks[0].depth = 17;
    assert!(invalid.validate().is_err());
    let mut invalid = document();
    invalid.blocks[0].heading_level = 7;
    assert!(invalid.validate().is_err());
    let mut invalid = document();
    invalid.blocks = vec![invalid.blocks[0].clone(); MAX_BLOCKS + 1];
    assert!(invalid.validate().is_err());
    let mut invalid = document();
    invalid.blocks[0].text.text = "x".repeat(MAX_PAGE_BYTES + 1);
    assert!(invalid.validate().is_err());
    let mut invalid = document();
    invalid.source = SourceRange {
        offset: 0,
        length: 300_000,
    };
    invalid.blocks = (0..2)
        .map(|index| {
            let offset = index * 150_000;
            let mut block = document().blocks.remove(0);
            block.text.source = SourceRange {
                offset,
                length: 150_000,
            };
            block.text.text = "x".repeat(150_000);
            block.text.anchors = vec![
                Anchor {
                    text_offset: 0,
                    source_offset: offset,
                },
                Anchor {
                    text_offset: 150_000,
                    source_offset: offset + 150_000,
                },
            ];
            block.links.clear();
            block
        })
        .collect();
    assert!(
        invalid.validate().is_err(),
        "several individually valid blocks share one page budget"
    );
}

#[test]
fn binary_page_metadata_never_requests_oversized_or_wrapped_content() {
    let valid = BinaryPage {
        revision: 1,
        source: SourceRange {
            offset: 1 << 40,
            length: 256 * 1024,
        },
        has_before: true,
        has_after: true,
    };
    assert!(codec::encode(&valid).is_ok());
    assert!(
        BinaryPage {
            source: SourceRange {
                offset: 0,
                length: u64::MAX
            },
            ..valid.clone()
        }
        .validate()
        .is_err()
    );
    assert!(
        BinaryPage {
            source: SourceRange {
                offset: u64::MAX,
                length: 1
            },
            ..valid
        }
        .validate()
        .is_err()
    );
}

// Encode the additive metadata explicitly so this also exercises the wire boundary.
fn encoded_table_cell(start: u32, end: u32, alignment: u8) -> Vec<u8> {
    let page = document();
    let block = &page.blocks[0];
    let mut encoder = minicbor::Encoder::new(Vec::new());
    encoder.map(5).unwrap().u8(0).unwrap().u64(1).unwrap();
    encoder.u8(1).unwrap().encode(&page.source).unwrap();
    encoder.u8(2).unwrap().array(1).unwrap().map(6).unwrap();
    encoder.u8(0).unwrap().u16(BLOCK_PARAGRAPH).unwrap();
    encoder.u8(1).unwrap().u8(0).unwrap();
    encoder.u8(2).unwrap().u8(0).unwrap();
    encoder.u8(3).unwrap().encode(&block.text).unwrap();
    encoder.u8(4).unwrap().array(0).unwrap();
    encoder.u8(5).unwrap().map(2).unwrap();
    encoder.u8(0).unwrap().array(1).unwrap().map(3).unwrap();
    encoder.u8(0).unwrap().u32(start).unwrap();
    encoder.u8(1).unwrap().u32(end).unwrap();
    encoder.u8(2).unwrap().u8(alignment).unwrap();
    encoder.u8(1).unwrap().bool(true).unwrap();
    encoder.u8(3).unwrap().bool(false).unwrap();
    encoder.u8(4).unwrap().bool(false).unwrap();
    encoder.into_writer()
}

#[test]
fn table_metadata_rejects_invalid_utf8_ranges_and_alignment_at_wire_boundary() {
    assert!(codec::decode::<DocumentPage>(&encoded_table_cell(0, 6, 0)).is_ok());
    for (start, end, alignment) in [(1, 6, 0), (0, 7, 0), (6, 0, 0), (0, 6, 255)] {
        assert!(
            codec::decode::<DocumentPage>(&encoded_table_cell(start, end, alignment)).is_err(),
            "invalid table cell {start}..{end} with alignment {alignment} must be rejected"
        );
    }
}

#[test]
fn table_and_decoration_metadata_remain_optional_and_bounded() {
    let mut page = codec::decode::<DocumentPage>(&encoded_table_cell(0, 6, 0)).unwrap();
    assert_eq!(page.blocks[0].table.as_ref().unwrap().cells[0].end, 6);
    assert!(page.blocks[0].strikethrough.is_none());
    page.blocks[0].strikethrough = Some(vec![TextRange { start: 0, end: 6 }]);
    assert_eq!(
        codec::decode::<DocumentPage>(&codec::encode(&page).unwrap()).unwrap(),
        page
    );
    for ranges in [
        vec![TextRange { start: 1, end: 6 }],
        vec![TextRange { start: 0, end: 0 }],
        vec![
            TextRange { start: 0, end: 6 },
            TextRange { start: 3, end: 6 },
        ],
    ] {
        let mut invalid = page.clone();
        invalid.blocks[0].strikethrough = Some(ranges);
        assert!(invalid.validate().is_err());
    }
    let mut invalid = page.clone();
    invalid.blocks[0].table.as_mut().unwrap().cells = vec![
        TableCell {
            start: 0,
            end: 0,
            alignment: TABLE_ALIGN_LEFT,
        };
        MAX_TABLE_COLUMNS + 1
    ];
    assert!(invalid.validate().is_err());
    let mut invalid = page.clone();
    invalid.blocks[0]
        .table
        .as_mut()
        .unwrap()
        .cells
        .push(TableCell {
            start: 3,
            end: 6,
            alignment: TABLE_ALIGN_LEFT,
        });
    assert!(
        invalid.validate().is_err(),
        "overlapping cells must be rejected"
    );
    let mut invalid = page.clone();
    invalid.blocks[0].kind = BLOCK_CODE;
    assert!(invalid.validate().is_err());
    let mut invalid = page.clone();
    invalid.blocks[0].strikethrough = Some(vec![TextRange { start: 0, end: 6 }; MAX_SPANS + 1]);
    assert!(invalid.validate().is_err());
    let mut block = page.blocks[0].clone();
    block.table.as_mut().unwrap().cells = vec![
        TableCell {
            start: 0,
            end: 0,
            alignment: TABLE_ALIGN_LEFT,
        };
        MAX_TABLE_COLUMNS
    ];
    page.blocks = (0..65)
        .map(|index| {
            let mut block = block.clone();
            block.text.source.offset = 100 + index * 6;
            for anchor in &mut block.text.anchors {
                anchor.source_offset += index * 6;
            }
            block
        })
        .collect();
    page.source.length = 65 * 6;
    assert!(
        page.validate().is_err(),
        "all rows share the page cell budget"
    );
}
