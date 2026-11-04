use std::sync::atomic::{AtomicBool, Ordering};

/// Coalesces backend notifications into at most one pending UI callback.
#[derive(Default)]
pub struct WakeGate(AtomicBool);

impl WakeGate {
    pub fn request(&self) -> bool {
        !self.0.swap(true, Ordering::AcqRel)
    }
    pub fn release(&self) {
        self.0.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn many_backend_events_schedule_one_callback_until_drained() {
        let gate = WakeGate::default();
        assert!(gate.request());
        for _ in 0..100 {
            assert!(
                !gate.request(),
                "event flood must not create more UI callbacks"
            );
        }
        gate.release();
        assert!(gate.request(), "a drained UI must be woken by new data");
    }

    #[test]
    fn concurrent_producers_do_not_duplicate_the_pending_callback() {
        let gate = Arc::new(WakeGate::default());
        let barrier = Arc::new(Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let gate = gate.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    gate.request()
                })
            })
            .collect();
        let scheduled = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .filter(|scheduled| *scheduled)
            .count();
        assert_eq!(scheduled, 1);
    }
}
