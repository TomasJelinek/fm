//! The parent pools drop an entry they do not hand out only after their lock is released.
//! Each pooled type's Drop locks its pool to remove its own entry, and the entry the pool
//! upgraded can become its last reference while the pool checks it: its other holders let
//! go meanwhile (on another thread; here, inside the check). Dropping it under the lock
//! would lock the pool twice on one thread, which never returns.
use super::*;
use std::{sync::mpsc, time::Duration};

/// A pooled entry whose Drop, like `Anchor`'s, removes its own entry from its pool.
struct Probe {
    identity: TransferIdentity,
}
static PROBES: OnceLock<Pool<Probe>> = OnceLock::new();
impl Drop for Probe {
    fn drop(&mut self) {
        let mut pool = probes().lock().unwrap_or_else(|e| e.into_inner());
        if pool
            .get(&self.identity)
            .is_some_and(|value| std::ptr::eq(value.as_ptr(), self))
        {
            pool.remove(&self.identity);
        }
    }
}
fn probes() -> &'static Pool<Probe> {
    PROBES.get_or_init(Default::default)
}

/// A pooled probe under its own identity, and its only other holder.
fn pooled_probe(object: u8) -> (TransferIdentity, Arc<Probe>) {
    let identity = TransferIdentity {
        device: 0x5eed,
        object: [object; 16],
    };
    let probe = Arc::new(Probe { identity });
    probes()
        .lock()
        .unwrap()
        .insert(identity, Arc::downgrade(&probe));
    (identity, probe)
}

/// Runs `test` on a thread of its own; fails if it does not return, as a thread that
/// locked a pool twice never does.
fn returns(test: impl FnOnce() + Send + 'static) {
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        test();
        let _ = done.send(());
    });
    finished
        .recv_timeout(Duration::from_secs(10))
        .expect("the pool was locked twice on one thread: dropping a stale entry deadlocked");
}

#[test]
fn a_stale_entry_whose_holders_let_go_during_the_check_is_dropped_unlocked() {
    returns(|| {
        let (identity, holder) = pooled_probe(1);
        let holder = std::cell::Cell::new(Some(holder));
        let found = pooled(probes(), identity, |_| {
            // The last other holder lets go while the pool checks the entry.
            drop(holder.take());
            false
        });
        assert!(found.is_none());
        // Its Drop removed its own entry.
        assert!(!probes().lock().unwrap().contains_key(&identity));
    });
}

#[test]
fn a_candidate_replaces_a_stale_entry_whose_holders_let_go_during_the_check() {
    returns(|| {
        let (identity, holder) = pooled_probe(2);
        let holder = std::cell::Cell::new(Some(holder));
        let candidate = Arc::new(Probe { identity });
        let pooled = pool_or_found(
            probes(),
            identity,
            |_| {
                drop(holder.take());
                false
            },
            candidate.clone(),
        );
        assert!(Arc::ptr_eq(&pooled, &candidate));
        let entry = probes().lock().unwrap().get(&identity).cloned();
        assert!(entry.is_some_and(|entry| std::ptr::eq(entry.as_ptr(), Arc::as_ptr(&candidate))));
    });
}

#[test]
fn a_current_entry_is_handed_out_and_the_candidate_dropped_unlocked() {
    returns(|| {
        let (identity, holder) = pooled_probe(3);
        let candidate = Arc::new(Probe { identity });
        let pooled = pool_or_found(probes(), identity, |_| true, candidate);
        assert!(Arc::ptr_eq(&pooled, &holder));
    });
}
