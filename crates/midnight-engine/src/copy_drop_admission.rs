//! Native drops arm their original registration before publishing either fence.
use super::*;

pub(crate) struct StagedDrop {
    admission: Admission,
    owned: Option<Owned<()>>,
    payload: Option<oneshot::Sender<Request>>,
    receiver: Option<oneshot::Receiver<Request>>,
    completions: Option<Arc<HashMap<std::path::PathBuf, TreeCopyOccurrence>>>,
    retained_registration: bool,
    publication_started: bool,
}

impl Admission {
    /// Reserve the existing actual capacity and retirement slot before any drag
    /// state/snapshot mutex. No provider method or visible admission runs here.
    pub(crate) fn stage_drop(&self) -> Result<StagedDrop, FsError> {
        if self.0.capacity.is_closed() || self.0.cancel.is_cancelled() {
            return Err(closed());
        }
        let lease = self.0.retirement.try_reserve().map_err(|_| busy())?;
        let capacity = self.0.capacity.clone().try_acquire_owned().map_err(|_| {
            if self.0.capacity.is_closed() {
                closed()
            } else {
                busy()
            }
        })?;
        let resources = ownership::Resources::unpublished(capacity, self.0.cancel.pausable_child());
        let owned = lease.own(Scope::new((), resources));
        let (payload, receiver) = oneshot::channel();
        Ok(StagedDrop {
            admission: self.clone(),
            owned: Some(owned),
            payload: Some(payload),
            receiver: Some(receiver),
            completions: None,
            retained_registration: false,
            publication_started: false,
        })
    }
}

impl StagedDrop {
    /// Move the original registration into the unpublished sealed owner. Only
    /// its provider-free completion map is cloned; no capture getter is called.
    pub(crate) fn retain_registration(&mut self, retention: Arc<crate::drag_drop::Retention>) {
        let completions = retention.completions.clone();
        self.retained_registration = true;
        self.owned
            .as_mut()
            .expect("staged drop remains owned")
            .resources_mut()
            .retain_checked_provider(Box::new(crate::drag_drop::RegistrationOwner::new(
                retention,
            )));
        self.completions = Some(completions);
    }

    /// Publish the already armed native ownership in the shared ordered ledger.
    /// All fallible admission checks precede publication. The guard parameter
    /// outlives both metadata mutex guards on every return and unwind path.
    pub(crate) fn publish(mut self) -> Result<Reservation, FsError> {
        let admission = self.admission.clone();
        let mut state = match admission.0.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(busy()),
        };
        if admission.0.capacity.is_closed() || admission.0.cancel.is_cancelled() {
            drop(state);
            return Err(closed());
        }
        if state.slots.len() >= MAX_JOBS {
            drop(state);
            return Err(busy());
        }
        let sequence = match state.finished.prepare_sequence() {
            Ok(sequence) => sequence,
            Err(_) => {
                drop(state);
                return Err(busy());
            }
        };
        let mut fence = match admission.0.fence.try_registration() {
            Ok(fence) => fence,
            Err(()) => {
                drop(state);
                return Err(busy());
            }
        };
        // A partial publication unwind must retire even an empty stage. It may
        // never take the unpublished metadata-only rollback once this begins.
        self.publication_started = true;
        let resources = self
            .owned
            .as_mut()
            .expect("staged drop remains owned")
            .resources_mut();
        resources.publish_admission(sequence, &mut state.finished, &mut fence);
        let pending = resources.receipt_observer();
        state.slots.push_back(Slot {
            payload: self
                .receiver
                .take()
                .expect("staged drop owns its payload receiver"),
            pending: Some(pending),
        });
        let payload = self
            .payload
            .take()
            .expect("published drop owns its payload sender");
        let completions = self.completions.take();
        let owned = self
            .owned
            .take()
            .expect("published drop keeps its original sealed owner");
        let reservation = Reservation {
            owned,
            payload: Payload::new(payload, &admission),
            completions,
        };
        drop(fence);
        drop(state);
        admission.0.ready.notify_one();
        Ok(reservation)
    }
}

impl Drop for StagedDrop {
    fn drop(&mut self) {
        let Some(owned) = self.owned.take() else {
            return;
        };
        if self.retained_registration || self.publication_started {
            // The existing worker performs consuming provider release; a refused
            // publication never confirms FINISHED from a missing slot or token.
            drop(owned);
        } else {
            ownership::rollback_empty_reservation(owned);
        }
    }
}

fn busy() -> FsError {
    error(
        Path::new(""),
        io::ErrorKind::WouldBlock,
        "Native drop admission or retirement capacity is occupied.",
    )
}
fn closed() -> FsError {
    error(
        Path::new(""),
        io::ErrorKind::Interrupted,
        "Copy admission is closing.",
    )
}

#[cfg(test)]
#[path = "copy_drop_admission_tests.rs"]
mod tests;
