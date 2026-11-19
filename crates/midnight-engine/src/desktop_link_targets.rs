//! Bounded advisory metadata reads for committed native directory-link rows.
use super::*;
use crate::Snapshots;
use std::path::Component;

#[derive(Clone)]
pub(super) struct Lookup {
    pub(super) request: DesktopRequestId,
    pub(super) pane: PaneId,
    pub(super) navigation: NavigationId,
    pub(super) path: PathBuf,
    pub(super) snapshots: Snapshots,
}

pub(super) fn current(
    snapshots: &Snapshots,
    pane: PaneId,
    navigation: NavigationId,
    path: &Path,
) -> bool {
    let state = snapshots.lock().unwrap_or_else(|error| error.into_inner());
    let pane = &state[pane.index()];
    if pane.pending.is_some() || !pane.has_native_view() {
        return false;
    }
    let Some(visible) = pane
        .visible
        .as_ref()
        .filter(|view| view.navigation == navigation)
    else {
        return false;
    };
    let Ok(relative) = path.strip_prefix(&visible.path) else {
        return false;
    };
    let mut components = relative.components();
    matches!(components.next(), Some(Component::Normal(_)))
        && components.all(|component| matches!(component, Component::Normal(_)))
}

/// A single worker reads at most one target at a time. The watch retains only
/// the latest pending hover while a native metadata read is in flight.
pub(super) async fn run(
    scope: TaskLifetime,
    services: Arc<dyn DesktopServices>,
    outbox: Outbox,
    mut lookups: watch::Receiver<Option<Lookup>>,
) {
    while lookups.changed().await.is_ok() {
        let latest = lookups.borrow_and_update().clone();
        let Some(lookup) = latest else {
            continue;
        };
        if !current(
            &lookup.snapshots,
            lookup.pane,
            lookup.navigation,
            &lookup.path,
        ) {
            continue;
        }
        let services = services.clone();
        let path = lookup.path.clone();
        let target = tokio::task::spawn_blocking(
            scope.blocking(move || services.read_link_target(&path).ok().flatten()),
        )
        .await
        .ok()
        .flatten();
        if lookups.has_changed().unwrap_or(true)
            || !current(
                &lookup.snapshots,
                lookup.pane,
                lookup.navigation,
                &lookup.path,
            )
        {
            continue;
        }
        outbox
            .send(Event::DirectoryLinkTargetRead {
                request: lookup.request,
                pane: lookup.pane,
                navigation: lookup.navigation,
                target,
            })
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommittedSnapshot, SnapshotState, test_snapshots};

    #[test]
    fn tooltip_requests_are_limited_to_the_current_native_pane_and_path() {
        let root = std::env::temp_dir().join("midnight-link-tooltip");
        let nav = NavigationId(17);
        let snapshots = test_snapshots([
            SnapshotState {
                visible: Some(CommittedSnapshot {
                    navigation: nav,
                    path: root.clone(),
                    entries: Arc::from([]),
                    sizes: Default::default(),
                }),
                ..Default::default()
            },
            SnapshotState::default(),
        ]);
        let candidate = root.join("expanded").join("junction");
        assert!(current(&snapshots, PaneId::Left, nav, &candidate));
        assert!(!current(&snapshots, PaneId::Right, nav, &candidate));
        assert!(!current(
            &snapshots,
            PaneId::Left,
            NavigationId(16),
            &candidate
        ));
        assert!(!current(&snapshots, PaneId::Left, nav, &root));
        assert!(!current(
            &snapshots,
            PaneId::Left,
            nav,
            &root.join("../outside")
        ));
        snapshots.lock().unwrap()[0].archive_navigation = Some(nav);
        assert!(!current(&snapshots, PaneId::Left, nav, &candidate));
        snapshots.lock().unwrap()[0].archive_navigation = None;
        snapshots.lock().unwrap()[0].pending = Some(NavigationId(18));
        assert!(!current(&snapshots, PaneId::Left, nav, &candidate));
    }
}
