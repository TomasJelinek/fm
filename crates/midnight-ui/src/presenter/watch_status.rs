//! Non-modal presentation of how the backend watches a pane's view.
use super::*;

/// The backend registers no native change notifications for the view.
const POLLING_ONLY: &str = "Change notifications are unavailable for this folder; it is checked periodically (about every 15 seconds).";
/// The pane's watch worker waits for its rate-limited restart.
const RESTARTING: &str =
    "Automatic refresh is paused after an internal error; it is retried at most once a minute.";

impl Presenter {
    /// Keeps the newest status of the pane's watch; it is presented only while
    /// its view is the pane's current view.
    pub(super) fn apply_watch_status(&mut self, view: WatchedView, status: WatchStatus) {
        let p = &mut self.panes[view.pane().index()];
        self.retired
            .push(Box::new(p.watch_status.replace((view, status))));
    }

    /// The pane's watch status text, never a panel error or a dialog: shown
    /// while the reported view is the pane's current view (a flat navigation
    /// without a tree, or the tree's navigation and session) and it is not
    /// watched natively.
    pub(crate) fn watch_status_text(&self, pane: PaneId) -> Option<&'static str> {
        let p = &self.panes[pane.index()];
        let (view, status) = p.watch_status.as_ref()?;
        let current = match view {
            WatchedView::Flat { navigation, .. } => {
                p.tree_view().is_none() && p.successful_generation == Some(*navigation)
            }
            WatchedView::Tree(stamp) => p.tree_view().is_some_and(|view| {
                view.stamp.navigation == stamp.navigation && view.stamp.session == stamp.session
            }),
        };
        match status {
            _ if !current => None,
            WatchStatus::Native => None,
            WatchStatus::PollingOnly(_) => Some(POLLING_ONLY),
            WatchStatus::Restarting => Some(RESTARTING),
        }
    }
}
