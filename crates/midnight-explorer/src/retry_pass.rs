//! The retry pass: shutdown continuation, deferred flushes and the accepted-listing
//! acknowledgement. Bounded queue saturation cannot permanently lose any of them; the
//! pass runs while it reports work and stops after a quiet pass.
use crate::{pulse::Refusals, viewers::ImagePreparation, visible_sync::VisibleSync};
use midnight_core::{Command, PaneId};
use midnight_ui::{BeginOwned, MainWindow, Presenter};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::Duration,
};

pub(crate) const INTERVAL: Duration = Duration::from_millis(100);

pub(crate) struct RetryPass {
    pub(crate) presenter: Rc<RefCell<Presenter>>,
    pub(crate) controls: Rc<dyn Fn(Command) -> bool>,
    pub(crate) begin_owned: BeginOwned,
    pub(crate) window: slint::Weak<MainWindow>,
    /// Closes viewer and comparator windows; never borrows the presenter.
    pub(crate) close_viewers: Box<dyn Fn()>,
    pub(crate) preparation: Weak<ImagePreparation>,
    pub(crate) visible: Rc<RefCell<VisibleSync>>,
    pub(crate) refusals: Rc<Refusals>,
}

impl RetryPass {
    /// Returns whether work remains: live jobs (including a requested exit that waits
    /// for them), a hidden main window, a refused send, an unavailable owned scope, or a
    /// flush that sent something and may continue.
    pub(crate) fn run(&self) -> bool {
        // This pass retries everything refused so far.
        self.refusals.take();
        let live = {
            let mut state = self.presenter.borrow_mut();
            if let Some(preparation) = self.preparation.upgrade() {
                preparation.exit_requested(&state);
            }
            if state.continue_operation_shutdown(&*self.controls)
                && let Some(window) = self.window.upgrade()
            {
                state.shutdown_drag_drop(&*self.controls);
                (self.close_viewers)();
                // Every tool window family (Jobs, job progress, decisions and
                // the embedded forms too); the helper never borrows Presenter.
                midnight_ui::close_tool_windows(&window);
                let _ = window.hide();
            }
            state.has_live_operations()
        };
        let hidden = self
            .window
            .upgrade()
            .is_none_or(|window| !window.window().is_visible());
        if hidden {
            (self.close_viewers)();
        }
        let sent = Cell::new(false);
        let flushed =
            midnight_ui::with_owned_commands(&self.presenter, &self.begin_owned, |state, send| {
                let send = |command| {
                    let accepted = send(command);
                    sent.set(sent.get() || accepted);
                    accepted
                };
                state.flush_selection(&send);
                state.flush_filters(&send);
                state.flush_filename_colors(&send);
                state.flush_search(&send);
                state.flush_comparison(&send);
            })
            .is_some();
        let synced = sync_visible(&self.presenter, &self.begin_owned, &self.visible);
        live || hidden || sent.get() || self.refusals.take() || !flushed || !synced
    }
}

/// Send each pane's accepted listing once. Returns false when an owned scope was
/// unavailable, so a pane may still wait.
pub(crate) fn sync_visible(
    presenter: &Rc<RefCell<Presenter>>,
    begin_owned: &BeginOwned,
    sync: &RefCell<VisibleSync>,
) -> bool {
    for pane in [PaneId::Left, PaneId::Right] {
        if midnight_ui::with_owned_commands(presenter, begin_owned, |p, send| {
            if let Some(visible) = p.visible_directory(pane) {
                sync.borrow_mut().send(visible, send);
            }
        })
        .is_none()
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
#[path = "retry_pass_tests.rs"]
mod tests;
