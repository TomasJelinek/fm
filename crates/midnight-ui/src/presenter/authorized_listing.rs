//! Rows that the authorization helper read (**Retry with authorization**).
//!
//! This process may not read their folder, so the pane keeps them as listed:
//! the backend neither watches nor checks the folder, and nothing re-reads it
//! on its own (a watch hint, Show hidden files, a finished operation, a removed
//! drive or a dropped tree), because each re-read with ordinary rights would be
//! denied and its recovery would move the pane to a parent folder. A re-sort
//! keeps the rows. Refresh asks with ordinary rights first and, while access is
//! still denied, offers authorization again as opening the folder did; only
//! that explicit retry prompts. A denial never moves the pane elsewhere.
use super::*;

/// The pane's status row while it shows such rows.
const AUTHORIZED: &str =
    "Listed with authorization; not updated automatically. Refresh offers authorization again.";
const AUTHORIZED_DETAILS: &str = "The authorization helper listed this folder once. Your \
    account cannot read it, so Midnight Explorer neither watches it for changes nor lists it \
    again on its own. Refresh lists it with your account's rights first and, while access is \
    still denied, offers Retry with authorization.";

impl Pane {
    /// The pane shows the committed rows of an authorized listing.
    pub(crate) fn shows_authorized_rows(&self) -> bool {
        self.native_capabilities()
            && self.successful_generation.is_some()
            && self.authorized_rows == self.successful_generation
    }
}

impl Presenter {
    /// The status-row note of a pane that shows rows the authorization helper
    /// read (flat rows; a tree reports its own status).
    pub(super) fn authorized_notice(&self, pane: PaneId) -> Option<watch_status::WatchNotice> {
        let p = &self.panes[pane.index()];
        (p.shows_authorized_rows() && p.tree_view().is_none()).then(|| watch_status::WatchNotice {
            text: AUTHORIZED,
            details: AUTHORIZED_DETAILS.to_owned(),
        })
    }

    /// A denied re-read of rows the authorization helper read is reported in
    /// place: it never moves the pane to the parent folder that the backend's
    /// recovery suggests (`apply_navigation_fallback`).
    pub(super) fn keeps_authorized_rows(&self, pane: PaneId, error: &FsError) -> bool {
        let p = &self.panes[pane.index()];
        error.kind == std::io::ErrorKind::PermissionDenied
            && p.shows_authorized_rows()
            && p.pending
                .as_ref()
                .is_some_and(|request| request.keep_rows && request.path == p.path)
    }
}
