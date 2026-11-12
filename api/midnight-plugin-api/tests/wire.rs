use midnight_plugin_api::wire::{MAX_DEPTH, MAX_ITEMS, MAX_MESSAGE_BYTES, validate};

#[test]
fn definite_numeric_maps_and_exact_u64_offsets_are_accepted() {
    // {0: 1, 1: [4294967296, "文", h'00ff'], 7: {0: true}}
    let bytes = [
        0xa3, 0x00, 0x01, 0x01, 0x83, 0x1b, 0, 0, 0, 1, 0, 0, 0, 0, 0x63, 0xe6, 0x96, 0x87, 0x42,
        0, 255, 0x07, 0xa1, 0, 0xf5,
    ];
    assert!(validate(&bytes).is_ok());
}

#[test]
fn malformed_lengths_indefinite_values_and_ambiguous_maps_are_refused() {
    for bytes in [
        &[][..],
        &[0xa1, 0],
        &[0x5b, 255, 255, 255, 255, 255, 255, 255, 255],
        &[0x9f, 1, 2, 0xff],
        &[0xbf, 0, 1, 0xff],
        &[0x7f, 0x61, b'a', 0xff],
        &[0xa2, 0, 1, 0, 2],
        &[0xa2, 1, 1, 0, 2],
        &[0xa1, 0x61, b'x', 1],
        &[0x61, 0xff],
        &[0xa0, 0],
        &[0xc0, 0x61, b'x'],
        &[0xfa, 0x7f, 0xc0, 0, 0],
    ] {
        assert!(validate(bytes).is_err(), "accepted {bytes:x?}");
    }
}

#[test]
fn input_depth_and_collection_counts_are_checked_without_large_allocations() {
    assert!(validate(&vec![0; MAX_MESSAGE_BYTES + 1]).is_err());
    let mut deep = vec![0x81; MAX_DEPTH + 1];
    deep.push(0);
    assert!(validate(&deep).is_err());
    let mut many = vec![0x9a];
    many.extend_from_slice(&(MAX_ITEMS as u32 + 1).to_be_bytes());
    // Count alone is sufficient for refusal; no attacker-sized allocation.
    assert!(validate(&many).is_err());
}

#[test]
fn every_short_byte_pattern_is_handled_without_panics_or_unbounded_recursion() {
    for first in 0..=u8::MAX {
        let _ = validate(&[first]);
        for second in 0..=u8::MAX {
            let _ = validate(&[first, second]);
        }
    }
}
