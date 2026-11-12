use super::*;
use std::{
    sync::mpsc as sync_mpsc,
    thread::{self, ThreadId},
};

pub(super) type Gate = Arc<(Mutex<bool>, Condvar)>;
pub(super) fn gate() -> Gate {
    Arc::new((Mutex::new(false), Condvar::new()))
}
pub(super) fn release(gate: &Gate) {
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
}
pub(super) struct Release(pub(super) Gate);
impl Drop for Release {
    fn drop(&mut self) {
        release(&self.0);
    }
}
#[derive(Debug)]
pub(super) struct Probe {
    caller: ThreadId,
    entered: sync_mpsc::Sender<ThreadId>,
    gate: Gate,
}
impl Drop for Probe {
    fn drop(&mut self) {
        let current = thread::current().id();
        let _ = self.entered.send(current);
        // Broken caller destruction must fail deterministically rather than hang.
        if current != self.caller {
            let _released = self
                .gate
                .1
                .wait_while(self.gate.0.lock().unwrap(), |released| !*released)
                .unwrap();
        }
    }
}
#[derive(Debug)]
struct NativeProvider {
    _drop: Probe,
    inner: Fake,
}
impl NetworkNeighborhood for NativeProvider {
    fn root(&self, c: &Cancellation) -> Result<NetworkAddress, NetworkFailure> {
        self.inner.root(c)
    }
    fn parse(&self, text: &str, c: &Cancellation) -> Result<NetworkAddress, NetworkFailure> {
        self.inner.parse(text, c)
    }
    fn parent(
        &self,
        a: &NetworkAddress,
        c: &Cancellation,
    ) -> Result<Option<NetworkAddress>, NetworkFailure> {
        self.inner.parent(a, c)
    }
    fn list(
        &self,
        a: &NetworkAddress,
        c: &Cancellation,
        emit: &mut dyn FnMut(NetworkNode) -> bool,
    ) -> Result<(), NetworkFailure> {
        self.inner.list(a, c, emit)
    }
    fn activate(
        &self,
        a: &NetworkAddress,
        c: &Cancellation,
    ) -> Result<NetworkDestination, NetworkFailure> {
        self.inner.activate(a, c)
    }
    fn refresh(&self, a: &NetworkAddress, c: &Cancellation) -> Result<(), NetworkFailure> {
        self.inner.refresh(a, c)
    }
}
pub(super) struct Observer {
    pub(super) dropped: sync_mpsc::Receiver<ThreadId>,
    pub(super) _release: Release,
}
pub(super) fn factory(
    inner: Fake,
) -> (
    impl FnOnce() -> Result<Arc<dyn NetworkNeighborhood>, NetworkFailure> + Send + 'static,
    Observer,
) {
    let (entered, dropped) = sync_mpsc::channel();
    let gate = gate();
    let caller = thread::current().id();
    let drop_gate = gate.clone();
    (
        move || {
            Ok(Arc::new(NativeProvider {
                _drop: Probe {
                    caller,
                    entered,
                    gate: drop_gate,
                },
                inner,
            }))
        },
        Observer {
            dropped,
            _release: Release(gate),
        },
    )
}
pub(super) fn wait_finished<T>(work: &tokio::task::JoinHandle<T>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !work.is_finished() {
        assert!(Instant::now() < deadline, "native creation did not finish");
        thread::sleep(Duration::from_millis(1));
    }
}
pub(super) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
pub(super) fn assert_worker_drop(observer: &Observer) {
    assert_ne!(
        observer
            .dropped
            .recv_timeout(Duration::from_secs(5))
            .unwrap(),
        thread::current().id(),
        "final discovered native provider must not be destroyed on caller",
    );
}
