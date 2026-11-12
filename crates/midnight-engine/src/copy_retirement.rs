//! Async cancellation must not destroy provider owners on executor threads.
//! Normal paths await retirement. Fallback Drop protects future aborts; captured
//! inputs retain their existing bounded queue slots and tree capture permits.
use std::ops::{Deref, DerefMut};

pub(super) struct WorkerOwned<T: Send + 'static>(Option<T>);
impl<T: Send + 'static> WorkerOwned<T> {
    pub(super) fn new(value: T) -> Self {
        Self(Some(value))
    }
    pub(super) fn into_inner(mut self) -> T {
        self.0.take().expect("worker ownership is consumed once")
    }
}
impl<T: Send + 'static> Deref for WorkerOwned<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.as_ref().expect("worker ownership is live")
    }
}
impl<T: Send + 'static> DerefMut for WorkerOwned<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.0.as_mut().expect("worker ownership is live")
    }
}
impl<T: Send + 'static> Drop for WorkerOwned<T> {
    fn drop(&mut self) {
        if let Some(value) = self.0.take() {
            // Normal rejection awaits retirement. This fallback protects aborted
            // bounded consumers without admitting replacement work or resources.
            tokio::task::spawn_blocking(move || drop(value));
        }
    }
}
pub(super) async fn retire<T: Send + 'static>(value: T) {
    let _ = tokio::task::spawn_blocking(move || drop(value)).await;
}
