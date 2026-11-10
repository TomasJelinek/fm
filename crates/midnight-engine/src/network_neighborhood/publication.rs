//! A nonblocking enqueue and snapshot update share one generation fence. Never
//! wait for channel capacity or retire old rows while holding the snapshot lock.
use super::*;
use tokio::sync::mpsc::error::TrySendError;

#[derive(Clone)]
pub(super) struct Publication {
    outbox: Outbox,
    snapshots: Snapshots,
    pane: PaneId,
    navigation: NavigationId,
    previous: Option<NavigationId>,
    cancel: Cancellation,
}
enum Attempt {
    Sent,
    Retry(Box<NetworkUpdate>),
    Obsolete,
}
impl Publication {
    pub(super) fn new(
        outbox: Outbox,
        snapshots: Snapshots,
        pane: PaneId,
        navigation: NavigationId,
        previous: Option<NavigationId>,
        cancel: Cancellation,
    ) -> Self {
        Self {
            outbox,
            snapshots,
            pane,
            navigation,
            previous,
            cancel,
        }
    }
    fn attempt(&self, update: Box<NetworkUpdate>) -> Attempt {
        let mut states = self
            .snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let state = &mut states[self.pane.index()];
        if self.cancel.is_cancelled() || state.network_navigation != Some(self.navigation) {
            drop(states);
            return Attempt::Obsolete;
        }
        let entries = match update.as_ref() {
            NetworkUpdate::Listing { listing, .. } => Some(listing.entries.clone()),
            _ => None,
        };
        let failed = matches!(update.as_ref(), NetworkUpdate::Failed(_));
        let event = Event::NetworkUpdated {
            pane: self.pane,
            navigation: self.navigation,
            update,
        };
        let result = self.outbox.sender.try_send(event);
        let mut retired = None;
        if result.is_ok() {
            if let Some(entries) = entries {
                retired = state.network.replace((self.navigation, entries));
            } else if failed
                && state
                    .network
                    .as_ref()
                    .is_none_or(|(navigation, _)| *navigation != self.navigation)
            {
                state.network_navigation = self.previous;
            }
        }
        drop(states);
        drop(retired);
        match result {
            Ok(()) => {
                (self.outbox.wakeup)();
                Attempt::Sent
            }
            Err(TrySendError::Full(Event::NetworkUpdated { update, .. })) => Attempt::Retry(update),
            Err(_) => Attempt::Obsolete,
        }
    }
    pub(super) fn blocking_send(&self, update: NetworkUpdate) -> bool {
        let mut update = Box::new(update);
        loop {
            match self.attempt(update) {
                Attempt::Sent => return true,
                Attempt::Obsolete => return false,
                Attempt::Retry(pending) => update = pending,
            }
            std::thread::park_timeout(Duration::from_millis(10));
        }
    }
    pub(super) async fn send_failure(&self, error: NetworkFailure) {
        let mut update = Box::new(NetworkUpdate::Failed(error));
        loop {
            match self.attempt(update) {
                Attempt::Sent | Attempt::Obsolete => return,
                Attempt::Retry(pending) => update = pending,
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
