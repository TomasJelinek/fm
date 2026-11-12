#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, messages::Open, viewer::MODE_SOURCE};
use midnight_viewer_source::midnight_plugin_entry_v1 as entry;
use support::Host;

#[test]
fn optional_locate_request_returns_source_coordinates_without_a_viewport() {
    let mut host = Host::new(b"first\nsecond", entry);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            length: Some(12),
            revision: 3,
            mode: MODE_SOURCE,
            display_name: "test.rs".into(),
        },
    );
    host.finish();
    // Optional v1 request, deliberately wire-level so the old SDK fails at the
    // behavior boundary rather than merely failing to compile a new type name.
    let locate = host.submit_bytes(6, &[0xa1, 0, 6]);
    let events = host.finish();
    assert!(
        events
            .iter()
            .any(|event| event.id == locate && event.kind == 7)
    );
}
