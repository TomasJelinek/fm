use midnight_viewer_source::decode::{Encoding, decode, encoding};

#[test]
fn exact_utf8_malformed_control_and_newline_anchors_above_four_gib() {
    let offset = 1_u64 << 35;
    let decoded = decode(
        b"A\xc3\xa9\xff\x1b\r\nZ",
        offset,
        Encoding::Utf8,
        true,
        8192,
        30,
        || false,
    )
    .unwrap();
    assert_eq!(decoded.text, "Aé�␛\nZ");
    assert_eq!(
        decoded
            .anchors
            .iter()
            .map(|a| (a.text_offset, a.source_offset - offset))
            .collect::<Vec<_>>(),
        vec![(0, 0), (1, 1), (3, 3), (6, 4), (9, 5), (10, 7), (11, 8)]
    );
    decoded.page(9, true).validate().unwrap();
}

#[test]
fn utf16_surrogate_pairs_and_malformed_units_are_exact() {
    assert_eq!(encoding(&[0xff, 0xfe]), (Encoding::Le, 2));
    assert_eq!(encoding(&[0xfe, 0xff]), (Encoding::Be, 2));
    let bytes = [0x41, 0, 0x3d, 0xd8, 0, 0xde, 0, 0xd8, 0x42, 0];
    let decoded = decode(&bytes, 2, Encoding::Le, true, 8192, 30, || false).unwrap();
    assert_eq!(decoded.text, "A😀�B");
    assert_eq!(
        decoded
            .anchors
            .iter()
            .map(|a| a.source_offset)
            .collect::<Vec<_>>(),
        [2, 4, 8, 10, 12]
    );
    decoded.page(1, true).validate().unwrap();
}

#[test]
fn partial_units_and_crlf_are_deferred_and_huge_lines_are_bounded() {
    let decoded = decode(b"A\xf0\x9f", 0, Encoding::Utf8, false, 8192, 30, || false).unwrap();
    assert_eq!((decoded.text.as_str(), decoded.consumed), ("A", 1));
    let decoded = decode(b"A\r", 0, Encoding::Utf8, false, 8192, 30, || false).unwrap();
    assert_eq!((decoded.text.as_str(), decoded.consumed), ("A", 1));
    let decoded = decode(
        &vec![b'x'; 200_000],
        0,
        Encoding::Utf8,
        false,
        8192,
        30,
        || false,
    )
    .unwrap();
    assert_eq!(decoded.text.len(), 8192);
    assert!(decoded.page(1, false).partial_line);
    assert!(decode(b"anything", 0, Encoding::Utf8, true, 8192, 30, || true).is_none());
    assert!(
        decode(
            b"overflow",
            u64::MAX,
            Encoding::Utf8,
            true,
            8192,
            30,
            || false
        )
        .is_none()
    );
}
