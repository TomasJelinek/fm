//! One retirement reservation follows a copy and its capacity through every phase.
use super::*;
use std::ops::{Deref, DerefMut};

pub(super) struct Resources {
    pub receipt: crate::copy_fence::Receipt,
    // Declared after payload in Scope; neither permit may escape native teardown.
    pub mutation: Option<OwnedSemaphorePermit>,
    pub _capacity: OwnedSemaphorePermit,
}
pub(super) struct Scope<T> {
    pub payload: T,
    pub resources: Resources,
}
pub(super) type Owned<T> = WorkerOwned<Scope<T>>;
impl<T> Deref for Scope<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.payload
    }
}
impl<T> DerefMut for Scope<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.payload
    }
}
// Only ownership-preserving transformations belong on the actor. Work which
// destroys provider-bearing values must call this from a blocking worker.
pub(super) fn map<T: Send + 'static, U: Send + 'static>(
    value: Owned<T>,
    transform: impl FnOnce(T) -> U,
) -> Owned<U> {
    let (scope, lease) = value.into_parts();
    let Scope { payload, resources } = scope;
    lease.own(Scope {
        payload: transform(payload),
        resources,
    })
}
pub(super) fn finish(job: Owned<worker::Job>) -> CopyProgress {
    let (scope, lease) = job.into_parts();
    let Scope { payload, resources } = scope;
    let progress = payload.finish();
    resources.receipt.finish(&progress);
    drop(resources);
    drop(lease);
    progress
}

#[cfg(test)]
#[path = "copy_ownership_tests.rs"]
mod tests;
