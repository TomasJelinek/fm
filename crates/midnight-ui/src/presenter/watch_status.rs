//! Non-modal presentation of how the backend watches a pane's view.
use super::*;

/// The backend could not register native change notifications for the view
/// for a while (a flat folder's registration keeps being retried meanwhile).
const POLLING_ONLY: &str =
    "Live updates are unavailable for this folder; it is checked for changes regularly instead.";
/// The pane's watch worker waits for its rate-limited restart.
const RESTARTING: &str =
    "Automatic refresh is paused after an internal error; it is retried at most once a minute.";

/// A pane's watch status as the pane shows it: never a panel error or a dialog.
pub(crate) struct WatchNotice {
    pub(crate) text: &'static str,
    /// Why notifications are unavailable (the registration error), or empty.
    pub(crate) details: String,
}

impl Presenter {
    /// Keeps the newest status of the pane's watch; it is presented only while
    /// its view is the pane's current view.
    pub(super) fn apply_watch_status(&mut self, view: WatchedView, status: WatchStatus) {
        let p = &mut self.panes[view.pane().index()];
        self.retired
            .push(Box::new(p.watch_status.replace((view, status))));
    }

    /// The pane's watch status while the reported view is the pane's current
    /// view (a flat navigation without a tree, or the tree's navigation and
    /// session) and it is not watched natively.
    pub(crate) fn watch_notice(&self, pane: PaneId) -> Option<WatchNotice> {
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
            WatchStatus::PollingOnly(error) => Some(WatchNotice {
                text: POLLING_ONLY,
                details: crate::error_presentation::describe(error),
            }),
            WatchStatus::Restarting => Some(WatchNotice {
                text: RESTARTING,
                details: String::new(),
            }),
        }
    }
}
