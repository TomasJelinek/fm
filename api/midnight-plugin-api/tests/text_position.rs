use midnight_plugin_api::{codec, messages::Locate, viewer::TextPosition};

#[test]
fn optional_positions_preserve_u64_offsets_and_require_one_based_coordinates() {
    let query = Locate {
        source_anchor: (1 << 40) + 123,
    };
    assert_eq!(
        codec::decode::<Locate>(&codec::encode(&query).unwrap()).unwrap(),
        query
    );
    let position = TextPosition {
        revision: 9,
        source_offset: query.source_anchor,
        line: (1 << 35) + 8,
        column: (1 << 33) + 5,
    };
    assert_eq!(
        codec::decode::<TextPosition>(&codec::encode(&position).unwrap()).unwrap(),
        position
    );
    assert!(
        codec::encode(&TextPosition {
            line: 0,
            ..position
        })
        .is_err()
    );
    assert!(
        codec::encode(&TextPosition {
            column: 0,
            ..position
        })
        .is_err()
    );
}
