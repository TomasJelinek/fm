//! Real Content lease acknowledgements, with a controlled native transport.
use super::*;
use crate::{Request, Response, Transport};
use midnight_filesystem_plugin_api::ErrorCode;
use std::sync::{
    Mutex,
    atomic::{AtomicU8, Ordering},
};

#[derive(Debug, Default)]
struct Calls {
    requests: Mutex<Vec<Request>>,
    failure: AtomicU8,
    bomb_drops: Arc<std::sync::atomic::AtomicUsize>,
}
#[derive(Debug)]
struct Bomb(Arc<std::sync::atomic::AtomicUsize>);
impl Drop for Bomb {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("opaque panic must not be dropped");
    }
}
impl Transport for Calls {
    fn call(&self, request: &Request, _: Cancel<'_>) -> Result<Response, Failure> {
        self.requests.lock().unwrap().push(request.clone());
        if matches!(request, Request::ValidateRead { .. })
            && self.failure.load(Ordering::SeqCst) == 4
        {
            std::panic::panic_any(Bomb(self.bomb_drops.clone()));
        }
        if matches!(request, Request::CloseRead { .. }) && self.failure.load(Ordering::SeqCst) == 3
        {
            std::panic::panic_any(Bomb(self.bomb_drops.clone()));
        }
        if (matches!(request, Request::CloseRead { .. })
            && self.failure.load(Ordering::SeqCst) == 1)
            || (matches!(request, Request::CloseMount { .. })
                && self.failure.load(Ordering::SeqCst) == 2)
        {
            return Ok(Response::Failed(Failure::new(
                ErrorCode::Io,
                "close refused",
            )));
        }
        Ok(Response::Unit)
    }
}
fn fixture() -> (Arc<Content>, Arc<Calls>) {
    let calls = Arc::new(Calls::default());
    let provider = NativeProvider {
        transport: calls.clone(),
        capabilities: 0,
    };
    (
        Arc::new(Content {
            reader_lease: Lease::new(&provider, Request::CloseRead { reader: 41 }),
            _mount_lease: Lease::new(&provider, Request::CloseMount { mount: 9 }),
            reader: 41,
            length: 1,
            revision: vec![1],
            name: "source".into(),
            release_unconfirmed: AtomicBool::new(false),
        }),
        calls,
    )
}
#[test]
fn last_content_release_confirms_reader_before_mount_once() {
    let (content, calls) = fixture();
    content.release_on_worker().unwrap();
    assert!(matches!(
        calls.requests.lock().unwrap().as_slice(),
        [
            Request::CloseRead { reader: 41 },
            Request::CloseMount { mount: 9 }
        ]
    ));
}
#[test]
fn shared_content_release_leaves_the_other_owner_usable() {
    let (content, calls) = fixture();
    content.clone().release_on_worker().unwrap();
    assert!(calls.requests.lock().unwrap().is_empty());
    content.validate().unwrap();
    calls.requests.lock().unwrap().clear();
    content.release_on_worker().unwrap();
    assert_eq!(calls.requests.lock().unwrap().len(), 2);
}
#[test]
fn reader_close_refusal_retains_mount_without_fallback_retry() {
    let (content, calls) = fixture();
    calls.failure.store(1, Ordering::SeqCst);
    assert!(content.release_on_worker().is_err());
    assert!(matches!(
        calls.requests.lock().unwrap().as_slice(),
        [Request::CloseRead { reader: 41 }]
    ));
    assert!(
        Arc::strong_count(&calls) > 1,
        "unconfirmed leases retain their provider"
    );
}
#[test]
fn mount_close_refusal_preserves_tail_after_confirmed_reader_close() {
    let (content, calls) = fixture();
    calls.failure.store(2, Ordering::SeqCst);
    assert!(content.release_on_worker().is_err());
    assert!(matches!(
        calls.requests.lock().unwrap().as_slice(),
        [
            Request::CloseRead { reader: 41 },
            Request::CloseMount { mount: 9 }
        ]
    ));
    assert!(Arc::strong_count(&calls) > 1);
}
#[test]
fn ordinary_content_drop_keeps_existing_best_effort_cleanup() {
    let (content, calls) = fixture();
    drop(content);
    assert!(matches!(
        calls.requests.lock().unwrap().as_slice(),
        [
            Request::CloseRead { reader: 41 },
            Request::CloseMount { mount: 9 }
        ]
    ));
}

#[test]
fn panicking_reader_close_retains_mount_and_does_not_retry_from_drop() {
    let (content, calls) = fixture();
    calls.failure.store(3, Ordering::SeqCst);
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| content.release_on_worker()));
    let panic = result.expect_err("close must unwind to the enclosing Copy seal");
    std::mem::forget(panic);
    assert_eq!(calls.bomb_drops.load(Ordering::SeqCst), 0);
    assert!(matches!(
        calls.requests.lock().unwrap().as_slice(),
        [Request::CloseRead { reader: 41 }]
    ));
    assert!(Arc::strong_count(&calls) > 1);
}

#[test]
fn native_callback_panic_is_sticky_until_explicit_release() {
    let (content, calls) = fixture();
    calls.failure.store(4, Ordering::SeqCst);
    assert!(content.validate().is_err());
    assert!(content.release_on_worker().is_err());
    assert_eq!(calls.bomb_drops.load(Ordering::SeqCst), 0);
    assert!(matches!(
        calls.requests.lock().unwrap().as_slice(),
        [Request::ValidateRead { reader: 41 }]
    ));
}

#[test]
fn shared_tainted_content_cannot_report_success_without_closing() {
    let (content, calls) = fixture();
    let other = content.clone();
    calls.failure.store(4, Ordering::SeqCst);
    assert!(content.validate().is_err());
    let requests = calls.requests.lock().unwrap().len();
    assert!(content.cleanup_unconfirmed());
    assert!(other.cleanup_unconfirmed());
    assert_eq!(calls.requests.lock().unwrap().len(), requests);
    assert!(
        content.release_on_worker().is_err(),
        "prior taint must precede shared success"
    );
    assert!(other.release_on_worker().is_err());
    assert_eq!(calls.requests.lock().unwrap().len(), requests);
    assert_eq!(calls.bomb_drops.load(Ordering::SeqCst), 0);
}
