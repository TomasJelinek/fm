//! The suspended engine future is owned before its first poll.
use crate::{EngineServices, retirement::*};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

type RunFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
struct Run {
    future: RunFuture,
    // Keep original service owners until after the future/controllers are retired,
    // including normal completion, whose inner future drops locals during poll.
    // Independent actors must additionally guard their own last-owner boundaries.
    _services: EngineServices,
}
/// Engine execution owned before its first poll; dropping it schedules retirement.
///
/// Completion acknowledges normal retirement. Aborting never waits on provider
/// destruction or outstanding command permits on the calling thread.
pub struct EngineRun {
    owned: Option<WorkerOwned<Run>>,
    finishing: Option<Retirement>,
}
impl EngineRun {
    pub(crate) fn new(
        lease: RetirementLease,
        future: impl Future<Output = ()> + Send + 'static,
        services: EngineServices,
    ) -> Self {
        Self {
            owned: Some(lease.own(Run {
                future: Box::pin(future),
                _services: services,
            })),
            finishing: None,
        }
    }
}
impl Future for EngineRun {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        if let Some(run) = self.owned.as_mut() {
            if run.future.as_mut().poll(context).is_pending() {
                return Poll::Pending;
            }
            self.finishing = Some(self.owned.take().expect("completed run is owned").retire());
        }
        if let Some(retirement) = self.finishing.as_mut() {
            match Pin::new(retirement).poll(context) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => eprintln!("Engine cleanup failed: {error}"),
                Poll::Ready(Ok(())) => {}
            }
            self.finishing = None;
        }
        Poll::Ready(())
    }
}
