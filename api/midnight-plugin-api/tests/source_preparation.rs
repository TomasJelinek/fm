use midnight_plugin_api::{
    codec,
    messages::{PrepareSource, SourcePrepared, Viewport},
    wire::WireError,
};

fn prepare() -> PrepareSource {
    PrepareSource {
        revision: 9,
        viewport: Viewport {
            source_anchor: 23,
            backwards: true,
            max_bytes: 24,
            max_lines: 2,
        },
        max_scan_bytes: 262_144,
    }
}

#[test]
fn preparation_uses_a_definite_map_with_the_complete_nested_viewport() {
    let bytes = [
        0xa3, 0x00, 0x09, 0x01, 0xa4, 0x00, 0x17, 0x01, 0xf5, 0x02, 0x18, 0x18, 0x03, 0x02, 0x02,
        0x1a, 0x00, 0x04, 0x00, 0x00,
    ];
    assert_eq!(codec::encode(&prepare()).unwrap(), bytes);
    assert_eq!(codec::decode::<PrepareSource>(&bytes).unwrap(), prepare());

    let mut indefinite = bytes.to_vec();
    indefinite[0] = 0xbf;
    indefinite.push(0xff);
    assert_eq!(
        codec::decode::<PrepareSource>(&indefinite),
        Err(WireError::Unsupported)
    );
}

#[test]
fn preparation_preserves_u64_revision_and_directional_target() {
    let mut request = prepare();
    request.revision = (1 << 40) + 9;
    request.viewport.source_anchor = (1 << 52) + 17;
    request.viewport.max_bytes = 262_144;
    request.viewport.max_lines = 4096;
    request.max_scan_bytes = 1;
    assert_eq!(
        codec::decode::<PrepareSource>(&codec::encode(&request).unwrap()).unwrap(),
        request
    );
}

#[test]
fn preparation_validates_scan_budget_on_encode_and_decode() {
    for max_scan_bytes in [0, 262_145, u32::MAX] {
        let request = PrepareSource {
            max_scan_bytes,
            ..prepare()
        };
        assert_eq!(codec::encode(&request), Err(WireError::Limit));
        let bytes = minicbor::to_vec(&request).unwrap();
        assert_eq!(
            codec::decode::<PrepareSource>(&bytes),
            Err(WireError::Limit)
        );
    }
}

#[test]
fn preparation_validates_the_nested_viewport_on_encode_and_decode() {
    for (max_bytes, max_lines) in [(0, 1), (262_145, 1), (1, 0), (1, 4097)] {
        let mut request = prepare();
        request.viewport.max_bytes = max_bytes;
        request.viewport.max_lines = max_lines;
        assert_eq!(codec::encode(&request), Err(WireError::Limit));
        let bytes = minicbor::to_vec(&request).unwrap();
        assert_eq!(
            codec::decode::<PrepareSource>(&bytes),
            Err(WireError::Limit)
        );
    }
}

#[test]
fn prepared_result_preserves_exact_frontier_and_explicit_readiness() {
    let ready = SourcePrepared {
        revision: 9,
        frontier: 23,
        ready: true,
    };
    let bytes = [0xa3, 0x00, 0x09, 0x01, 0x17, 0x02, 0xf5];
    assert_eq!(codec::encode(&ready).unwrap(), bytes);
    assert_eq!(codec::decode::<SourcePrepared>(&bytes).unwrap(), ready);

    let partial = SourcePrepared {
        revision: u64::MAX,
        frontier: u64::MAX,
        ready: false,
    };
    assert_eq!(
        codec::decode::<SourcePrepared>(&codec::encode(&partial).unwrap()).unwrap(),
        partial
    );
    assert_eq!(
        codec::decode::<SourcePrepared>(&[0xa3, 0x00, 0x09, 0x01, 0x17, 0x02, 0x01]),
        Err(WireError::Malformed)
    );
}
