//! Cheap root discovery and bounded background metadata have independent workers.

use crate::Outbox;
use midnight_core::{Event, FileSystem, FsError, IconKey, LocaleService, Volume, VolumeDetails};
use std::{
    collections::{HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::{Semaphore, watch},
    task::JoinSet,
    time::{Instant, MissedTickBehavior},
};

const REFRESH: Duration = Duration::from_secs(45);
const MAX_VOLUME_ICONS: usize = 256;

pub(crate) struct Controller {
    requests: watch::Sender<u64>,
}

impl Controller {
    pub(crate) fn new(
        filesystem: Arc<dyn FileSystem>,
        locale: Option<Arc<dyn LocaleService>>,
        outbox: Outbox,
        icons: watch::Sender<Vec<IconKey>>,
        context: watch::Sender<u64>,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let (requests, receiver) = watch::channel(0);
        tasks.spawn(run(filesystem, locale, outbox, icons, context, receiver));
        Self { requests }
    }

    pub(crate) fn refresh(&self) {
        // A watch slot coalesces arbitrarily many refresh commands without
        // allocating tasks or dropping ownership of uninterruptible OS calls.
        self.requests
            .send_modify(|version| *version = version.wrapping_add(1));
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    id: String,
    root: PathBuf,
}

impl From<&Volume> for Key {
    fn from(volume: &Volume) -> Self {
        Self {
            id: volume.id.clone(),
            root: volume.root.clone(),
        }
    }
}

struct Cached {
    at: Instant,
    details: VolumeDetails,
}

struct Completed {
    key: Key,
    incarnation: u64,
    details: VolumeDetails,
}

#[derive(Default)]
struct Details {
    active: HashMap<Key, u64>,
    cached: HashMap<Key, Cached>,
    in_flight: HashSet<Key>,
    incarnation: u64,
}

impl Details {
    fn complete(&mut self, completed: Completed) -> Option<Event> {
        self.in_flight.remove(&completed.key);
        if self.active.get(&completed.key) != Some(&completed.incarnation) {
            return None;
        }
        self.cached.insert(
            completed.key.clone(),
            Cached {
                at: Instant::now(),
                details: completed.details.clone(),
            },
        );
        Some(event(completed.key, completed.details))
    }

    fn next_refresh(&self) -> Option<Instant> {
        self.cached
            .iter()
            .filter(|(key, _)| !self.in_flight.contains(*key))
            .map(|(_, cached)| cached.at + REFRESH)
            .min()
    }

    fn discover(&mut self, volumes: &[Volume]) -> bool {
        let keys: HashSet<_> = volumes.iter().map(Key::from).collect();
        let changed = keys.len() != self.active.len()
            || keys.iter().any(|key| !self.active.contains_key(key));
        self.active.retain(|key, _| keys.contains(key));
        self.cached.retain(|key, _| keys.contains(key));
        for key in keys {
            self.active.entry(key).or_insert_with(|| {
                self.incarnation = self.incarnation.wrapping_add(1);
                self.incarnation
            });
        }
        changed
    }

    fn schedule(
        &mut self,
        filesystem: &Arc<dyn FileSystem>,
        budget: &Arc<Semaphore>,
        workers: &mut JoinSet<Completed>,
    ) {
        for (key, incarnation) in &self.active {
            if self.in_flight.contains(key)
                || self
                    .cached
                    .get(key)
                    .is_some_and(|cached| cached.at.elapsed() < REFRESH)
            {
                continue;
            }
            let Ok(permit) = budget.clone().try_acquire_owned() else {
                break;
            };
            let filesystem = filesystem.clone();
            let key = key.clone();
            let incarnation = *incarnation;
            self.in_flight.insert(key.clone());
            workers.spawn(async move {
                let path = key.root.clone();
                let fallback = path.clone();
                let details = tokio::task::spawn_blocking(move || {
                    // The real blocking call owns the permit, even on shutdown.
                    let _permit = permit;
                    let label = filesystem.volume_label(&path);
                    let space = filesystem.space_info(&path);
                    VolumeDetails { label, space }
                })
                .await
                .unwrap_or_else(|_| {
                    let error = worker_error("volume details worker", &fallback);
                    VolumeDetails {
                        label: Err(error.clone()),
                        space: Err(error),
                    }
                });
                Completed {
                    key,
                    incarnation,
                    details,
                }
            });
        }
    }
}

async fn run(
    filesystem: Arc<dyn FileSystem>,
    locale: Option<Arc<dyn LocaleService>>,
    outbox: Outbox,
    icons: watch::Sender<Vec<IconKey>>,
    context: watch::Sender<u64>,
    mut requests: watch::Receiver<u64>,
) {
    let mut discovery = JoinSet::new();
    let mut workers = JoinSet::new();
    let budget = Arc::new(Semaphore::new(2));
    let mut details = Details::default();
    let mut pending = false;
    let mut enabled = false;
    let mut timer = tokio::time::interval_at(Instant::now() + REFRESH, REFRESH);
    timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        if pending && discovery.is_empty() {
            pending = false;
            let version = *requests.borrow_and_update();
            let filesystem = filesystem.clone();
            let locale = locale.clone();
            discovery.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    filesystem.list_volumes().and_then(|volumes| {
                        crate::navigation_ordering::volumes(volumes, locale.as_deref())
                    })
                })
                .await
                .unwrap_or_else(|_| Err(worker_error("volume discovery worker", Path::new(""))));
                (version, result)
            });
        }
        details.schedule(&filesystem, &budget, &mut workers);
        let next_refresh = if budget.available_permits() > 0 {
            details.next_refresh()
        } else {
            None
        };
        tokio::select! {
            changed = requests.changed() => {
                if changed.is_err() { break; }
                enabled = true;
                pending = true;
            }
            _ = timer.tick(), if enabled => {
                // Discovery also notices removable media; it never waits on details.
                // A slow root has exactly one in-flight details call across ticks.
                pending = true;
            }
            _ = tokio::time::sleep_until(next_refresh.unwrap_or_else(|| Instant::now() + REFRESH)),
                if next_refresh.is_some() => {}
            result = discovery.join_next(), if !discovery.is_empty() => {
                let Some(Ok((version, result))) = result else { continue; };
                if !publish_discovery(version, result, &requests, &outbox, &icons, &context, &mut details).await {
                    break;
                }
            }
            result = workers.join_next(), if !workers.is_empty() => {
                let Some(Ok(completed)) = result else { continue; };
                if let Some(event) = details.complete(completed)
                    && !outbox.send(event).await { break; }
            }
        }
    }
}

async fn publish_discovery(
    version: u64,
    result: Result<Vec<Volume>, FsError>,
    requests: &watch::Receiver<u64>,
    outbox: &Outbox,
    icons: &watch::Sender<Vec<IconKey>>,
    context: &watch::Sender<u64>,
    details: &mut Details,
) -> bool {
    if version != *requests.borrow() {
        return true;
    }
    // Reserve before the final version check: a full UI queue must not publish
    // discovery superseded while waiting. Hold the watch read guard through send
    // to serialize with refresh(), without holding it across an await.
    let Ok(permit) = outbox.sender.reserve().await else {
        return false;
    };
    let discovered = {
        let latest = requests.borrow();
        if version != *latest {
            return true;
        }
        match result {
            Ok(volumes) => {
                let changed = details.discover(&volumes);
                let mut seen = HashSet::new();
                let roots = volumes
                    .iter()
                    .map(|volume| IconKey::for_drive(&volume.root))
                    .filter(|key| seen.insert(key.clone()))
                    .take(MAX_VOLUME_ICONS)
                    .collect();
                if changed {
                    // The context worker holds the matching read guard while
                    // publishing. Serialize its invalidation with this event so
                    // an older context cannot slip behind the new inventory.
                    context.send_modify(|version| {
                        *version = version.wrapping_add(1);
                        permit.send(Event::VolumesUpdated(volumes));
                    });
                } else {
                    permit.send(Event::VolumesUpdated(volumes));
                }
                // Only a published, current inventory reaches the icon actor. Raw
                // roots identify cache entries, so reusing a label/id cannot retarget one.
                icons.send_replace(roots);
                true
            }
            Err(error) => {
                permit.send(Event::VolumesFailed(error));
                false
            }
        }
    };
    (outbox.wakeup)();
    if discovered {
        for (key, cached) in &details.cached {
            if !outbox
                .send(event(key.clone(), cached.details.clone()))
                .await
            {
                return false;
            }
        }
    }
    true
}

fn event(key: Key, details: VolumeDetails) -> Event {
    Event::VolumeDetailsUpdated {
        volume_id: key.id,
        root: key.root,
        details,
    }
}

fn worker_error(operation: &'static str, path: &Path) -> FsError {
    FsError::from_io(
        operation,
        path,
        io::Error::other("The volume worker failed."),
    )
}
