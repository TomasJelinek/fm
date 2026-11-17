//! Admission-ordered snapshots of confirmed Copy/Move retirement.
use midnight_core::Cancellation;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

struct Completion {
    sequence: u64,
    finished: AtomicBool,
    ready: Notify,
}

/// Accessed only under the admission mutex, together with its payload slots.
pub(crate) struct Registry {
    last: u64,
    // Strong entries deliberately preserve unconfirmed, vanished owners.
    pending: Vec<Arc<Completion>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            last: 0,
            pending: Vec::with_capacity(crate::copy_operations::MAX_JOBS),
        }
    }
}

/// Only the sealed Copy owner may commit after all native teardown returns.
/// Dropping this token does not confirm retirement.
pub(crate) struct Owner(Arc<Completion>);

/// Metadata-only observation; this carries no authority to confirm retirement.
#[derive(Clone)]
pub(crate) struct FinishedObserver(Arc<Completion>);

#[derive(Clone, Default)]
pub(crate) struct Predecessors(Vec<Arc<Completion>>);

impl Registry {
    #[cfg(test)]
    pub(crate) fn register(&mut self) -> Result<(Owner, Predecessors), &'static str> {
        let mut owner = Owner::unpublished();
        let mut predecessors = Predecessors::with_capacity();
        let sequence = self.prepare_sequence()?;
        self.publish_prepared(sequence, &mut owner, &mut predecessors);
        Ok((owner, predecessors))
    }

    /// Caller holds the admission mutex continuously through publication.
    /// Refusal and repeated preparation never consume a sequence or add a latch.
    pub(crate) fn prepare_sequence(&mut self) -> Result<u64, &'static str> {
        self.pending
            .retain(|entry| !entry.finished.load(Ordering::Acquire));
        if self.pending.len() >= crate::copy_operations::MAX_JOBS {
            return Err("Copy retirement remains unconfirmed for 32 admissions.");
        }
        self.last
            .checked_add(1)
            .ok_or("Copy admission sequence exhausted.")
    }

    /// Publish an already sealed, exclusively owned staged latch. All storage
    /// was allocated before entering the admission critical section.
    pub(crate) fn publish_prepared(
        &mut self,
        sequence: u64,
        owner: &mut Owner,
        predecessors: &mut Predecessors,
    ) {
        assert_eq!(
            self.last.checked_add(1),
            Some(sequence),
            "prepared admission sequence changed"
        );
        assert!(self.pending.len() < crate::copy_operations::MAX_JOBS);
        assert!(self.pending.capacity() >= crate::copy_operations::MAX_JOBS);
        assert!(predecessors.0.is_empty());
        assert!(predecessors.0.capacity() >= crate::copy_operations::MAX_JOBS);
        let completion = Arc::get_mut(&mut owner.0).expect("unpublished owner has no observers");
        assert_eq!(completion.sequence, 0);
        assert!(!completion.finished.load(Ordering::Acquire));
        completion.sequence = sequence;
        predecessors.0.extend(self.pending.iter().cloned());
        self.pending.push(owner.0.clone());
        self.last = sequence;
    }
}

impl Owner {
    /// Allocate outside the admission mutex; this is not registered or visible.
    pub(crate) fn unpublished() -> Self {
        Self(Arc::new(Completion {
            sequence: 0,
            finished: AtomicBool::new(false),
            ready: Notify::new(),
        }))
    }

    pub(crate) fn sequence(&self) -> u64 {
        self.0.sequence
    }

    pub(crate) fn finish(self) {
        self.0.finished.store(true, Ordering::Release);
        self.0.ready.notify_waiters();
    }

    /// Obtain only after publish_prepared assigned the immutable sequence.
    pub(crate) fn observer(&self) -> FinishedObserver {
        assert_ne!(
            self.sequence(),
            0,
            "unpublished owner cannot have observers"
        );
        FinishedObserver(self.0.clone())
    }
}

impl FinishedObserver {
    pub(crate) fn is_finished(&self) -> bool {
        self.0.finished.load(Ordering::Acquire)
    }
}

impl Predecessors {
    pub(crate) fn with_capacity() -> Self {
        Self(Vec::with_capacity(crate::copy_operations::MAX_JOBS))
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.0
            .iter()
            .all(|entry| entry.finished.load(Ordering::Acquire))
    }

    /// False means cancellation; neither cancellation nor owner loss completes a latch.
    pub(crate) async fn wait(&self, cancel: &Cancellation) -> bool {
        for completion in &self.0 {
            loop {
                let notified = completion.ready.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if cancel.is_cancelled() {
                    return false;
                }
                if completion.finished.load(Ordering::Acquire) {
                    break;
                }
                tokio::select! {
                    _ = notified => {},
                    _ = tokio::time::sleep(Duration::from_millis(25)) => {},
                }
            }
        }
        !cancel.is_cancelled()
    }
}

#[cfg(test)]
#[path = "copy_finished_tests.rs"]
mod tests;

#[cfg(test)]
mod prepared_tests {
    use super::*;

    #[test]
    fn unpublished_owner_and_preparation_do_not_publish_or_consume_a_sequence() {
        let mut registry = Registry::default();
        let unpublished = Owner::unpublished();
        assert_eq!(unpublished.sequence(), 0);
        assert_eq!(registry.prepare_sequence(), Ok(1));
        assert_eq!(registry.prepare_sequence(), Ok(1));
        assert!(registry.pending.is_empty());
        assert_eq!(registry.last, 0);
        drop(unpublished);
        let (first, earlier) = registry.register().unwrap();
        assert_eq!(first.sequence(), 1);
        assert!(earlier.is_finished());
        first.finish();
    }

    #[test]
    fn prepared_publication_reuses_storage_and_captures_only_earlier_owners() {
        let mut registry = Registry::default();
        let registry_storage = registry.pending.as_ptr();
        let mut first = Owner::unpublished();
        let first_identity = Arc::as_ptr(&first.0);
        let mut first_wait = Predecessors::with_capacity();
        let first_storage = first_wait.0.as_ptr();
        let sequence = registry.prepare_sequence().unwrap();
        registry.publish_prepared(sequence, &mut first, &mut first_wait);
        let mut second = Owner::unpublished();
        let mut second_wait = Predecessors::with_capacity();
        let second_storage = second_wait.0.as_ptr();
        let sequence = registry.prepare_sequence().unwrap();
        registry.publish_prepared(sequence, &mut second, &mut second_wait);
        let (third, third_wait) = registry.register().unwrap();
        assert_eq!(registry.pending.as_ptr(), registry_storage);
        assert_eq!(first_wait.0.as_ptr(), first_storage);
        assert_eq!(second_wait.0.as_ptr(), second_storage);
        assert_eq!(Arc::as_ptr(&registry.pending[0]), first_identity);
        assert_eq!(second_wait.0.len(), 1);
        assert_eq!(
            (first.sequence(), second.sequence(), third.sequence()),
            (1, 2, 3)
        );
        assert!(first_wait.is_finished());
        assert!(!second_wait.is_finished() && !third_wait.is_finished());
        first.finish();
        assert!(second_wait.is_finished());
        assert!(!third_wait.is_finished());
        second.finish();
        assert!(third_wait.is_finished());
        third.finish();
    }

    #[test]
    fn prepare_refuses_full_or_exhausted_registry_without_publishing() {
        let mut registry = Registry::default();
        let mut owners = Vec::new();
        for _ in 0..crate::copy_operations::MAX_JOBS {
            owners.push(registry.register().unwrap().0);
        }
        let storage = registry.pending.as_ptr();
        let last = registry.last;
        assert!(registry.prepare_sequence().is_err());
        assert_eq!(registry.last, last);
        assert_eq!(registry.pending.len(), crate::copy_operations::MAX_JOBS);
        owners.pop().unwrap().finish();
        assert_eq!(registry.prepare_sequence(), Ok(last + 1));
        assert_eq!(registry.last, last);
        assert_eq!(registry.pending.as_ptr(), storage);
        for owner in owners {
            owner.finish();
        }
        registry.last = u64::MAX;
        assert!(registry.prepare_sequence().is_err());
        assert_eq!(registry.last, u64::MAX);
        assert!(registry.pending.is_empty());
    }

    #[test]
    fn observer_only_confirms_explicit_finish_and_never_owner_loss() {
        let mut registry = Registry::default();
        let (owner, _) = registry.register().unwrap();
        let observer = owner.observer();
        drop(owner);
        assert!(!observer.is_finished());
        let (confirmed, _) = registry.register().unwrap();
        let confirmed_observer = confirmed.observer();
        confirmed.finish();
        assert!(confirmed_observer.is_finished());
        registry.prepare_sequence().unwrap();
        assert!(confirmed_observer.is_finished());
        assert!(!observer.is_finished());
        assert_eq!(registry.pending.len(), 1);
    }
}
