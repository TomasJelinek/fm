//! Shared final service ownership across independently cancelled child work.
use crate::retirement::WorkerOwned;
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

/// Only already guarded owners can enter this anchor. Outputs containing newly
/// captured provider/native resources still need their own admitted job guard.
#[derive(Clone)]
pub(crate) struct TaskLifetime {
    _owner: Arc<dyn Send + Sync>,
}
impl TaskLifetime {
    pub(crate) fn new<T: Send + Sync + 'static>(owner: Arc<WorkerOwned<T>>) -> Self {
        Self { _owner: owner }
    }
    pub(crate) fn future<F: Future>(&self, future: F) -> TaskFuture<F> {
        TaskFuture {
            future: Box::pin(future),
            _lifetime: self.clone(),
        }
    }
    /// The complete closure remains anchored even if cancelled before invocation.
    /// Its result is deliberately not covered by this service-only envelope.
    pub(crate) fn blocking<F, R>(&self, work: F) -> impl FnOnce() -> R + Send + 'static
    where
        F: FnOnce() -> R + Send + 'static,
    {
        let work = Blocking {
            work: Some(work),
            _lifetime: self.clone(),
        };
        move || work.run()
    }
    #[cfg(test)]
    pub(crate) fn testing<T: Send + Sync + 'static>(owner: T) -> Self {
        let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
        Self::new(Arc::new(domain.try_reserve().unwrap().own(owner)))
    }
}

// Declaration order is intentional: captured service references must be dropped
// before the last guarded owner, on Ready, panic, unpolled drop and cancellation.
pub(crate) struct TaskFuture<F: Future> {
    future: Pin<Box<F>>,
    _lifetime: TaskLifetime,
}
impl<F: Future> Future for TaskFuture<F> {
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().future.as_mut().poll(context)
    }
}
struct Blocking<F> {
    work: Option<F>,
    _lifetime: TaskLifetime,
}
impl<F> Blocking<F> {
    fn run<R>(mut self) -> R
    where
        F: FnOnce() -> R,
    {
        self.work.take().expect("owned closure is invoked once")()
    }
}
