//! Bounded output waiting that cannot strand a completed maintenance mutation permit.
use super::*;
use midnight_core::Cancellation;

impl Worker {
    pub(super) fn emit(&mut self, event: TreeEvent) {
        let Some(cancel) = self
            .generation
            .as_ref()
            .map(|generation| generation.cancel.clone())
        else {
            return;
        };
        if let TreeEvent::Published(publication) = &event
            && self
                .awaiting_commit
                .as_ref()
                .is_some_and(|emitted| !Arc::ptr_eq(emitted, &publication.view))
        {
            // Only the latest unsent publication matters. Replaced projections
            // and their selection/report owners are retired on this model worker.
            if let TreeEvent::Published(publication) = event {
                self.pending_publication = Some(publication);
            }
            return;
        }
        let summary = match &event {
            TreeEvent::Published(publication) | TreeEvent::Reprojected { publication, .. } => self
                .sizes
                .enabled()
                .then(|| self.sizes.selected(publication)),
            _ => None,
        };
        if let TreeEvent::Published(publication) | TreeEvent::Reprojected { publication, .. } =
            &event
            && !self.inbox.publish(&publication.view)
        {
            return;
        }
        if let TreeEvent::Published(publication) | TreeEvent::Reprojected { publication, .. } =
            &event
        {
            self.awaiting_commit = Some(publication.view.clone());
        }
        // Exactly one branch can complete while this response waits. No commands
        // or new provider jobs run here, so this local publication cannot accumulate.
        let mut completion = self.send_event(Event::Tree(Box::new(event)), &cancel);
        if let Some(summary) = summary {
            let next = self.send_event(Event::Tree(Box::new(TreeEvent::Sizes(summary))), &cancel);
            debug_assert!(completion.is_none() || next.is_none());
            completion = completion.or(next);
        }
        if !cancel.is_cancelled()
            && let Some(event) = completion
        {
            // The original response and its summary precede the newer publication.
            // complete() already released the gate; this send may use ordinary backpressure.
            self.emit(event);
        }
    }

    fn send_event(&mut self, mut event: Event, cancel: &Cancellation) -> Option<TreeEvent> {
        let mut completion = None;
        loop {
            if cancel.is_cancelled() {
                return None;
            }
            if !self.maintenance_inflight {
                return self
                    .outbox
                    .blocking_send(event, cancel)
                    .then_some(completion)
                    .flatten();
            }
            match self.outbox.sender.try_send(event) {
                Ok(()) => {
                    (self.outbox.wakeup)();
                    return completion;
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => return None,
                Err(tokio::sync::mpsc::error::TrySendError::Full(unsent)) => event = unsent,
            }
            if let Some(result) = self.inbox.take_completion() {
                completion = self.complete(result);
                continue;
            }
            let sender = self.outbox.sender.clone();
            let inbox = self.inbox.clone();
            let ready = tokio::runtime::Handle::current().block_on(async {
                tokio::select! {
                    _ = inbox.wake.notified() => None,
                    permit = sender.reserve_owned() => Some(permit),
                }
            });
            match ready {
                Some(Ok(permit)) => {
                    if cancel.is_cancelled() {
                        return None;
                    }
                    permit.send(event);
                    (self.outbox.wakeup)();
                    return completion;
                }
                Some(Err(_)) => return None,
                None => {}
            }
        }
    }
}
