//! The watch status a pane reports for the view it shows (`Event::WatchStatus`).
use super::*;
use midnight_core::TreeSessionId;

/// A view as the UI matches it: tree views by navigation and session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ViewKey {
    Flat(NavigationId),
    Tree(NavigationId, TreeSessionId),
}

impl ViewKey {
    pub(super) fn of(view: WatchedView) -> Self {
        match view {
            WatchedView::Flat { navigation, .. } => Self::Flat(navigation),
            WatchedView::Tree(stamp) => Self::Tree(stamp.navigation, stamp.session),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Native,
    PollingOnly,
    Restarting,
}

impl Kind {
    fn of(status: &WatchStatus) -> Self {
        match status {
            WatchStatus::Native => Self::Native,
            WatchStatus::PollingOnly(_) => Self::PollingOnly,
            WatchStatus::Restarting => Self::Restarting,
        }
    }
}

/// Sends a pane's status only when it differs from what the UI shows for its
/// view. A new view shows `Native`, so `Native` is sent only to clear an
/// earlier status of the same view. Sending never blocks: a full queue keeps
/// the newest status until it is sent (the worker then parks for at most
/// 10 ms), and a status for a view the pane no longer shows is dropped.
pub(super) struct StatusReport {
    /// Without a watch service nothing is reported.
    enabled: bool,
    /// The last status sent, and its view.
    sent: Option<(ViewKey, Kind)>,
    unsent: Option<(WatchedView, WatchStatus)>,
}

impl StatusReport {
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            sent: None,
            unsent: None,
        }
    }

    pub(super) fn set(&mut self, view: WatchedView, status: WatchStatus) {
        if !self.enabled {
            return;
        }
        let key = ViewKey::of(view);
        let shown = match self.sent {
            Some((sent, kind)) if sent == key => kind,
            _ => Kind::Native,
        };
        self.unsent = (Kind::of(&status) != shown).then_some((view, status));
    }

    pub(super) fn unsent(&self) -> bool {
        self.unsent.is_some()
    }

    /// Sends the unsent status while `shown` (evaluated only then) is its view.
    /// Returns false when the event queue is closed.
    pub(super) fn flush(
        &mut self,
        shown: impl FnOnce() -> Option<WatchedView>,
        outbox: &Outbox,
    ) -> bool {
        let Some((view, status)) = self.unsent.take() else {
            return true;
        };
        let key = ViewKey::of(view);
        if shown().map(ViewKey::of) != Some(key) {
            return true;
        }
        match outbox.sender.try_send(Event::WatchStatus {
            view,
            status: status.clone(),
        }) {
            Ok(()) => {
                self.sent = Some((key, Kind::of(&status)));
                (outbox.wakeup)();
                true
            }
            Err(TrySendError::Full(_)) => {
                self.unsent = Some((view, status));
                true
            }
            Err(TrySendError::Closed(_)) => false,
        }
    }
}
