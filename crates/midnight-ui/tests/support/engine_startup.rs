//! Startup readiness shared by the real-Engine UI harnesses (support/tree_filters.rs
//! and support/operations_native.rs).
//!
//! Applying the saved preferences restores the saved tabs: each pane's startup tab
//! is replaced by a new tab whose location is listed again, and the replaced pane
//! loses its rows, cursor and selection without a message. Production keeps its
//! window hidden until the preferences are applied, so a harness must wait for the
//! restore before it clicks or presses keys. A restore whose navigation the bounded
//! command queue refused keeps the startup tab and is sent again with a later
//! event, so the harnesses compare tab identities instead of only checking that the
//! preferences were applied.
use midnight_core::{PaneId, TabId};
use midnight_ui::Presenter;

/// The active tab of each pane.
pub fn active_tabs(presenter: &Presenter) -> [Option<TabId>; 2] {
    [PaneId::Left, PaneId::Right].map(|pane| {
        presenter
            .tabs(pane)
            .into_iter()
            .find(|tab| tab.active)
            .map(|tab| tab.id)
    })
}

/// Whether the saved preferences are applied and their tab restore has replaced
/// the `startup` tab of both panes. The restored locations may still be loading.
pub fn tabs_restored(presenter: &Presenter, startup: &[Option<TabId>; 2]) -> bool {
    presenter.window_geometry_ready()
        && active_tabs(presenter)
            .iter()
            .zip(startup)
            .all(|(active, startup)| active != startup)
}
