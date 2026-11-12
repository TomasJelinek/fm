#![cfg(windows)]
//! Opt-in native root enumeration through the real public ABI; never opens a child.
mod support;
use midnight_filesystem_network::{api::*, windows::Windows};
use std::collections::HashSet;
use std::time::{Duration, Instant};

#[test]
#[ignore = "explicit native MPR provider enumeration; no child discovery or credentials"]
fn native_provider_root_publishes_promptly_through_the_public_abi() {
    let client = support::Client::new::<Windows>();
    let (mount, root) = client.root();
    let started = Instant::now();
    let mut response = client.call(Request::ListBegin {
        mount,
        directory: root.id,
        max_entries: 1,
    });
    let mut ordinal = 0;
    let mut expected_cursor = None;
    let mut addresses = HashSet::new();
    loop {
        let Response::Page {
            cursor,
            page,
            done,
            entries,
        } = response
        else {
            panic!("Native provider root failed: {response:?}");
        };
        assert_eq!(page, ordinal);
        assert_ne!(cursor, 0);
        assert_eq!(*expected_cursor.get_or_insert(cursor), cursor);
        for node in entries {
            assert_eq!(node.kind, Kind::Directory);
            assert_eq!(node.address.encoding, AddressEncoding::Opaque);
            assert!(!node.display.is_empty());
            assert!(addresses.insert(node.address.bytes));
            println!("provider: {}", node.display);
        }
        if done {
            assert!(matches!(
                client.call(Request::CloseList { cursor }),
                Response::Unit
            ));
            break;
        }
        ordinal += 1;
        assert!(
            ordinal <= 4096,
            "provider enumeration must make bounded progress"
        );
        response = client.call(Request::ListNext {
            cursor,
            expected_page: ordinal,
            max_entries: 128,
        });
    }
    let elapsed = started.elapsed();
    println!("{} provider containers in {elapsed:?}", addresses.len());
    assert!(
        !addresses.is_empty(),
        "this qualification needs installed providers"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "root enumeration took {elapsed:?}"
    );
    assert!(matches!(
        client.call(Request::CloseMount { mount }),
        Response::Unit
    ));
}
