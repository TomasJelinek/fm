//! Denied destinations remain backend authority; UI receipts carry only navigation identities.
use midnight_core::{NavigationId, PaneId, SortSpec};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
pub(super) struct DeniedNavigations(Arc<Mutex<[State; 2]>>);
#[derive(Default)]
struct State {
    current: Option<NavigationId>,
    denied: Option<(NavigationId, PathBuf, SortSpec)>,
}
impl DeniedNavigations {
    pub(super) fn begin(&self, pane: PaneId, navigation: NavigationId) {
        let mut states = self.0.lock().unwrap_or_else(|error| error.into_inner());
        states[pane.index()] = State {
            current: Some(navigation),
            denied: None,
        };
    }
    /// Whether `navigation` is the pane's current one. Only then, and only when
    /// `retain` (an authorized retry can ask for permission), it keeps the denied
    /// destination for one retry.
    pub(super) fn record(
        &self,
        pane: PaneId,
        navigation: NavigationId,
        path: &std::path::Path,
        sort: SortSpec,
        retain: bool,
    ) -> bool {
        let mut states = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let state = &mut states[pane.index()];
        if state.current != Some(navigation) {
            return false;
        }
        if retain {
            state.denied = Some((navigation, path.to_owned(), sort));
        }
        true
    }
    pub(super) fn consume(
        &self,
        pane: PaneId,
        denied: NavigationId,
        next: NavigationId,
    ) -> Option<(PathBuf, SortSpec)> {
        let mut states = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let state = &mut states[pane.index()];
        if state.current != Some(denied)
            || next <= denied
            || state
                .denied
                .as_ref()
                .is_none_or(|record| record.0 != denied)
        {
            return None;
        }
        let (_, path, sort) = state.denied.take()?;
        Some((path, sort))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipts_are_single_use_and_bound_to_current_pane_generation() {
        let denied = DeniedNavigations::default();
        let pane = PaneId::Left;
        denied.begin(pane, NavigationId(3));
        assert!(denied.record(
            pane,
            NavigationId(3),
            std::path::Path::new("denied"),
            SortSpec::default(),
            true
        ));
        assert!(
            denied
                .consume(PaneId::Right, NavigationId(3), NavigationId(4))
                .is_none()
        );
        assert!(
            denied
                .consume(pane, NavigationId(3), NavigationId(3))
                .is_none()
        );
        assert!(
            denied
                .consume(pane, NavigationId(3), NavigationId(4))
                .is_some()
        );
        assert!(
            denied
                .consume(pane, NavigationId(3), NavigationId(5))
                .is_none()
        );
        denied.begin(pane, NavigationId(6));
        assert!(!denied.record(
            pane,
            NavigationId(3),
            std::path::Path::new("stale"),
            SortSpec::default(),
            true
        ));
        // A destination whose retry cannot ask for permission is current but not kept.
        assert!(denied.record(
            pane,
            NavigationId(6),
            std::path::Path::new("unavailable"),
            SortSpec::default(),
            false
        ));
        assert!(
            denied
                .consume(pane, NavigationId(6), NavigationId(7))
                .is_none()
        );
    }
}
