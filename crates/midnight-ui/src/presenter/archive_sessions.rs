//! The sessions of archives opened from native files that tabs and viewers hold, and the
//! session that a tab let go of, which its next such archive may replace
//! (`Command::ArchiveNavigate::replaces`).
use super::*;
use std::rc::Rc;

#[derive(Default)]
pub(crate) struct State {
    /// The archive sessions (`ArchiveMount::id`) that open viewers hold, each with every
    /// session that encloses it.
    viewers: Option<Rc<dyn Fn() -> Vec<u64>>>,
    /// Per pane: the session of an archive opened from a native file that its tab let go
    /// of last.
    released: [Option<u64>; 2],
    /// The session that the next such archive replaces whoever else showed it: the one
    /// that the user closed to open it (`archive_prompt`).
    closed: Option<u64>,
}

impl State {
    /// `pane`'s tab let go of the archive session `session`.
    pub(super) fn released(&mut self, pane: PaneId, session: u64) {
        self.released[pane.index()] = Some(session);
    }
    /// The next archive opened from a native file replaces `session`, which the user
    /// closed for it.
    pub(super) fn replace_next(&mut self, session: u64) {
        self.closed = Some(session);
    }
}

impl Presenter {
    /// Where the sessions that open viewers hold are read (`ViewerWindows`).
    pub fn set_viewer_archives(&mut self, viewers: Rc<dyn Fn() -> Vec<u64>>) {
        self.archive_sessions.viewers = Some(viewers);
    }

    /// The session that the next archive that `pane` opens from a native file replaces:
    /// the one the user just closed for it (`replace_next`), else the one its tab holds,
    /// or else the one it let go of last, unless the other panel, a tab or a viewer holds
    /// it too. Asking consumes the session let go of and the closed one.
    pub(super) fn replaceable_session(&mut self, pane: PaneId) -> Option<u64> {
        let released = self.archive_sessions.released[pane.index()].take();
        if let Some(closed) = self.archive_sessions.closed.take() {
            return Some(closed);
        }
        self.panes[pane.index()]
            .archive
            .as_ref()
            .and_then(archives::ArchivePane::native_session)
            .or(released)
            .filter(|session| !self.archive_session_shared(pane, *session))
    }

    /// The sessions that open viewers hold, one entry per viewer and session.
    pub(super) fn viewer_archive_sessions(&self) -> Vec<u64> {
        self.archive_sessions
            .viewers
            .as_ref()
            .map(|viewers| viewers())
            .unwrap_or_default()
    }

    fn archive_session_shared(&self, pane: PaneId, session: u64) -> bool {
        self.panes[pane.other().index()]
            .archive
            .as_ref()
            .is_some_and(|archive| archive.holds_session(session))
            || self.parked_tabs_hold(session)
            || self
                .archive_sessions
                .viewers
                .as_ref()
                .is_some_and(|viewers| viewers().contains(&session))
    }
}
