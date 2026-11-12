#[path = "network_support/mod.rs"]
mod support;
use midnight_filesystem_network::{adapter::*, api::*};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};
static SERIAL: Mutex<()> = Mutex::new(());
static COUNT: AtomicUsize = AtomicUsize::new(257);
static FAIL_AT: AtomicUsize = AtomicUsize::new(usize::MAX);
static BEGINS: AtomicUsize = AtomicUsize::new(0);
static NEXTS: AtomicUsize = AtomicUsize::new(0);
static DROPS: AtomicUsize = AtomicUsize::new(0);
static BEGIN_DEADLINE: Mutex<Option<Instant>> = Mutex::new(None);
static NEXT_DEADLINES: Mutex<Vec<Instant>> = Mutex::new(Vec::new());
struct Fixture;
struct Cursor {
    index: usize,
    count: usize,
    fail_at: usize,
}
impl Drop for Cursor {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Ordering::SeqCst);
    }
}
impl Adapter for Fixture {
    type Cursor = Cursor;
    fn root() -> Record {
        Record {
            target: Target {
                kind: NodeKind::Root,
                key: b"root".to_vec(),
            },
            display: "Network".into(),
        }
    }
    fn validate(_: &Target) -> Result<(), Failure> {
        Ok(())
    }
    fn manual(text: &str) -> Result<Record, Failure> {
        Ok(Record {
            target: Target {
                kind: NodeKind::Server,
                key: text.as_bytes().to_vec(),
            },
            display: text.into(),
        })
    }
    fn begin(_: &Target, context: &Context<'_>) -> Result<Cursor, Failure> {
        context.check()?;
        *BEGIN_DEADLINE.lock().unwrap() = Some(context.deadline);
        BEGINS.fetch_add(1, Ordering::SeqCst);
        if COUNT.load(Ordering::SeqCst) == usize::MAX {
            return Err(Failure::new(ErrorCode::Unavailable, "No fixture backend."));
        }
        Ok(Cursor {
            index: 0,
            count: COUNT.load(Ordering::SeqCst),
            fail_at: FAIL_AT.load(Ordering::SeqCst),
        })
    }
    fn next(cursor: &mut Cursor, context: &Context<'_>) -> Result<Option<Record>, Failure> {
        context.check()?;
        NEXT_DEADLINES.lock().unwrap().push(context.deadline);
        NEXTS.fetch_add(1, Ordering::SeqCst);
        if cursor.index == cursor.fail_at {
            return Err(Failure::new(ErrorCode::Offline, "Fixture disconnected."));
        }
        if cursor.index == cursor.count {
            return Ok(None);
        }
        let index = cursor.index;
        cursor.index += 1;
        Ok(Some(Record {
            target: Target {
                kind: NodeKind::Server,
                key: format!("server-{index}").into_bytes(),
            },
            display: "Same caption".into(),
        }))
    }
    fn activate(target: &Target, _: &Context<'_>) -> Result<ActivationTarget, Failure> {
        Ok(ActivationTarget::WithinProvider(Address {
            encoding: AddressEncoding::Opaque,
            bytes: target.key.clone(),
        }))
    }
}
fn reset(count: usize, fail: usize) {
    COUNT.store(count, Ordering::SeqCst);
    FAIL_AT.store(fail, Ordering::SeqCst);
    BEGINS.store(0, Ordering::SeqCst);
    NEXTS.store(0, Ordering::SeqCst);
    DROPS.store(0, Ordering::SeqCst);
    *BEGIN_DEADLINE.lock().unwrap() = None;
    NEXT_DEADLINES.lock().unwrap().clear();
}
fn page(response: Response) -> (u64, u64, bool, Vec<Node>) {
    let Response::Page {
        cursor,
        page,
        done,
        entries,
    } = response
    else {
        panic!("{response:?}")
    };
    (cursor, page, done, entries)
}
fn failed(response: Response, expected: ErrorCode) {
    assert!(matches!(response,Response::Failed(Failure {code,..}) if code==expected));
}

#[test]
fn first_page_uses_the_begin_deadline_and_next_call_gets_its_own_budget() {
    let _serial = SERIAL.lock().unwrap();
    reset(3, usize::MAX);
    let client = support::Client::new::<Fixture>();
    let (mount, node) = client.root();
    let before = Instant::now();
    let (cursor, _, _, entries) = page(client.call(Request::ListBegin {
        mount,
        directory: node.id,
        max_entries: 1,
    }));
    let after = Instant::now();
    assert_eq!(entries.len(), 1);
    let begin = BEGIN_DEADLINE.lock().unwrap().unwrap();
    assert!(begin >= before + Duration::from_secs(10));
    assert!(begin <= after + Duration::from_secs(10));
    assert_eq!(*NEXT_DEADLINES.lock().unwrap(), vec![begin]);
    let before_next = Instant::now();
    page(client.call(Request::ListNext {
        cursor,
        expected_page: 1,
        max_entries: 1,
    }));
    assert!(NEXT_DEADLINES.lock().unwrap()[1] >= before_next + Duration::from_secs(10));
}

#[test]
fn streams_first_page_and_requires_exact_ordinals_then_reuses_completed_cache() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    reset(257, usize::MAX);
    let client = support::Client::new::<Fixture>();
    let (mount, root) = client.root();
    let (cursor, ordinal, done, first) = page(client.call(Request::ListBegin {
        mount,
        directory: root.id,
        max_entries: 128,
    }));
    assert_eq!(ordinal, 0);
    assert!(!done);
    assert_eq!(first.len(), 128);
    assert_eq!(NEXTS.load(Ordering::SeqCst), 128);
    assert_ne!(first[0].id, first[1].id);
    assert_eq!(first[0].display, first[1].display);
    failed(
        client.call(Request::ListNext {
            cursor,
            expected_page: 2,
            max_entries: 128,
        }),
        ErrorCode::StaleReference,
    );
    let (_, ordinal, done, second) = page(client.call(Request::ListNext {
        cursor,
        expected_page: 1,
        max_entries: 128,
    }));
    assert_eq!(ordinal, 1);
    assert!(!done);
    assert_eq!(second.len(), 128);
    let (_, ordinal, done, last) = page(client.call(Request::ListNext {
        cursor,
        expected_page: 2,
        max_entries: 128,
    }));
    assert_eq!(ordinal, 2);
    assert!(done);
    assert_eq!(last.len(), 1);
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(
        client.cancelled(Request::CloseList { cursor }, true),
        Response::Unit
    );
    let (cached, _, _, again) = page(client.call(Request::ListBegin {
        mount,
        directory: root.id,
        max_entries: 3,
    }));
    assert_eq!(
        again.iter().map(|node| node.id).collect::<Vec<_>>(),
        first[..3].iter().map(|node| node.id).collect::<Vec<_>>()
    );
    assert_eq!(BEGINS.load(Ordering::SeqCst), 1);
    assert_eq!(
        client.call(Request::CloseList { cursor: cached }),
        Response::Unit
    );
}

#[test]
fn later_failure_preserves_published_nodes_and_never_populates_cache() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    reset(200, 128);
    let client = support::Client::new::<Fixture>();
    let (mount, root) = client.root();
    let (cursor, _, _, first) = page(client.call(Request::ListBegin {
        mount,
        directory: root.id,
        max_entries: 128,
    }));
    failed(
        client.call(Request::ListNext {
            cursor,
            expected_page: 1,
            max_entries: 128,
        }),
        ErrorCode::Offline,
    );
    assert!(matches!(
        client.call(Request::Stat {
            mount,
            node: first[0].id
        }),
        Response::Node(_)
    ));
    failed(
        client.call(Request::CloseList { cursor }),
        ErrorCode::StaleReference,
    );
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);
    let (next, _, _, _) = page(client.call(Request::ListBegin {
        mount,
        directory: root.id,
        max_entries: 1,
    }));
    assert_eq!(BEGINS.load(Ordering::SeqCst), 2);
    client.call(Request::CloseList { cursor: next });
}

#[test]
fn refresh_invalidates_complete_cache_and_retires_live_cursor() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    reset(2, usize::MAX);
    let client = support::Client::new::<Fixture>();
    let (mount, root) = client.root();
    let (cursor, _, done, _) = page(client.call(Request::ListBegin {
        mount,
        directory: root.id,
        max_entries: 128,
    }));
    assert!(done);
    client.call(Request::CloseList { cursor });
    assert_eq!(
        client.call(Request::Refresh {
            mount,
            node: root.id
        }),
        Response::Unit
    );
    let (active, _, done, _) = page(client.call(Request::ListBegin {
        mount,
        directory: root.id,
        max_entries: 1,
    }));
    assert!(!done);
    assert_eq!(BEGINS.load(Ordering::SeqCst), 2);
    assert_eq!(
        client.call(Request::Refresh {
            mount,
            node: root.id
        }),
        Response::Unit
    );
    failed(
        client.call(Request::ListNext {
            cursor: active,
            expected_page: 1,
            max_entries: 1,
        }),
        ErrorCode::StaleReference,
    );
    assert_eq!(DROPS.load(Ordering::SeqCst), 2);
}

#[test]
fn foreign_ids_and_cancelled_cleanup_are_enforced() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    reset(4, usize::MAX);
    let one = support::Client::new::<Fixture>();
    let two = support::Client::new::<Fixture>();
    let (mount, root) = one.root();
    let (other_mount, other) = two.root();
    failed(
        one.call(Request::ListBegin {
            mount,
            directory: other.id,
            max_entries: 1,
        }),
        ErrorCode::StaleReference,
    );
    let (cursor, _, _, _) = page(one.call(Request::ListBegin {
        mount,
        directory: root.id,
        max_entries: 1,
    }));
    failed(
        two.call(Request::ListNext {
            cursor,
            expected_page: 1,
            max_entries: 1,
        }),
        ErrorCode::StaleReference,
    );
    failed(
        one.call(Request::CloseMount { mount: other_mount }),
        ErrorCode::StaleReference,
    );
    assert_eq!(
        one.cancelled(Request::CloseMount { mount }, true),
        Response::Unit
    );
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);
    failed(
        one.call(Request::ListNext {
            cursor,
            expected_page: 1,
            max_entries: 1,
        }),
        ErrorCode::StaleReference,
    );
}

#[test]
fn replaying_a_complete_cache_does_not_replace_it_with_an_empty_listing() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    reset(2, usize::MAX);
    let client = support::Client::new::<Fixture>();
    let (mount, root) = client.root();
    for _ in 0..3 {
        let (cursor, _, done, entries) = page(client.call(Request::ListBegin {
            mount,
            directory: root.id,
            max_entries: 128,
        }));
        assert!(done);
        assert_eq!(entries.len(), 2);
        client.call(Request::CloseList { cursor });
    }
    assert_eq!(BEGINS.load(Ordering::SeqCst), 1);
}

#[test]
fn begin_unavailable_failure_is_preserved_and_mount_is_still_usable() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    reset(usize::MAX, usize::MAX);
    let client = support::Client::new::<Fixture>();
    let (mount, root) = client.root();
    failed(
        client.call(Request::ListBegin {
            mount,
            directory: root.id,
            max_entries: 1,
        }),
        ErrorCode::Unavailable,
    );
    assert!(matches!(
        client.call(Request::Stat {
            mount,
            node: root.id
        }),
        Response::Node(_)
    ));
}

#[test]
fn rejected_publication_rolls_back_mount_and_node_admission() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    reset(1, usize::MAX);
    let client = support::Client::configured::<Fixture>(abi::Config {
        reply_bytes: 256,
        ..abi::Config::default()
    });
    for _ in 0..80 {
        assert_eq!(
            client.invoke(
                Request::ResolveText {
                    text: "x".repeat(300)
                },
                &|| false
            ),
            Err(abi::RESOURCE_LIMIT)
        );
    }
    let (mount, root) = client.root();
    assert!(matches!(
        client.call(Request::Stat {
            mount,
            node: root.id
        }),
        Response::Node(_)
    ));
}

#[test]
fn late_closed_metadata_reply_is_rejected_without_poisoning_the_instance() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    reset(1, usize::MAX);
    let client = support::Client::new::<Fixture>();
    for parent in [false, true] {
        let (mount, root) = client.root();
        let callbacks = AtomicUsize::new(0);
        let request = if parent {
            Request::Parent {
                mount,
                node: root.id,
            }
        } else {
            Request::Stat {
                mount,
                node: root.id,
            }
        };
        let response = client.invoke(request, &|| {
            // SDK asks once before handling and once immediately before publication.
            if callbacks.fetch_add(1, Ordering::SeqCst) == 1 {
                assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
            }
            false
        });
        assert_eq!(response, Err(abi::UNKNOWN_HANDLE));
    }
    let (mount, root) = client.root();
    assert!(matches!(
        client.call(Request::Stat {
            mount,
            node: root.id
        }),
        Response::Node(_)
    ));
}

#[test]
fn sixty_four_mount_budget_is_reclaimed_after_close() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    reset(1, usize::MAX);
    let client = support::Client::new::<Fixture>();
    let mut mounts = Vec::new();
    for _ in 0..64 {
        mounts.push(client.root().0);
    }
    failed(
        client.call(Request::ResolveText {
            text: "another".into(),
        }),
        ErrorCode::ResourceLimit,
    );
    client.call(Request::CloseMount {
        mount: mounts.pop().unwrap(),
    });
    assert!(matches!(
        client.call(Request::ResolveText {
            text: "another".into()
        }),
        Response::Resolved { .. }
    ));
}

#[test]
fn node_limit_failure_preserves_prior_pages_and_releases_with_mount() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    reset(5000, usize::MAX);
    let client = support::Client::new::<Fixture>();
    let (mount, root) = client.root();
    let (cursor, _, _, first) = page(client.call(Request::ListBegin {
        mount,
        directory: root.id,
        max_entries: 128,
    }));
    let mut ordinal = 1;
    loop {
        match client.call(Request::ListNext {
            cursor,
            expected_page: ordinal,
            max_entries: 128,
        }) {
            Response::Page { done: false, .. } => ordinal += 1,
            Response::Failed(Failure {
                code: ErrorCode::ResourceLimit,
                ..
            }) => break,
            other => panic!("{other:?}"),
        }
        assert!(ordinal < 40);
    }
    assert!(matches!(
        client.call(Request::Stat {
            mount,
            node: first[0].id
        }),
        Response::Node(_)
    ));
    let other = support::Client::new::<Fixture>();
    let (other_mount, other_root) = other.root();
    failed(
        other.call(Request::ListBegin {
            mount: other_mount,
            directory: other_root.id,
            max_entries: 128,
        }),
        ErrorCode::ResourceLimit,
    );
    client.call(Request::CloseMount { mount });
    let (mount, root) = client.root();
    let (cursor, _, _, _) = page(client.call(Request::ListBegin {
        mount,
        directory: root.id,
        max_entries: 1,
    }));
    client.call(Request::CloseList { cursor });
}

#[test]
fn per_plugin_mount_budget_is_shared_across_instances() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    reset(1, usize::MAX);
    let one = support::Client::new::<Fixture>();
    let two = support::Client::new::<Fixture>();
    let mut first = Vec::new();
    let mut second = Vec::new();
    for _ in 0..32 {
        first.push(one.root().0);
        second.push(two.root().0);
    }
    failed(
        two.call(Request::ResolveText {
            text: "overflow".into(),
        }),
        ErrorCode::ResourceLimit,
    );
    one.call(Request::CloseMount {
        mount: first.pop().unwrap(),
    });
    assert!(matches!(
        two.call(Request::ResolveText {
            text: "available".into()
        }),
        Response::Resolved { .. }
    ));
}

#[test]
fn per_plugin_retained_byte_budget_is_shared_and_reclaimed() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    reset(1, usize::MAX);
    let one = support::Client::new::<Fixture>();
    let two = support::Client::new::<Fixture>();
    let text = "x".repeat(100_000);
    let mut mounts = Vec::new();
    loop {
        match one.call(Request::ResolveText { text: text.clone() }) {
            Response::Resolved { mount, .. } => mounts.push(mount),
            Response::Failed(Failure {
                code: ErrorCode::ResourceLimit,
                ..
            }) => break,
            other => panic!("{other:?}"),
        }
        assert!(mounts.len() < 64);
    }
    assert!(!mounts.is_empty());
    failed(
        two.call(Request::ResolveText { text: text.clone() }),
        ErrorCode::ResourceLimit,
    );
    one.call(Request::CloseMount {
        mount: mounts.pop().unwrap(),
    });
    assert!(matches!(
        two.call(Request::ResolveText { text }),
        Response::Resolved { .. }
    ));
}

#[test]
fn cancelled_complete_page_publication_rolls_back_and_never_seeds_cache() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    reset(2, usize::MAX);
    let client = support::Client::new::<Fixture>();
    let (mount, root) = client.root();
    let after_eof = AtomicUsize::new(0);
    let response = client
        .invoke(
            Request::ListBegin {
                mount,
                directory: root.id,
                max_entries: 128,
            },
            &|| NEXTS.load(Ordering::SeqCst) >= 3 && after_eof.fetch_add(1, Ordering::SeqCst) >= 1,
        )
        .unwrap();
    failed(response, ErrorCode::Cancelled);
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);
    let (cursor, _, done, entries) = page(client.call(Request::ListBegin {
        mount,
        directory: root.id,
        max_entries: 128,
    }));
    assert!(done);
    assert_eq!(entries.len(), 2);
    assert_eq!(BEGINS.load(Ordering::SeqCst), 2);
    client.call(Request::CloseList { cursor });
}

#[test]
fn reply_byte_boundary_keeps_pending_records_in_live_and_cached_cursors() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    reset(2, usize::MAX);
    let client = support::Client::configured::<Fixture>(abi::Config {
        reply_bytes: 512,
        ..Default::default()
    });
    let (mount, root) = client.root();
    for _ in 0..3 {
        let (cursor, ordinal, done, first) = page(client.call(Request::ListBegin {
            mount,
            directory: root.id,
            max_entries: 128,
        }));
        assert_eq!((ordinal, done, first.len()), (0, false, 1));
        let (_, ordinal, done, second) = page(client.call(Request::ListNext {
            cursor,
            expected_page: 1,
            max_entries: 128,
        }));
        assert_eq!((ordinal, done, second.len()), (1, true, 1));
        assert_ne!(first[0].id, second[0].id);
        assert_eq!(client.call(Request::CloseList { cursor }), Response::Unit);
    }
    assert_eq!(BEGINS.load(Ordering::SeqCst), 1);
}
