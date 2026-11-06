//! Reliable retry of the UI's accepted snapshot through a bounded command queue.

use midnight_core::{Command, NavigationId, VisibleDirectory};
use tokio::sync::mpsc;

#[derive(Default)]
pub(crate) struct VisibleSync {
    sent: [Option<NavigationId>; 2],
}

impl VisibleSync {
    pub(crate) fn send(&mut self, visible: VisibleDirectory, commands: &mpsc::Sender<Command>) {
        let index = visible.pane.index();
        let navigation = visible.navigation;
        if self.sent[index] != Some(navigation)
            && commands.try_send(Command::PaneCommitted(visible)).is_ok()
        {
            self.sent[index] = Some(navigation);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::PaneId;
    use std::sync::Arc;

    fn visible(pane: PaneId, generation: u64) -> VisibleDirectory {
        VisibleDirectory {
            pane,
            navigation: NavigationId(generation),
            path: "文档/📁".into(),
            entries: Arc::from([]),
        }
    }

    #[test]
    fn full_queue_retries_the_same_snapshot_without_marking_it_delivered() {
        let (commands, mut receiver) = mpsc::channel(1);
        commands.try_send(Command::ListVolumes).unwrap();
        let mut sync = VisibleSync::default();
        sync.send(visible(PaneId::Left, 1), &commands);
        assert_eq!(sync.sent[0], None);
        receiver.try_recv().unwrap();
        sync.send(visible(PaneId::Left, 1), &commands);
        assert!(matches!(
            receiver.try_recv().unwrap(),
            Command::PaneCommitted(VisibleDirectory {
                navigation: NavigationId(1),
                ..
            })
        ));
        sync.send(visible(PaneId::Left, 1), &commands);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn panes_are_independent_and_only_the_latest_retry_is_sent() {
        let (commands, mut receiver) = mpsc::channel(2);
        let mut sync = VisibleSync::default();
        sync.send(visible(PaneId::Left, 1), &commands);
        sync.send(visible(PaneId::Right, 1), &commands);
        sync.send(visible(PaneId::Left, 2), &commands);
        sync.send(visible(PaneId::Left, 3), &commands);
        assert_eq!(sync.sent, [Some(NavigationId(1)); 2]);
        receiver.try_recv().unwrap();
        receiver.try_recv().unwrap();
        sync.send(visible(PaneId::Left, 3), &commands);
        assert!(matches!(
            receiver.try_recv().unwrap(),
            Command::PaneCommitted(VisibleDirectory {
                navigation: NavigationId(3),
                ..
            })
        ));
    }
}
