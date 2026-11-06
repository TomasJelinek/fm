use midnight_plugin_api::{codec, viewer::*};

const EMPTY_PAGE: &[u8] = &[
    0xa8, 0, 1, 1, 0xa2, 0, 0, 1, 0, 2, 0x60, 3, 0x80, 4, 0x81, 0xa2, 0, 0, 1, 0, 5, 0xf4, 6, 0xf4,
    7, 0xf4,
];

#[test]
fn cbor_wire_tags_match_the_published_fixture() {
    let page: TextPage = codec::decode(EMPTY_PAGE).unwrap();
    assert_eq!(page.revision, 1);
    assert_eq!(
        page.source,
        SourceRange {
            offset: 0,
            length: 0
        }
    );
    assert!(page.text.is_empty());
    assert!(!page.has_after);
    assert_eq!(codec::encode(&page).unwrap(), EMPTY_PAGE);
}

#[test]
fn codec_validates_decoded_ranges_and_rejects_oversized_text_before_encoding() {
    let mut page: TextPage = codec::decode(EMPTY_PAGE).unwrap();
    page.text = "文🙂".into();
    page.source = SourceRange {
        offset: (1u64 << 40) + 3,
        length: 7,
    };
    page.anchors = vec![
        Anchor {
            text_offset: 0,
            source_offset: (1u64 << 40) + 3,
        },
        Anchor {
            text_offset: 7,
            source_offset: (1u64 << 40) + 10,
        },
    ];
    let encoded = codec::encode(&page).unwrap();
    let decoded: TextPage = codec::decode(&encoded).unwrap();
    assert_eq!(decoded.source.offset, 1_099_511_627_779);
    assert_eq!(decoded.text, "文🙂");
    page.spans.push(Span {
        start: 1,
        end: 2,
        role: STYLE_PLAIN,
        flags: 0,
    });
    let invalid = minicbor::to_vec(&page).unwrap();
    assert!(codec::decode::<TextPage>(&invalid).is_err());
    page.text = "a".repeat(MAX_PAGE_BYTES + 1);
    assert!(codec::encode(&page).is_err());
}

#[test]
fn malformed_unknown_fields_cannot_bypass_preflight_limits() {
    let mut nested = EMPTY_PAGE.to_vec();
    nested[0] = 0xa9;
    nested.push(20);
    nested.extend(std::iter::repeat_n(0x81, 100));
    nested.push(0);
    assert!(codec::decode::<TextPage>(&nested).is_err());
    let mut optional = EMPTY_PAGE.to_vec();
    optional[0] = 0xa9;
    optional.extend_from_slice(&[20, 0xa1, 0, 0xf5]);
    assert!(codec::decode::<TextPage>(&optional).is_ok());
}
