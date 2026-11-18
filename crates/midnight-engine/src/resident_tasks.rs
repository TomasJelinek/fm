//! Completion and abort authority for Engine actors formerly in a blocking JoinSet.

use midnight_work_budget::resident::{TaskError, Ticket};
use std::{
    future::{Future, poll_fn},
    pin::Pin,
    task::Poll,
};

#[derive(Default)]
pub(crate) struct ResidentTasks(Vec<Ticket>);

impl ResidentTasks {
    pub(crate) fn insert(&mut self, ticket: Ticket) {
        self.0.push(ticket);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn abort_all(&self) {
        for ticket in &self.0 {
            ticket.abort();
        }
    }

    pub(crate) async fn join_next(&mut self) -> Option<Result<(), TaskError>> {
        poll_fn(|context| {
            for (index, ticket) in self.0.iter_mut().enumerate() {
                if let Poll::Ready(result) = Pin::new(ticket).poll(context) {
                    self.0.swap_remove(index);
                    return Poll::Ready(Some(result));
                }
            }
            if self.is_empty() {
                Poll::Ready(None)
            } else {
                Poll::Pending
            }
        })
        .await
    }
}

impl Drop for ResidentTasks {
    fn drop(&mut self) {
        self.abort_all();
    }
}
