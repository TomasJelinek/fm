use super::*;
use std::sync::{atomic::AtomicUsize, mpsc};

/// Stands in for a pinned native directory handle; reports the thread that closes it.
struct Probe(mpsc::Sender<std::thread::ThreadId>);
impl Drop for Probe {
    fn drop(&mut self) {
        let _ = self.0.send(std::thread::current().id());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn idle_parent_pins_are_released_on_a_backend_worker_without_another_capture() {
    let (closed, released) = mpsc::channel();
    let prepared = Arc::new(AtomicUsize::new(0));
    let pins =
        DirectoryPins::<Arc<Probe>>::with_idle(tokio::runtime::Handle::current(), PIN_IDLE / 300);
    let parent = Path::new("removable/文档");
    let prepare = || {
        prepared.fetch_add(1, Ordering::Relaxed);
        Some(Arc::new(Probe(closed.clone())))
    };
    drop(pins.pin(parent, |_| true, prepare));
    // A sibling viewed right away reuses the pin.
    drop(pins.pin(parent, |_| true, prepare));
    assert_eq!(prepared.load(Ordering::Relaxed), 1);
    drop(closed);
    let caller = std::thread::current().id();
    // With no further capture, the idle handle is released on a blocking worker.
    let releaser =
        tokio::task::spawn_blocking(move || released.recv_timeout(Duration::from_secs(10)))
            .await
            .unwrap()
            .expect("the idle pin was not released");
    assert_ne!(releaser, caller);
    assert!(pins.lock().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn pins_are_bounded_and_invalid_pins_are_replaced() {
    let (closed, released) = mpsc::channel();
    let pins = DirectoryPins::<Arc<Probe>>::new(tokio::runtime::Handle::current());
    let probe = || Some(Arc::new(Probe(closed.clone())));
    for index in 0..=MAX_PINS {
        drop(pins.pin(&PathBuf::from(format!("parent-{index}")), |_| true, probe));
    }
    assert_eq!(pins.lock().len(), MAX_PINS);
    assert_eq!(released.try_iter().count(), 1, "the oldest pin was evicted");
    assert!(
        pins.pin(Path::new("parent-0"), |_| true, || None).is_none(),
        "evicted parents are prepared again"
    );
    // A pin that no longer revalidates is replaced, and the stale one released.
    assert!(pins.pin(Path::new("parent-1"), |_| false, probe).is_some());
    assert_eq!(released.try_iter().count(), 1);
    assert_eq!(pins.lock().len(), MAX_PINS);
}
