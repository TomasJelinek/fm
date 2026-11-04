//! Isolate each scan's writes from cancelled workers and unrelated snapshots.
use super::*;

pub(super) fn prepare(
    mut selected: CommittedSnapshot,
    snapshots: &Snapshots,
    pane: PaneId,
) -> CommittedSnapshot {
    let sizes = Arc::new(Mutex::new(HashMap::new()));
    let mut snapshots = snapshots.lock().unwrap_or_else(|error| error.into_inner());
    let state = &mut snapshots[pane.index()];
    for cached in [&mut state.visible, &mut state.committed]
        .into_iter()
        .flatten()
    {
        if cached.navigation == selected.navigation
            && cached.path == selected.path
            && Arc::ptr_eq(&cached.entries, &selected.entries)
        {
            cached.sizes = sizes.clone();
        }
    }
    selected.sizes = sizes;
    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(navigation: u64) -> CommittedSnapshot {
        CommittedSnapshot {
            navigation: NavigationId(navigation),
            path: "active".into(),
            entries: vec![Entry::new(
                fm_core::EntryId(1),
                "active/folder".into(),
                EntryKind::Directory,
            )]
            .into(),
            sizes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[test]
    fn cancelled_worker_cache_cannot_write_into_a_replacement_scan() {
        let original = snapshot(1);
        let old_worker_cache = original.sizes.clone();
        let snapshots = Arc::new(Mutex::new([
            SnapshotState {
                visible: Some(original.clone()),
                committed: Some(original.clone()),
                pending: None,
            },
            SnapshotState::default(),
        ]));
        let next = prepare(original, &snapshots, PaneId::Left);
        assert!(!Arc::ptr_eq(&old_worker_cache, &next.sizes));
        old_worker_cache.lock().unwrap().insert(
            "active/folder".into(),
            FolderSize {
                bytes: 999,
                finished: true,
                ..Default::default()
            },
        );
        assert!(next.sizes.lock().unwrap().is_empty());
        let cached = snapshots.lock().unwrap();
        assert!(Arc::ptr_eq(
            &cached[0].visible.as_ref().unwrap().sizes,
            &next.sizes
        ));
        assert!(Arc::ptr_eq(
            &cached[0].committed.as_ref().unwrap().sizes,
            &next.sizes
        ));
    }

    #[test]
    fn only_the_exact_selected_snapshot_gets_the_new_cache() {
        let original = snapshot(1);
        // Even an identical navigation/path must not retarget a different Arc.
        let unrelated = snapshot(1);
        let unrelated_cache = unrelated.sizes.clone();
        let snapshots = Arc::new(Mutex::new([
            SnapshotState {
                visible: Some(original.clone()),
                committed: Some(unrelated),
                pending: None,
            },
            SnapshotState::default(),
        ]));
        let next = prepare(original, &snapshots, PaneId::Left);
        let cached = snapshots.lock().unwrap();
        assert!(Arc::ptr_eq(
            &cached[0].visible.as_ref().unwrap().sizes,
            &next.sizes
        ));
        assert!(Arc::ptr_eq(
            &cached[0].committed.as_ref().unwrap().sizes,
            &unrelated_cache
        ));
    }
}
