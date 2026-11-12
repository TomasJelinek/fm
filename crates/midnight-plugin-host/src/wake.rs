//! Coalesced signals; callbacks only enqueue work or unpark an owning thread.
use std::sync::{Arc, Mutex};

type Callback = Arc<dyn Fn() -> bool + Send + Sync>;

#[derive(Default)]
pub(crate) struct Wake {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    callback: Option<Callback>,
    pending: bool,
}

impl Wake {
    pub fn bind(&self, callback: Callback) {
        let pending = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.callback = Some(callback.clone());
            state.pending
        };
        if pending {
            self.fire(callback);
        }
    }
    pub fn notify(&self) {
        let callback = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.pending {
                return;
            }
            state.pending = true;
            state.callback.clone()
        };
        if let Some(callback) = callback {
            self.fire(callback);
        }
    }
    fn fire(&self, callback: Callback) {
        if !callback() {
            self.reset();
        }
    }
    pub fn reset(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).pending = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        time::Duration,
    };

    #[test]
    fn notification_before_binding_and_bursts_preserve_one_pending_signal() {
        let wake = Wake::default();
        wake.notify();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        wake.bind(Arc::new(move || {
            counted.fetch_add(1, Ordering::Relaxed);
            true
        }));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        for _ in 0..1000 {
            wake.notify();
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        wake.reset();
        wake.notify();
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn publication_wakes_a_parked_owner_without_waiting_for_poll_timeout() {
        let wake = Arc::new(Wake::default());
        let owner = wake.clone();
        let (ready, started) = mpsc::sync_channel(1);
        let (done, finished) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let thread = std::thread::current();
            owner.bind(Arc::new(move || {
                thread.unpark();
                true
            }));
            ready.send(()).unwrap();
            std::thread::park_timeout(Duration::from_secs(2));
            done.send(()).unwrap();
        });
        started.recv().unwrap();
        wake.notify();
        let promptly = finished.recv_timeout(Duration::from_millis(200)).is_ok();
        worker.join().unwrap();
        assert!(
            promptly,
            "ready publication must wake the owner before fallback polling"
        );
    }
}
