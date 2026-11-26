//! The SDK admits 16 live plugin instances per process, fewer than the tests a
//! binary may run at once. The test host queues a further host until a slot is
//! released instead of failing it with QUEUE_FULL. Keep this binary to this one
//! test: its hosts must be the only ones in the process.
#[allow(dead_code)]
mod harness;
use harness::Host;
use midnight_plugin_sdk::api::{abi, viewer};

#[test]
fn a_host_beyond_the_sdk_instance_limit_waits_for_a_released_slot() {
    let mut held: Vec<Host> = (0..16).map(|_| Host::new(b"held".to_vec())).collect();
    let late = std::thread::spawn(|| {
        let mut host = Host::new(b"late".to_vec());
        host.open(viewer::MODE_TEXT);
        host.event(1, abi::EVENT_OPENED);
    });
    harness::await_slot_waiters(1);
    drop(held.pop());
    late.join()
        .expect("the late host is created once a slot is released");
}
