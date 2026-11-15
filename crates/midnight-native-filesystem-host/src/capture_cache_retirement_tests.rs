//! Snapshot retirement covers only its own cache entry, including eviction tails.
use super::*;
use crate::{Request, Response, Transport, test_path};
use midnight_filesystem_plugin_api::{ErrorCode, Kind, Metadata, Node, abi};
use std::sync::{Weak, atomic::AtomicBool};

#[derive(Debug, Default)]
struct State {
    requests: Vec<Request>,
    next_mount: u64,
    next_reader: u64,
}
#[derive(Debug, Default)]
struct Calls {
    state: Mutex<State>,
    fail_close: AtomicBool,
    fail_mount: AtomicBool,
    refuse_open: AtomicBool,
    refuse_read: AtomicBool,
    malformed_read: AtomicBool,
    invalid_open_revision: AtomicBool,
    read_barrier: Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
    cache: Mutex<Weak<CaptureCache>>,
}
impl Transport for Calls {
    fn call(&self, request: &Request, _: Cancel<'_>) -> Result<Response, Failure> {
        let barrier = if matches!(request, Request::ReadAt { .. }) {
            lock(&self.read_barrier).take()
        } else {
            None
        };
        if let Some((entered, release)) = barrier {
            entered.send(()).unwrap();
            release
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        if matches!(
            request,
            Request::CloseRead { .. } | Request::CloseMount { .. }
        ) && let Some(cache) = lock(&self.cache).upgrade()
        {
            assert!(
                cache.readers.try_lock().is_ok(),
                "native close ran under readers mutex"
            );
        }
        let mut state = lock(&self.state);
        state.requests.push(request.clone());
        Ok(match request {
            Request::Resolve { address } => {
                state.next_mount += 1;
                Response::Resolved {
                    mount: state.next_mount,
                    node: Node {
                        id: 2,
                        address: address.clone(),
                        name_encoding: test_path::encoding(),
                        raw_name: test_path::name("source.bin"),
                        display: "source.bin".into(),
                        kind: Kind::File,
                        metadata: Metadata {
                            size: Some(1),
                            ..Metadata::default()
                        },
                    },
                }
            }
            Request::OpenRead { .. } if self.refuse_open.load(Ordering::SeqCst) => {
                Response::Failed(Failure::new(
                    ErrorCode::SourceChanged,
                    "source changed before open",
                ))
            }
            Request::OpenRead { .. } => {
                state.next_reader += 1;
                Response::Opened {
                    reader: state.next_reader,
                    length: 1,
                    revision: if self.invalid_open_revision.load(Ordering::SeqCst) {
                        Vec::new()
                    } else {
                        vec![1]
                    },
                }
            }
            Request::ValidateRead { .. } => Response::Unit,
            Request::ReadAt { .. } if self.malformed_read.load(Ordering::SeqCst) => {
                Response::Opened {
                    reader: 900,
                    length: 1,
                    revision: vec![1],
                }
            }
            Request::ReadAt { .. } if self.refuse_read.load(Ordering::SeqCst) => Response::Failed(
                Failure::new(ErrorCode::SourceChanged, "source changed during read"),
            ),
            Request::ReadAt { .. } => Response::Content {
                bytes: vec![7],
                eof: true,
            },
            Request::CloseRead { .. } if self.fail_close.load(Ordering::SeqCst) => {
                Response::Failed(Failure::new(ErrorCode::Io, "reader close unconfirmed"))
            }
            Request::CloseMount { .. } if self.fail_mount.load(Ordering::SeqCst) => {
                Response::Failed(Failure::new(ErrorCode::Io, "mount close unconfirmed"))
            }
            Request::CloseRead { .. } | Request::CloseMount { .. } => Response::Unit,
            _ => panic!("unexpected content request: {request:?}"),
        })
    }
}
fn fixture() -> (Arc<CaptureCache>, Arc<Calls>, PathBuf) {
    let calls = Arc::new(Calls::default());
    let cache = CaptureCache::new(NativeProvider {
        transport: calls.clone(),
        capabilities: abi::CAP_NATIVE_ADDRESS | abi::CAP_READ,
    });
    *lock(&calls.cache) = Arc::downgrade(&cache);
    (
        cache,
        calls,
        std::env::temp_dir().join("capture-retirement/source.bin"),
    )
}
#[test]
fn last_snapshot_removes_only_its_reader_and_closes_outside_cache_mutex() {
    let (cache, calls, path) = fixture();
    let first = cache.capture(&path, &|| false).unwrap();
    let second = cache.capture(&path, &|| false).unwrap();
    first.read_at(0, 1).unwrap();
    second.read_at(0, 1).unwrap();
    lock(&calls.state).requests.clear();
    first.release_on_worker().unwrap();
    assert_eq!(lock(&cache.readers).len(), 1);
    assert!(matches!(
        lock(&calls.state).requests.as_slice(),
        [
            Request::CloseRead { reader: 3 },
            Request::CloseMount { mount: 3 }
        ]
    ));
    assert_eq!(second.read_at(0, 1).unwrap(), [7]);
    second.release_on_worker().unwrap();
    assert!(lock(&cache.readers).is_empty());
}
#[test]
fn shared_snapshot_release_does_not_close_another_tab() {
    let (cache, calls, path) = fixture();
    let first = cache.capture(&path, &|| false).unwrap();
    let other_tab = first.clone();
    first.read_at(0, 1).unwrap();
    lock(&calls.state).requests.clear();
    first.release_on_worker().unwrap();
    assert!(lock(&calls.state).requests.is_empty());
    assert_eq!(other_tab.read_at(0, 1).unwrap(), [7]);
    other_tab.release_on_worker().unwrap();
    assert!(lock(&cache.readers).is_empty());
}
#[test]
fn refused_cached_reader_close_is_not_retried_by_fallback_drop() {
    let (cache, calls, path) = fixture();
    let file = cache.capture(&path, &|| false).unwrap();
    file.read_at(0, 1).unwrap();
    lock(&calls.state).requests.clear();
    calls.fail_close.store(true, Ordering::SeqCst);
    assert!(file.release_on_worker().is_err());
    assert!(matches!(
        lock(&calls.state).requests.as_slice(),
        [Request::CloseRead { reader: 2 }]
    ));
    calls.fail_close.store(false, Ordering::SeqCst);
}
#[test]
fn eviction_close_failure_remains_unconfirmed_at_the_triggering_snapshots_release() {
    let (cache, calls, path) = fixture();
    let mut files: Vec<_> = (0..=READERS)
        .map(|_| cache.capture(&path, &|| false).unwrap())
        .collect();
    for file in files.iter().take(READERS) {
        file.read_at(0, 1).unwrap();
    }
    let last = files.pop().unwrap();
    calls.fail_close.store(true, Ordering::SeqCst);
    assert!(
        last.read_at(0, 1).is_err(),
        "eviction failure cannot be hidden by a successful read"
    );
    assert!(
        last.release_on_worker().is_err(),
        "final release must preserve the unconfirmed eviction tail"
    );
    calls.fail_close.store(false, Ordering::SeqCst);
}

#[test]
fn initial_reader_close_failure_prevents_a_successful_capture() {
    let (cache, calls, path) = fixture();
    calls.fail_close.store(true, Ordering::SeqCst);
    assert!(cache.capture(&path, &|| false).is_err());
    let state = lock(&calls.state);
    assert_eq!(
        state
            .requests
            .iter()
            .filter(|request| matches!(request, Request::CloseRead { .. }))
            .count(),
        1
    );
    assert!(
        !state
            .requests
            .iter()
            .any(|request| matches!(request, Request::CloseMount { .. }))
    );
    assert!(Arc::strong_count(&calls) > 1);
}

#[test]
fn ordinary_read_or_open_failure_still_allows_confirmed_teardown() {
    for open_failure in [false, true] {
        let (cache, calls, path) = fixture();
        let file = cache.capture(&path, &|| false).unwrap();
        assert!(!file.cleanup_unconfirmed());
        calls.refuse_open.store(open_failure, Ordering::SeqCst);
        calls.refuse_read.store(!open_failure, Ordering::SeqCst);
        assert!(file.read_at(0, 1).is_err());
        assert!(
            !file.cleanup_unconfirmed(),
            "ordinary SourceChanged must not become cleanup taint"
        );
        file.release_on_worker().unwrap();
        assert!(lock(&cache.readers).is_empty());
    }
}

#[test]
fn shared_tainted_snapshot_retains_the_consumed_reference() {
    let (cache, calls, path) = fixture();
    let first = cache.capture(&path, &|| false).unwrap();
    let other = first.clone();
    calls.refuse_open.store(true, Ordering::SeqCst);
    calls.fail_mount.store(true, Ordering::SeqCst);
    assert!(first.read_at(0, 1).is_err());
    let requests = lock(&calls.state).requests.len();
    assert!(first.cleanup_unconfirmed());
    assert!(other.cleanup_unconfirmed());
    assert!(
        first.release_on_worker().is_err(),
        "shared Snapshot must inspect its prior taint"
    );
    assert!(other.release_on_worker().is_err());
    assert_eq!(lock(&calls.state).requests.len(), requests);
}

#[test]
fn captured_file_observes_cached_temporary_content_taint_while_another_tab_survives() {
    let (cache, calls, path) = fixture();
    let first = cache.capture(&path, &|| false).unwrap();
    let other_tab = first.clone();
    first.read_at(0, 1).unwrap();
    assert!(!first.cleanup_unconfirmed());
    lock(&calls.state).requests.clear();
    calls.malformed_read.store(true, Ordering::SeqCst);
    calls.fail_close.store(true, Ordering::SeqCst);
    assert!(first.read_at(0, 1).is_err());
    assert!(first.cleanup_unconfirmed());
    assert!(other_tab.cleanup_unconfirmed());
    assert!(
        matches!(
            lock(&calls.state).requests.as_slice(),
            [
                Request::ReadAt { reader: 2, .. },
                Request::CloseRead { reader: 900 },
            ]
        ),
        "failed unexpected-reader cleanup must not close the cached original"
    );
    let requests = lock(&calls.state).requests.len();
    assert!(first.release_on_worker().is_err());
    assert!(other_tab.release_on_worker().is_err());
    assert_eq!(
        lock(&calls.state).requests.len(),
        requests,
        "status/release must not probe or stop the other tab"
    );
    assert_eq!(lock(&cache.readers).len(), 1);
    calls.fail_close.store(false, Ordering::SeqCst);
}

#[test]
fn malformed_opened_reader_refusal_keeps_enclosing_mount_and_provider_owned() {
    let (cache, calls, path) = fixture();
    let file = cache.capture(&path, &|| false).unwrap();
    let provider_owners = Arc::strong_count(&calls);
    lock(&calls.state).requests.clear();
    calls.invalid_open_revision.store(true, Ordering::SeqCst);
    calls.fail_close.store(true, Ordering::SeqCst);
    assert!(file.read_at(0, 1).is_err());
    assert!(file.cleanup_unconfirmed());
    assert!(
        matches!(
            lock(&calls.state).requests.as_slice(),
            [
                Request::Resolve { .. },
                Request::OpenRead { .. },
                Request::CloseRead { reader: 2 },
            ]
        ),
        "a granted reader with invalid revision still owns its enclosing mount"
    );
    assert!(Arc::strong_count(&calls) > provider_owners);
    assert!(file.release_on_worker().is_err());
    assert!(
        !lock(&calls.state)
            .requests
            .iter()
            .any(|request| matches!(request, Request::CloseMount { .. }))
    );
    calls.fail_close.store(false, Ordering::SeqCst);
}

#[test]
fn reopening_failure_with_refused_mount_close_remains_unconfirmed() {
    let (cache, calls, path) = fixture();
    let file = cache.capture(&path, &|| false).unwrap();
    calls.refuse_open.store(true, Ordering::SeqCst);
    calls.fail_mount.store(true, Ordering::SeqCst);
    assert!(file.read_at(0, 1).is_err());
    assert!(file.release_on_worker().is_err());
    calls.fail_mount.store(false, Ordering::SeqCst);
}

#[test]
fn evicted_reader_temporary_owner_confirms_close_after_its_inflight_read_returns() {
    let (cache, calls, path) = fixture();
    let mut files: Vec<_> = (0..=READERS)
        .map(|_| cache.capture(&path, &|| false).unwrap())
        .collect();
    let first = files.remove(0);
    first.read_at(0, 1).unwrap();
    let reader = lock(&calls.state).next_reader;
    lock(&calls.state).requests.clear();
    let (entered, waiting) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    *lock(&calls.read_barrier) = Some((entered, resume));
    let active = first.clone();
    let thread = std::thread::spawn(move || {
        let result = active.read_at(0, 1);
        let released = active.release_on_worker();
        (result, released)
    });
    waiting
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    for file in &files {
        file.read_at(0, 1).unwrap();
    }
    assert!(
        !lock(&calls.state)
            .requests
            .iter()
            .any(|request| matches!(request,
        Request::CloseRead { reader: closed } if *closed == reader))
    );
    calls.fail_close.store(true, Ordering::SeqCst);
    release.send(()).unwrap();
    let (read, released) = thread.join().unwrap();
    assert!(read.is_err());
    assert!(
        released.is_err(),
        "shared release must preserve the observed cleanup taint"
    );
    assert!(
        first.release_on_worker().is_err(),
        "temporary last-owner close failure must remain sticky"
    );
    assert_eq!(
        lock(&calls.state)
            .requests
            .iter()
            .filter(|request| matches!(request,
        Request::CloseRead { reader: closed } if *closed == reader))
            .count(),
        1
    );
    calls.fail_close.store(false, Ordering::SeqCst);
}
