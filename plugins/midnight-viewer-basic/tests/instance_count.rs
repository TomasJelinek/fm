//! From SDK 0.2.1 a loaded library has no instance count limit (0.1.0 and
//! 0.2.0 refused a 17th live instance with `QUEUE_FULL`).
#[allow(dead_code)]
mod harness;
use harness::Host;
use midnight_plugin_sdk::api::{abi, viewer};

#[test]
fn forty_hosts_of_this_plugin_are_open_at_once() {
    let mut hosts: Vec<Host> = (0..40)
        .map(|index| Host::new(format!("document {index}").into_bytes()))
        .collect();
    for host in &mut hosts {
        host.open(viewer::MODE_TEXT);
        host.event(1, abi::EVENT_OPENED);
    }
}
