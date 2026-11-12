//! Backend settings writes wait for a successful first reveal, or a bounded abort.
use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

pub(crate) struct Owner {
    gate: Arc<Gate>,
}

pub(crate) struct Gate {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Clone, Copy)]
enum State {
    Pending,
    Revealed,
    Aborted,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WaitError {
    Aborted,
    TimedOut,
}

impl Owner {
    pub(crate) fn new() -> Self {
        Self {
            gate: Arc::new(Gate {
                state: Mutex::new(State::Pending),
                changed: Condvar::new(),
            }),
        }
    }

    pub(crate) fn gate(&self) -> Arc<Gate> {
        self.gate.clone()
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.gate.abort();
    }
}

impl Gate {
    pub(crate) fn reveal(&self) {
        self.finish(State::Revealed);
    }

    pub(crate) fn abort(&self) {
        self.finish(State::Aborted);
    }

    fn finish(&self, target: State) {
        if let Ok(mut state) = self.state.lock() {
            if matches!(*state, State::Pending) {
                *state = target;
            }
            self.changed.notify_all();
        }
    }

    /// Only the existing blocking settings worker waits here. The UI only calls
    /// reveal/abort; dropping the attempt owner also wakes every waiter.
    pub(crate) fn wait(&self, timeout: Duration) -> Result<(), WaitError> {
        let started = Instant::now();
        let mut state = self.state.lock().map_err(|_| WaitError::Aborted)?;
        loop {
            match *state {
                State::Revealed => return Ok(()),
                State::Aborted => return Err(WaitError::Aborted),
                State::Pending => {
                    let remaining = timeout.saturating_sub(started.elapsed());
                    if remaining.is_zero() {
                        return Err(WaitError::TimedOut);
                    }
                    let (next, _) = self
                        .changed
                        .wait_timeout(state, remaining)
                        .map_err(|_| WaitError::Aborted)?;
                    state = next;
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, thread};

    #[test]
    fn startup_gate_write_waits_until_successful_reveal() {
        let owner = Owner::new();
        let gate = owner.gate();
        let worker_gate = gate.clone();
        let (done, received) = mpsc::channel();
        let worker = thread::spawn(move || {
            done.send(worker_gate.wait(Duration::from_secs(2))).unwrap();
        });
        assert!(received.recv_timeout(Duration::from_millis(40)).is_err());
        gate.reveal();
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            Ok(())
        );
        worker.join().unwrap();
    }

    #[test]
    fn startup_gate_owner_drop_wakes_all_pending_writes_without_admitting_them() {
        let owner = Owner::new();
        let gate = owner.gate();
        let (done, received) = mpsc::channel();
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let gate = gate.clone();
                let done = done.clone();
                thread::spawn(move || {
                    done.send(gate.wait(Duration::from_secs(2))).unwrap();
                })
            })
            .collect();
        assert!(received.recv_timeout(Duration::from_millis(40)).is_err());
        drop(owner);
        for _ in 0..2 {
            assert_eq!(
                received.recv_timeout(Duration::from_secs(1)).unwrap(),
                Err(WaitError::Aborted)
            );
        }
        gate.reveal();
        assert_eq!(gate.wait(Duration::ZERO), Err(WaitError::Aborted));
        for worker in workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn startup_gate_abort_wakes_a_waiter_and_prevents_later_reveal() {
        let owner = Owner::new();
        let gate = owner.gate();
        gate.abort();
        gate.reveal();
        assert_eq!(gate.wait(Duration::ZERO), Err(WaitError::Aborted));
    }

    #[test]
    fn startup_gate_has_a_deadline_and_allows_later_successful_reveal() {
        let owner = Owner::new();
        let gate = owner.gate();
        assert_eq!(
            gate.wait(Duration::from_millis(10)),
            Err(WaitError::TimedOut)
        );
        gate.reveal();
        drop(owner);
        assert_eq!(gate.wait(Duration::ZERO), Ok(()));
    }
}
