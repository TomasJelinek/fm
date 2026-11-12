//! Copy capacity and mutation serialization follow actual provider destruction.
use super::*;
use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

struct Native {
    entered: mpsc::Sender<thread::ThreadId>,
    release: mpsc::Receiver<()>,
}
impl Drop for Native {
    fn drop(&mut self) {
        let _ = self.entered.send(thread::current().id());
        let _ = self.release.recv();
    }
}
struct Provider(mpsc::Sender<thread::ThreadId>);
impl Drop for Provider {
    fn drop(&mut self) {
        let _ = self.0.send(thread::current().id());
    }
}
struct Payload {
    _native: Native,
    _provider: Provider,
}
struct Release(Option<mpsc::Sender<()>>);
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(send) = self.0.take() {
            let _ = send.send(());
        }
    }
}

#[test]
fn copy_scope_keeps_capacity_and_mutation_permit_until_native_and_provider_drop_without_runtime() {
    let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
    let lease = domain.try_reserve().unwrap();
    let capacity = Arc::new(Semaphore::new(1));
    let mutation = Arc::new(Semaphore::new(1));
    let permit = capacity.clone().try_acquire_owned().unwrap();
    let mutation_permit = mutation.clone().try_acquire_owned().unwrap();
    let fence = crate::copy_fence::CopyFence::default();
    let (entered, native) = mpsc::channel();
    let (send, receive) = mpsc::channel();
    let release = Release(Some(send));
    let (provider, retired) = mpsc::channel();
    let owned = lease.own(Scope {
        payload: Payload {
            _native: Native {
                entered,
                release: receive,
            },
            _provider: Provider(provider),
        },
        resources: Resources {
            receipt: fence.register(),
            mutation: Some(mutation_permit),
            _capacity: permit,
        },
    });
    drop(owned);
    let worker = native.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_ne!(worker, thread::current().id());
    assert!(
        retired.try_recv().is_err(),
        "native pin closes before provider"
    );
    assert_eq!(capacity.available_permits(), 0);
    assert_eq!(mutation.available_permits(), 0);
    assert!(
        domain.try_reserve().is_err(),
        "blocked cleanup retains its slot"
    );
    drop(release);
    assert_eq!(
        retired.recv_timeout(Duration::from_secs(5)).unwrap(),
        worker
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(lease) = domain.try_reserve() {
            assert_eq!(capacity.available_permits(), 1);
            assert_eq!(mutation.available_permits(), 1);
            drop(lease);
            break;
        }
        assert!(Instant::now() < deadline);
        thread::park_timeout(Duration::from_millis(1));
    }
}
