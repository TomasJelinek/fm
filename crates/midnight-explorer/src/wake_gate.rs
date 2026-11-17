use std::sync::atomic::{AtomicBool, Ordering};

/// Coalesces backend notifications into at most one pending UI callback.
///
/// Producers publish data, then `request`; the consumer `release`s, then rechecks the
/// data. Both sides use read-modify-write operations on the one flag, which totally
/// orders them: either the producer's request sees the release and schedules a new
/// callback, or the release synchronizes with that request and the recheck sees the data.
/// A plain release store would allow both sides to miss each other (store buffering).
#[derive(Default)]
pub struct WakeGate(AtomicBool);

impl WakeGate {
    pub fn request(&self) -> bool {
        !self.0.swap(true, Ordering::AcqRel)
    }
    pub fn release(&self) {
        self.0.swap(false, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicBool, AtomicUsize},
    };

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

    /// Store-buffering litmus test of the release-then-recheck handshake: in every round a
    /// callback is pending, the consumer releases and rechecks the queue while a producer
    /// publishes and requests. At least one side must observe the other. The run is
    /// bounded by a round count and a short time budget so a loaded machine stays fast.
    #[test]
    fn release_and_recheck_never_miss_a_concurrent_publication() {
        const ROUNDS: usize = 50_000;
        const STOP: usize = usize::MAX;
        let gate = Arc::new(WakeGate::default());
        let queued = Arc::new(AtomicBool::new(false));
        let round = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicUsize::new(0));
        let observed = Arc::new(AtomicBool::new(false));
        let scheduled = Arc::new(AtomicBool::new(false));
        // Run one side per round until the coordinator stops the test.
        let worker = |side: fn(&WakeGate, &AtomicBool, &AtomicBool), result: &Arc<AtomicBool>| {
            let (gate, queued, round, done, result) = (
                gate.clone(),
                queued.clone(),
                round.clone(),
                done.clone(),
                result.clone(),
            );
            std::thread::spawn(move || {
                let mut last = 0;
                loop {
                    let current = loop {
                        let current = round.load(Ordering::Acquire);
                        if current != last {
                            break current;
                        }
                        std::thread::yield_now();
                    };
                    if current == STOP {
                        return;
                    }
                    side(&gate, &queued, &result);
                    done.fetch_add(1, Ordering::AcqRel);
                    last = current;
                }
            })
        };
        fn consume(gate: &WakeGate, queued: &AtomicBool, observed: &AtomicBool) {
            gate.release();
            observed.store(queued.load(Ordering::Acquire), Ordering::Relaxed);
        }
        fn publish(gate: &WakeGate, queued: &AtomicBool, scheduled: &AtomicBool) {
            queued.store(true, Ordering::Release);
            scheduled.store(gate.request(), Ordering::Relaxed);
        }
        let consumer = worker(consume, &observed);
        let producer = worker(publish, &scheduled);
        let started = std::time::Instant::now();
        let mut missed = 0;
        let mut rounds = 0;
        while rounds < ROUNDS && started.elapsed() < std::time::Duration::from_millis(300) {
            rounds += 1;
            // A callback is pending and the queue is empty when the round starts.
            gate.0.store(true, Ordering::SeqCst);
            queued.store(false, Ordering::SeqCst);
            round.store(rounds, Ordering::Release);
            while done.load(Ordering::Acquire) != rounds * 2 {
                std::thread::yield_now();
            }
            if !observed.load(Ordering::Relaxed) && !scheduled.load(Ordering::Relaxed) {
                missed += 1;
            }
        }
        round.store(STOP, Ordering::Release);
        consumer.join().unwrap();
        producer.join().unwrap();
        assert_eq!(
            missed, 0,
            "{missed} of {rounds} publications were neither rechecked nor scheduled"
        );
    }
}
