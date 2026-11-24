use midnight_filesystem_plugin_api::*;

// Hand-written FSBR frame: lease 1, node 2, username "Zoë", empty domain, password "p@ss".
const FRAME: &[u8] = b"FSBR\x01\0\0\0\x01\0\0\0\0\0\0\0\x02\0\0\0\0\0\0\0\x04\0\0\0\0\0\0\0Zo\xc3\xab\0\0\0\0\0\0\0\0\x04\0\0\0\0\0\0\0p@ss";

#[test]
fn sign_in_request_round_trips_the_published_wire_shape_without_debug_secrets() {
    let request = decode_request(24, FRAME, FRAME.len()).unwrap();
    assert_eq!(request.operation() as u32, 24);
    assert_eq!(encode_request(&request, FRAME.len()).unwrap(), FRAME);
    assert_eq!(
        encode_request(&request, FRAME.len() - 1),
        Err(WireError::Limit)
    );
    let diagnostic = format!("{request:?}");
    assert!(!diagnostic.contains("p@ss"));
    assert!(!diagnostic.contains("Zoë"));
}

#[test]
fn sign_in_rejects_bad_fields_and_trailing_data() {
    for location in [8, 16, FRAME.len() - 1] {
        let mut bytes = FRAME.to_vec();
        bytes[location] = 0;
        assert_eq!(decode_request(24, &bytes, 1024), Err(WireError::Malformed));
    }
    let mut bytes = FRAME.to_vec();
    *bytes.last_mut().unwrap() = 0xff;
    assert_eq!(decode_request(24, &bytes, 1024), Err(WireError::Malformed));

    let mut bytes = FRAME.to_vec();
    bytes.push(0);
    assert_eq!(decode_request(24, &bytes, 1024), Err(WireError::Malformed));

    let mut bytes = FRAME.to_vec();
    bytes[24..32].copy_from_slice(&4097_u64.to_le_bytes());
    assert_eq!(decode_request(24, &bytes, 1024), Err(WireError::Limit));
}
