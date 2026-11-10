//! One latest summary request per pane; native cache updates replace one contribution.
use crate::{Outbox, SharedSizes, Snapshots, folder_size_totals::Aggregate};
use midnight_core::{Entry, Event, PaneId, SelectionSizeId, SelectionSizeRequest};
use std::{
    path::Path,
    sync::{Arc, Mutex, Weak},
};
use tokio::{sync::watch, task::JoinSet};

mod publication;

type WeakSizes =
    Weak<Mutex<std::collections::HashMap<std::path::PathBuf, midnight_core::FolderSize>>>;

struct Scope {
    request: Arc<SelectionSizeRequest>,
    cache: Option<WeakSizes>,
    aggregate: Aggregate,
}

#[derive(Default)]
struct State {
    wanted: SelectionSizeId,
    version: u64,
    scope: Option<Scope>,
}

#[derive(Clone, Default)]
pub(crate) struct Observer {
    states: Arc<[Mutex<State>; 2]>,
    publication: publication::Publication,
}

impl Observer {
    pub(crate) fn publish(&self, event: Event) {
        self.publication.submit(event);
    }

    fn want(&self, request: &SelectionSizeRequest) -> bool {
        let mut state = self.states[request.pane.index()]
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if request.id <= state.wanted {
            return false;
        }
        state.wanted = request.id;
        true
    }

    fn seed(&self, request: Arc<SelectionSizeRequest>, snapshots: &Snapshots) -> Option<Event> {
        loop {
            let snapshot = snapshot_for(
                &snapshots.lock().unwrap_or_else(|e| e.into_inner()),
                &request,
            );
            let cache = snapshot.as_ref().map(|snapshot| snapshot.sizes.clone());
            let sizes = cache
                .as_ref()
                .map(|cache| cache.lock().unwrap_or_else(|e| e.into_inner()));
            let empty = Default::default();
            let aggregate = Aggregate::new(
                request.selection.clone(),
                snapshot
                    .as_ref()
                    .map_or(&[], |snapshot| snapshot.entries.as_ref()),
                sizes.as_deref().unwrap_or(&empty),
            );
            // Hold snapshot identity through publication. A scan replacing the
            // cache either precedes this check (retry), or observes the published
            // old scope and replaces it afterwards. No replacement can be lost.
            let current_snapshots = snapshots.lock().unwrap_or_else(|e| e.into_inner());
            let current = snapshot_for(&current_snapshots, &request);
            let same = match (&cache, &current) {
                (Some(cache), Some(current)) => Arc::ptr_eq(cache, &current.sizes),
                (None, None) => true,
                _ => false,
            };
            let mut state = self.states[request.pane.index()]
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if request.id != state.wanted {
                return None;
            }
            if !same {
                continue;
            }
            state.scope = Some(Scope {
                request,
                cache: cache.as_ref().map(Arc::downgrade),
                aggregate,
            });
            return event(&mut state);
        }
    }

    /// Read the current cache under the same lock order as seeding. A delayed
    /// notification therefore cannot overwrite a newer cached contribution.
    pub(crate) fn updated(&self, pane: PaneId, cache: &SharedSizes, path: &Path) -> Option<Event> {
        let sizes = cache.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.states[pane.index()]
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let wanted = state.wanted;
        let scope = state.scope.as_mut()?;
        if scope.request.id != wanted
            || !same_cache(scope, cache)
            || !scope.aggregate.update(path, sizes.get(path))
        {
            return None;
        }
        event(&mut state)
    }

    pub(crate) fn replaced(
        &self,
        pane: PaneId,
        old: &SharedSizes,
        new: &SharedSizes,
        entries: &[Entry],
    ) -> Option<Event> {
        let sizes = new.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.states[pane.index()]
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let wanted = state.wanted;
        let scope = state.scope.as_mut()?;
        if scope.request.id != wanted || !same_cache(scope, old) {
            return None;
        }
        scope.aggregate = Aggregate::new(scope.request.selection.clone(), entries, &sizes);
        scope.cache = Some(Arc::downgrade(new));
        event(&mut state)
    }
}

fn snapshot_for(
    snapshots: &[crate::SnapshotState; 2],
    request: &SelectionSizeRequest,
) -> Option<crate::CommittedSnapshot> {
    let state = &snapshots[request.pane.index()];
    if !state.has_native_view() {
        return None;
    }
    [&state.visible, &state.committed]
        .into_iter()
        .flatten()
        .find(|snapshot| {
            snapshot.navigation == request.source_navigation && snapshot.path == request.location
        })
        .cloned()
}

fn same_cache(scope: &Scope, cache: &SharedSizes) -> bool {
    scope
        .cache
        .as_ref()
        .is_some_and(|old| old.ptr_eq(&Arc::downgrade(cache)))
}

fn event(state: &mut State) -> Option<Event> {
    let scope = state.scope.as_ref()?;
    state.version = state.version.checked_add(1)?;
    Some(Event::SelectedFolderSizes {
        request: scope.request.id,
        pane: scope.request.pane,
        navigation: scope.request.navigation,
        selection_revision: scope.request.revision,
        version: state.version,
        summary: scope.aggregate.summary(),
    })
}

pub(crate) struct Controller {
    pub(crate) observer: Observer,
    requests: [watch::Sender<Option<Arc<SelectionSizeRequest>>>; 2],
}

impl Controller {
    pub(crate) fn new(outbox: Outbox, snapshots: Snapshots, tasks: &mut JoinSet<()>) -> Self {
        let observer = Observer::default();
        observer.publication.start(outbox, tasks);
        let requests = std::array::from_fn(|_| {
            let (sender, mut receiver) = watch::channel::<Option<Arc<SelectionSizeRequest>>>(None);
            let observer = observer.clone();
            let snapshots = snapshots.clone();
            tasks.spawn(async move {
                while receiver.changed().await.is_ok() {
                    let Some(request) = receiver.borrow_and_update().clone() else {
                        continue;
                    };
                    let worker = observer.clone();
                    let snapshots = snapshots.clone();
                    if let Ok(Some(event)) =
                        tokio::task::spawn_blocking(move || worker.seed(request, &snapshots)).await
                    {
                        observer.publish(event);
                    }
                }
            });
            sender
        });
        Self { observer, requests }
    }

    pub(crate) fn submit(&self, request: SelectionSizeRequest) {
        if self.observer.want(&request) {
            self.requests[request.pane.index()].send_replace(Some(Arc::new(request)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommittedSnapshot, SnapshotState};
    use midnight_core::{
        DirectoryIndex, EntryId, EntryKind, FolderSize, NavigationId, SelectionRevision,
        SelectionSnapshot,
    };
    use std::collections::HashMap;

    fn fixture() -> (Observer, Snapshots, SharedSizes, Arc<SelectionSizeRequest>) {
        let entries: Arc<[Entry]> = vec![Entry::new(
            EntryId(7),
            "active/資料".into(),
            EntryKind::Directory,
        )]
        .into();
        let selection = Arc::new(
            SelectionSnapshot::from_bits(Arc::new(DirectoryIndex::from_entries(&entries)), vec![1])
                .unwrap(),
        );
        let cache = Arc::new(Mutex::new(HashMap::from([(
            Path::new("active/資料").to_owned(),
            FolderSize {
                bytes: 7,
                finished: true,
                ..Default::default()
            },
        )])));
        let snapshot = CommittedSnapshot {
            navigation: NavigationId(1),
            path: "active".into(),
            entries,
            sizes: cache.clone(),
        };
        let snapshots = Arc::new(Mutex::new([
            SnapshotState {
                visible: Some(snapshot),
                ..Default::default()
            },
            SnapshotState::default(),
        ]));
        let request = Arc::new(SelectionSizeRequest {
            id: SelectionSizeId(1),
            pane: PaneId::Left,
            navigation: NavigationId(1),
            revision: SelectionRevision(3),
            source_navigation: NavigationId(1),
            location: "active".into(),
            selection,
        });
        (Observer::default(), snapshots, cache, request)
    }

    fn summary(event: Event) -> (u64, midnight_core::SelectedFolderSizes) {
        let Event::SelectedFolderSizes {
            version, summary, ..
        } = event
        else {
            panic!("expected summary");
        };
        (version, summary)
    }

    #[test]
    fn failed_navigation_uses_the_restored_source_cache_with_the_current_ui_request_identity() {
        let (observer, snapshots, _, request) = fixture();
        let mut request = (*request).clone();
        request.navigation = midnight_core::NavigationId(2);
        assert!(observer.want(&request));
        let event = observer.seed(Arc::new(request), &snapshots).unwrap();
        assert!(matches!(
            &event,
            Event::SelectedFolderSizes {
                navigation: midnight_core::NavigationId(2),
                ..
            }
        ));
        assert_eq!(summary(event).1.complete_bytes, Some(7));
    }

    #[test]
    fn virtual_panes_never_expose_an_old_native_cache_as_their_selection_total() {
        for unavailable in [false, true] {
            let (observer, snapshots, _, request) = fixture();
            {
                let mut snapshots = snapshots.lock().unwrap();
                if unavailable {
                    snapshots[0].archive_navigation = Some(NavigationId(2));
                } else {
                    snapshots[0].archive = Some((NavigationId(2), Arc::from([])));
                }
            }
            assert!(observer.want(&request));
            assert_eq!(
                summary(observer.seed(request, &snapshots).unwrap())
                    .1
                    .complete,
                0
            );
        }
    }

    #[test]
    fn replacement_cache_rejects_old_workers_and_retains_monotonic_summary_versions() {
        let (observer, snapshots, old, request) = fixture();
        assert!(observer.want(&request));
        let first = summary(observer.seed(request, &snapshots).unwrap());
        assert_eq!(first.1.complete_bytes, Some(7));
        let new = Arc::new(Mutex::new(HashMap::new()));
        let entries = snapshots.lock().unwrap()[0]
            .visible
            .as_ref()
            .unwrap()
            .entries
            .clone();
        let reset = summary(
            observer
                .replaced(PaneId::Left, &old, &new, &entries)
                .unwrap(),
        );
        assert!(reset.0 > first.0);
        assert_eq!(reset.1.complete, 0);
        old.lock()
            .unwrap()
            .get_mut(Path::new("active/資料"))
            .unwrap()
            .bytes = 999;
        assert!(
            observer
                .updated(PaneId::Left, &old, Path::new("active/資料"))
                .is_none()
        );
        new.lock().unwrap().insert(
            "active/資料".into(),
            FolderSize {
                bytes: 11,
                finished: true,
                ..Default::default()
            },
        );
        let changed = summary(
            observer
                .updated(PaneId::Left, &new, Path::new("active/資料"))
                .unwrap(),
        );
        assert!(changed.0 > reset.0);
        assert_eq!(changed.1.complete_bytes, Some(11));
        assert!(
            observer
                .updated(PaneId::Left, &new, Path::new("active/資料"))
                .is_none()
        );
    }

    #[test]
    fn superseded_seed_cannot_replace_latest_selection_and_delayed_notification_reads_current_value()
     {
        let (observer, snapshots, cache, first) = fixture();
        assert!(observer.want(&first));
        let mut next = (*first).clone();
        next.id = SelectionSizeId(2);
        next.revision = SelectionRevision(4);
        let next = Arc::new(next);
        assert!(observer.want(&next));
        assert!(observer.seed(first, &snapshots).is_none());
        observer.seed(next, &snapshots).unwrap();
        cache
            .lock()
            .unwrap()
            .get_mut(Path::new("active/資料"))
            .unwrap()
            .bytes = 19;
        // The observer reads the cache, so an earlier 8-byte notification cannot
        // regress a 19-byte value already stored by a later worker callback.
        let latest = summary(
            observer
                .updated(PaneId::Left, &cache, Path::new("active/資料"))
                .unwrap(),
        );
        assert_eq!(latest.1.complete_bytes, Some(19));
    }

    #[test]
    fn cache_replacement_during_first_seed_cannot_strand_the_observer_on_old_values() {
        let (observer, snapshots, old, request) = fixture();
        assert!(observer.want(&request));
        let old_lock = old.lock().unwrap();
        let before = Arc::strong_count(&old);
        let worker_observer = observer.clone();
        let worker_snapshots = snapshots.clone();
        let worker =
            std::thread::spawn(move || worker_observer.seed(request, &worker_snapshots).unwrap());
        // The two worker clones establish that it captured the old snapshot and
        // is waiting on its cache lock, before the first observer scope exists.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while Arc::strong_count(&old) < before + 2 {
            assert!(
                std::time::Instant::now() < deadline,
                "summary worker did not capture cache"
            );
            std::thread::yield_now();
        }
        let new = Arc::new(Mutex::new(HashMap::from([(
            Path::new("active/資料").to_owned(),
            FolderSize {
                bytes: 23,
                finished: true,
                ..Default::default()
            },
        )])));
        let entries = {
            let mut snapshots = snapshots.lock().unwrap();
            let snapshot = snapshots[0].visible.as_mut().unwrap();
            snapshot.sizes = new.clone();
            snapshot.entries.clone()
        };
        assert!(
            observer
                .replaced(PaneId::Left, &old, &new, &entries)
                .is_none()
        );
        drop(old_lock);
        assert_eq!(summary(worker.join().unwrap()).1.complete_bytes, Some(23));
        new.lock()
            .unwrap()
            .get_mut(Path::new("active/資料"))
            .unwrap()
            .bytes = 27;
        assert_eq!(
            summary(
                observer
                    .updated(PaneId::Left, &new, Path::new("active/資料"))
                    .unwrap()
            )
            .1
            .complete_bytes,
            Some(27)
        );
    }
}
