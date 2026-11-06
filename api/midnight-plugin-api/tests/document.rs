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
