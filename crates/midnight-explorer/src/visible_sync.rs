//! Reliable retry of the UI's accepted snapshot through a bounded command queue.

use midnight_core::{Command, NavigationId, VisibleDirectory};

#[derive(Default)]
pub(crate) struct VisibleSync {
    sent: [Option<NavigationId>; 2],
}

impl VisibleSync {
    /// The caller claims before cloning the directory and supplies its scoped sender.
    pub(crate) fn send(&mut self, visible: VisibleDirectory, send: &dyn Fn(Command) -> bool) {
        let index = visible.pane.index();
        let navigation = visible.navigation;
        if self.sent[index] != Some(navigation) && send(Command::PaneCommitted(visible)) {
            self.sent[index] = Some(navigation);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::PaneId;
    use std::{cell::RefCell, sync::Arc};
    use tokio::sync::mpsc;

    // Boolean facade fixture only; production supplies the current ownership scope.
    fn sender<'a>(
        commands: &'a mpsc::Sender<Command>,
        refused: &'a RefCell<Vec<Command>>,
    ) -> impl Fn(Command) -> bool + 'a {
        move |command| match commands.try_send(command) {
            Ok(()) => true,
            Err(error) => {
                refused.borrow_mut().push(error.into_inner());
                false
            }
        }
    }

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
        let refused = RefCell::new(Vec::new());
        let send = sender(&commands, &refused);
        commands.try_send(Command::ListVolumes).unwrap();
        let mut sync = VisibleSync::default();
        sync.send(visible(PaneId::Left, 1), &send);
        assert_eq!(sync.sent[0], None);
        receiver.try_recv().unwrap();
        sync.send(visible(PaneId::Left, 1), &send);
        assert!(matches!(
            receiver.try_recv().unwrap(),
            Command::PaneCommitted(VisibleDirectory {
                navigation: NavigationId(1),
                ..
            })
        ));
        sync.send(visible(PaneId::Left, 1), &send);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn panes_are_independent_and_only_the_latest_retry_is_sent() {
        let (commands, mut receiver) = mpsc::channel(2);
        let refused = RefCell::new(Vec::new());
        let send = sender(&commands, &refused);
        let mut sync = VisibleSync::default();
        sync.send(visible(PaneId::Left, 1), &send);
        sync.send(visible(PaneId::Right, 1), &send);
        sync.send(visible(PaneId::Left, 2), &send);
        sync.send(visible(PaneId::Left, 3), &send);
        assert_eq!(sync.sent, [Some(NavigationId(1)); 2]);
        receiver.try_recv().unwrap();
        receiver.try_recv().unwrap();
        sync.send(visible(PaneId::Left, 3), &send);
        assert!(matches!(
            receiver.try_recv().unwrap(),
            Command::PaneCommitted(VisibleDirectory {
                navigation: NavigationId(3),
                ..
            })
        ));
    }
}
