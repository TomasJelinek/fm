//! Bounded latest-value delivery: one summary per pane, at most four times a second.
use crate::Outbox;
use midnight_core::Event;
use std::{sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinSet, time::Instant};

const INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub(super) struct Publication {
    updates: Arc<[watch::Sender<Option<Event>>; 2]>,
}

impl Default for Publication {
    fn default() -> Self {
        Self {
            updates: Arc::new(std::array::from_fn(|_| watch::channel(None).0)),
        }
    }
}

impl Publication {
    pub(super) fn start(&self, outbox: Outbox, tasks: &mut JoinSet<()>) {
        for sender in self.updates.iter() {
            let mut receiver = sender.subscribe();
            let outbox = outbox.clone();
            tasks.spawn(async move {
                let mut next_publication = Instant::now();
                while receiver.changed().await.is_ok() {
                    tokio::time::sleep_until(next_publication).await;
                    // Reserve before taking the latest value. A full UI channel
                    // must not queue stale summaries or block native cache updates.
                    let Ok(permit) = outbox.sender.reserve().await else {
                        break;
                    };
                    let Some(event) = receiver.borrow_and_update().clone() else {
                        continue;
                    };
                    permit.send(event);
                    (outbox.wakeup)();
                    next_publication = Instant::now() + INTERVAL;
                }
            });
        }
    }

    pub(super) fn submit(&self, event: Event) {
        let Event::SelectedFolderSizes { pane, version, .. } = event else {
            return;
        };
        self.updates[pane.index()].send_if_modified(|pending| {
            if matches!(pending, Some(Event::SelectedFolderSizes { version: current, .. }) if *current >= version) {
                return false;
            }
            *pending = Some(event);
            true
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{
        NavigationId, PaneId, SelectedFolderSizes, SelectionRevision, SelectionSizeId,
    };
    use tokio::sync::mpsc;

    fn event(pane: PaneId, version: u64) -> Event {
        Event::SelectedFolderSizes {
            request: SelectionSizeId(1),
            pane,
            navigation: NavigationId(1),
            selection_revision: SelectionRevision(1),
            version,
            summary: SelectedFolderSizes {
                complete_bytes: Some(version),
                ..Default::default()
            },
        }
    }

    fn version(event: Event) -> u64 {
        let Event::SelectedFolderSizes { version, .. } = event else {
            panic!("summary expected")
        };
        version
    }

    #[tokio::test(start_paused = true)]
    async fn coalesces_bursts_at_exactly_four_hz_and_keeps_panes_independent() {
        let (sender, mut events) = mpsc::channel(8);
        let mut tasks = JoinSet::new();
        let publication = Publication::default();
        publication.start(
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        );
        publication.submit(event(PaneId::Left, 1));
        assert_eq!(version(events.recv().await.unwrap()), 1);
        for stamp in 2..=100 {
            publication.submit(event(PaneId::Left, stamp));
        }
        publication.submit(event(PaneId::Left, 99)); // Delayed older callback.
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(249)).await;
        tokio::task::yield_now().await;
        assert!(events.try_recv().is_err());
        publication.submit(event(PaneId::Right, 4));
        assert_eq!(version(events.recv().await.unwrap()), 4);
        tokio::time::advance(Duration::from_millis(1)).await;
        assert_eq!(version(events.recv().await.unwrap()), 100);
        tokio::time::advance(Duration::from_secs(1)).await;
        publication.submit(event(PaneId::Left, 101));
        assert_eq!(version(events.recv().await.unwrap()), 101);
        assert!(events.try_recv().is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn backpressure_keeps_only_the_latest_summary_without_blocking_producers() {
        let (sender, mut events) = mpsc::channel(1);
        sender.send(event(PaneId::Left, 0)).await.unwrap();
        let mut tasks = JoinSet::new();
        let publication = Publication::default();
        publication.start(
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        );
        publication.submit(event(PaneId::Left, 1));
        tokio::task::yield_now().await;
        for stamp in 2..=1000 {
            publication.submit(event(PaneId::Left, stamp));
        }
        assert_eq!(version(events.recv().await.unwrap()), 0);
        assert_eq!(version(events.recv().await.unwrap()), 1000);
        assert!(events.try_recv().is_err());
    }
}
