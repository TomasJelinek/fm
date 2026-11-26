//! Cheap root discovery and bounded background metadata have independent workers.
//!
//! Discovery runs on an explicit request (`Command::ListVolumes`), on a native
//! drive-change hint ([`WatchService::watch_volumes`]) and every [`REFRESH`] as a
//! safety net. A hint or the timer publishes a list only when it differs from the
//! last one published; an explicit request always publishes. New volumes are
//! published before their labels and free space are read, so a slow drive never
//! delays the drive list.

use crate::{Outbox, task_lifetime::TaskLifetime};
use midnight_core::{
    Event, FileSystem, FsError, IconKey, LocaleService, Volume, VolumeDetails, WatchService,
    WatchSubscription,
};
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

/// The native services discovery uses, besides the filesystem.
pub(crate) struct Services {
    pub(crate) locale: Option<Arc<dyn LocaleService>>,
    /// Drive-change hints; without it (or when registration fails) only the
    /// explicit requests and the periodic refresh discover volumes.
    pub(crate) notifications: Option<Arc<dyn WatchService>>,
}

impl Controller {
    pub(crate) fn new(
        scope: TaskLifetime,
        filesystem: Arc<dyn FileSystem>,
        services: Services,
        outbox: Outbox,
        icons: watch::Sender<Vec<IconKey>>,
        context: watch::Sender<u64>,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let (requests, receiver) = watch::channel(0);
        tasks.spawn(scope.future(run(
            scope.clone(),
            filesystem,
            services,
            Channels {
                outbox,
                icons,
                context,
            },
            receiver,
        )));
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

    /// A drive-change hint can mean that media arrived in a drive whose details
    /// failed (an empty card reader), so failed details are read again. Details
    /// that were read stay cached; a read in flight is not repeated.
    fn expire_failures(&mut self) {
        self.cached
            .retain(|_, cached| cached.details.label.is_ok() && cached.details.space.is_ok());
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
        scope: &TaskLifetime,
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
            let worker_scope = scope.clone();
            workers.spawn(scope.future(async move {
                let path = key.root.clone();
                let fallback = path.clone();
                let details = tokio::task::spawn_blocking(worker_scope.blocking(move || {
                    // The real blocking call owns the permit, even on shutdown.
                    let _permit = permit;
                    let label = filesystem.volume_label(&path);
                    let space = filesystem.space_info(&path);
                    VolumeDetails { label, space }
                }))
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
            }));
        }
    }
}

struct Channels {
    outbox: Outbox,
    icons: watch::Sender<Vec<IconKey>>,
    context: watch::Sender<u64>,
}

/// What a finished discovery answers.
struct Discovered {
    version: u64,
    /// An explicit request asked for it, so it is published even when unchanged.
    requested: bool,
    result: Result<Vec<Volume>, FsError>,
}

/// Whether two inventories show the same drives: identities, roots and names in order.
fn same_inventory(before: &[Volume], after: &[Volume]) -> bool {
    before.len() == after.len()
        && before.iter().zip(after).all(|(before, after)| {
            before.id == after.id && before.root == after.root && before.label == after.label
        })
}

/// What starts a discovery. An explicit request wins over a hint or the timer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Pending {
    #[default]
    None,
    Hint,
    Requested,
}

impl Pending {
    fn add(&mut self, other: Pending) {
        if other == Pending::Requested || *self == Pending::None {
            *self = other;
        }
    }
}

async fn run(
    scope: TaskLifetime,
    filesystem: Arc<dyn FileSystem>,
    services: Services,
    channels: Channels,
    mut requests: watch::Receiver<u64>,
) {
    let Services {
        locale,
        mut notifications,
    } = services;
    let mut discovery = JoinSet::new();
    let mut workers = JoinSet::new();
    let mut registering = JoinSet::new();
    let mut subscription: Option<Box<dyn WatchSubscription>> = None;
    // A native hint only marks this slot, from any thread; arbitrarily many
    // hints coalesce into at most one discovery in flight and one owed.
    let (hint_sender, mut hints) = watch::channel(());
    let hint_sender = Arc::new(hint_sender);
    let budget = Arc::new(Semaphore::new(2));
    let mut details = Details::default();
    let mut published: Option<Vec<Volume>> = None;
    let mut pending = Pending::None;
    let mut enabled = false;
    let mut timer = tokio::time::interval_at(Instant::now() + REFRESH, REFRESH);
    timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        if pending != Pending::None && discovery.is_empty() {
            let requested = pending == Pending::Requested;
            pending = Pending::None;
            let version = *requests.borrow_and_update();
            let filesystem = filesystem.clone();
            let locale = locale.clone();
            let worker_scope = scope.clone();
            discovery.spawn(scope.future(async move {
                let result = tokio::task::spawn_blocking(worker_scope.blocking(move || {
                    filesystem.list_volumes().and_then(|volumes| {
                        crate::navigation_ordering::volumes(volumes, locale.as_deref())
                    })
                }))
                .await
                .unwrap_or_else(|_| Err(worker_error("volume discovery worker", Path::new(""))));
                Discovered {
                    version,
                    requested,
                    result,
                }
            }));
        }
        details.schedule(&scope, &filesystem, &budget, &mut workers);
        let next_refresh = if budget.available_permits() > 0 {
            details.next_refresh()
        } else {
            None
        };
        tokio::select! {
            changed = requests.changed() => {
                if changed.is_err() { break; }
                enabled = true;
                pending.add(Pending::Requested);
                // Drives are watched from the first request on, so engines that
                // never show drives start no native watch.
                if let Some(service) = notifications.take() {
                    let sender = hint_sender.clone();
                    registering.spawn_blocking(scope.blocking(move || {
                        service.watch_volumes(Arc::new(move || sender.send_replace(())))
                    }));
                }
            }
            changed = hints.changed() => {
                if changed.is_ok() {
                    details.expire_failures();
                    pending.add(Pending::Hint);
                }
            }
            result = registering.join_next(), if !registering.is_empty() => {
                if let Some(Ok(Ok(registered))) = result {
                    subscription = Some(registered);
                    // A change between the first discovery and the registration
                    // sent no hint.
                    pending.add(Pending::Hint);
                }
            }
            _ = timer.tick(), if enabled => {
                // Discovery also notices removable media; it never waits on details.
                // A slow root has exactly one in-flight details call across ticks.
                pending.add(Pending::Hint);
            }
            _ = tokio::time::sleep_until(next_refresh.unwrap_or_else(|| Instant::now() + REFRESH)),
                if next_refresh.is_some() => {}
            result = discovery.join_next(), if !discovery.is_empty() => {
                let Some(Ok(discovered)) = result else { continue; };
                if !discovered.requested
                    && let (Ok(volumes), Some(last)) = (&discovered.result, &published)
                    && same_inventory(last, volumes)
                {
                    // Nothing changed: the UI already shows this list and its details.
                    continue;
                }
                if !publish_discovery(discovered, &requests, &channels, &mut details, &mut published).await {
                    break;
                }
            }
            result = workers.join_next(), if !workers.is_empty() => {
                let Some(Ok(completed)) = result else { continue; };
                if let Some(event) = details.complete(completed)
                    && !channels.outbox.send(event).await { break; }
            }
        }
    }
    // The native watch may join its thread; that belongs on a blocking worker.
    if let Some(subscription) = subscription {
        let _ = tokio::task::spawn_blocking(move || drop(subscription)).await;
    }
}

async fn publish_discovery(
    discovered: Discovered,
    requests: &watch::Receiver<u64>,
    channels: &Channels,
    details: &mut Details,
    published: &mut Option<Vec<Volume>>,
) -> bool {
    let Discovered {
        version, result, ..
    } = discovered;
    let Channels {
        outbox,
        icons,
        context,
    } = channels;
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
                *published = Some(volumes.clone());
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
                *published = None;
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

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::SpaceInfo;

    fn volume(id: &str, label: &str, root: &str) -> Volume {
        Volume {
            id: id.into(),
            label: label.into(),
            root: root.into(),
        }
    }

    #[test]
    fn inventories_differ_in_identity_root_name_order_or_length() {
        let list = [volume("c", "C:", "C:\\"), volume("e", "E:", "E:\\")];
        assert!(same_inventory(&list, &list.clone()));
        assert!(!same_inventory(&list, &list[..1]));
        assert!(!same_inventory(&list, &[list[1].clone(), list[0].clone()]));
        for changed in [
            volume("x", "E:", "E:\\"),
            volume("e", "USB", "E:\\"),
            volume("e", "E:", "F:\\"),
        ] {
            assert!(!same_inventory(&list, &[list[0].clone(), changed]));
        }
    }

    #[test]
    fn an_explicit_request_outranks_hints_and_is_never_downgraded() {
        let mut pending = Pending::None;
        pending.add(Pending::Hint);
        assert_eq!(pending, Pending::Hint);
        pending.add(Pending::Requested);
        pending.add(Pending::Hint);
        assert_eq!(pending, Pending::Requested);
    }

    #[test]
    fn a_hint_expires_only_failed_details() {
        let mut details = Details::default();
        let key = |name: &str| Key {
            id: name.into(),
            root: name.into(),
        };
        let failed = || FsError::from_io("space", Path::new("e"), io::ErrorKind::NotFound.into());
        for (name, space) in [
            (
                "good",
                Ok(SpaceInfo {
                    available: 1,
                    total: 2,
                }),
            ),
            ("empty", Err(failed())),
        ] {
            details.cached.insert(
                key(name),
                Cached {
                    at: Instant::now(),
                    details: VolumeDetails {
                        label: Ok(None),
                        space,
                    },
                },
            );
        }
        details.expire_failures();
        assert!(details.cached.contains_key(&key("good")));
        assert!(!details.cached.contains_key(&key("empty")));
    }
}
