//! A pane's free space: read with each navigation, and read again when something may
//! have changed it (`Command::RefreshSpace`) and, while the main window is visible,
//! periodically for a fixed local disk.
//!
//! One worker serves both panes. Each pane has at most one read in flight; requests
//! that arrive meanwhile, or sooner than `Timing::throttle` after the last read
//! ended, leave one read owed, which starts once that time has passed, so the last
//! change is always read. A read answers only its own navigation, whose cancellation
//! drops it, as the read that accompanies a listing always did. Native reads share
//! the engine's auxiliary budget and run on the blocking pool, never on the command
//! loop; inputs are a watch slot, so no request queue can grow.

use crate::{Outbox, task_lifetime::TaskLifetime};
use midnight_core::{
    Cancellation, Event, FileSystem, FsError, NavigationId, PaneId, SpaceInfo, StorageKind,
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    sync::{Semaphore, watch},
    task::{AbortHandle, JoinSet},
    time::Instant,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Timing {
    /// The least time from the end of one read of a pane's free space to the start
    /// of its next requested one.
    pub(crate) throttle: Duration,
    /// How often a fixed local disk's free space is read while the window is visible.
    pub(crate) periodic: Duration,
}

/// A second between reads keeps a share answering slowly from being read back to
/// back while a job writes to it. Five seconds shows another program's writes to a
/// fixed local disk promptly; such a read takes microseconds and sends nothing when
/// the value did not change (see the free-space notes in the panel diagnostics).
pub(crate) const PRODUCTION_TIMING: Timing = Timing {
    throttle: Duration::from_secs(1),
    periodic: Duration::from_secs(5),
};

#[derive(Clone)]
struct Target {
    navigation: NavigationId,
    path: PathBuf,
    cancel: Cancellation,
}

#[derive(Clone, Default)]
struct PaneInput {
    target: Option<Target>,
    /// Counts the refresh requests accepted for any target.
    refreshes: u64,
}

struct Input {
    panes: [PaneInput; 2],
    visible: bool,
}

pub(crate) struct Controller {
    input: watch::Sender<Input>,
}

impl Controller {
    pub(crate) fn new(
        scope: TaskLifetime,
        filesystem: Arc<dyn FileSystem>,
        outbox: Outbox,
        budget: Arc<Semaphore>,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        Self::with_timing(scope, filesystem, outbox, budget, PRODUCTION_TIMING, tasks)
    }

    pub(crate) fn with_timing(
        scope: TaskLifetime,
        filesystem: Arc<dyn FileSystem>,
        outbox: Outbox,
        budget: Arc<Semaphore>,
        timing: Timing,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let (input, receiver) = watch::channel(Input {
            panes: Default::default(),
            visible: true,
        });
        let worker = Worker {
            scope: scope.clone(),
            filesystem,
            outbox,
            budget,
            timing,
        };
        tasks.spawn(scope.future(worker.run(receiver)));
        Self { input }
    }

    /// `pane` lists `path` for `navigation`: read its storage kind and free space
    /// now. `cancel` is the navigation's own; it drops the reads once superseded.
    pub(crate) fn navigate(
        &self,
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
        cancel: Cancellation,
    ) {
        self.input.send_modify(|input| {
            input.panes[pane.index()].target = Some(Target {
                navigation,
                path,
                cancel,
            });
        });
    }

    /// `pane` shows no native folder (an archive or Network): nothing to read.
    pub(crate) fn leave(&self, pane: PaneId) {
        self.input
            .send_if_modified(|input| input.panes[pane.index()].target.take().is_some());
    }

    /// Read `pane`'s free space again if it still shows `navigation`.
    pub(crate) fn refresh(&self, pane: PaneId, navigation: NavigationId) {
        self.input.send_if_modified(|input| {
            let pane = &mut input.panes[pane.index()];
            if pane
                .target
                .as_ref()
                .is_none_or(|target| target.navigation != navigation)
            {
                return false;
            }
            pane.refreshes = pane.refreshes.wrapping_add(1);
            true
        });
    }

    /// Whether the main window is visible; periodic reads wait while it is not.
    pub(crate) fn set_visible(&self, visible: bool) {
        self.input.send_if_modified(|input| {
            let changed = input.visible != visible;
            input.visible = visible;
            changed
        });
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Read {
    /// The first read of a navigation: its storage kind, then its free space.
    First,
    Again,
}

#[derive(Default)]
struct Pane {
    target: Option<Target>,
    refreshes: u64,
    /// The first read of `target` has not started.
    first: bool,
    /// A requested read waits for the running one or for the throttle.
    owed: bool,
    /// The running read of `target`; an older target's read is aborted.
    reading: Option<AbortHandle>,
    /// When the last read of `target` started or, once it ended, ended.
    last: Option<Instant>,
    kind: Option<StorageKind>,
    /// The value last sent for `target`; an equal one is not sent again.
    sent: Option<SpaceInfo>,
}

impl Pane {
    fn update(&mut self, input: &PaneInput) {
        let same = match (&self.target, &input.target) {
            (Some(old), Some(new)) => old.navigation == new.navigation,
            (None, None) => true,
            _ => false,
        };
        if !same {
            if let Some(read) = self.reading.take() {
                // Its native call, if it started, keeps its permit until it returns.
                read.abort();
            }
            *self = Self {
                first: input.target.is_some(),
                target: input.target.clone(),
                refreshes: input.refreshes,
                ..Self::default()
            };
        } else if self.refreshes != input.refreshes {
            self.refreshes = input.refreshes;
            self.owed = self.target.is_some();
        }
    }

    /// A fixed local disk: an SSD or rotational disk on a SATA, ATA or NVMe bus.
    /// Removable, network and unclassified storage is read only when asked.
    fn periodic(&self, visible: bool) -> bool {
        visible
            && matches!(
                self.kind,
                Some(StorageKind::LocalSsd | StorageKind::Rotational)
            )
    }

    fn due(&mut self, now: Instant, visible: bool, timing: Timing) -> Option<Read> {
        let target = self.target.as_ref()?;
        if self.reading.is_some() || target.cancel.is_cancelled() {
            return None;
        }
        if self.first {
            return Some(Read::First);
        }
        let last = self.last?;
        if !self.owed && self.periodic(visible) && now >= last + timing.periodic {
            self.owed = true;
        }
        (self.owed && now >= last + timing.throttle).then_some(Read::Again)
    }

    fn deadline(&self, visible: bool, timing: Timing) -> Option<Instant> {
        let target = self.target.as_ref()?;
        if self.reading.is_some() || target.cancel.is_cancelled() {
            return None;
        }
        let last = self.last?;
        if self.owed {
            Some(last + timing.throttle)
        } else {
            self.periodic(visible).then(|| last + timing.periodic)
        }
    }
}

struct Done {
    pane: PaneId,
    navigation: NavigationId,
    kind: Option<StorageKind>,
    space: Option<Result<SpaceInfo, FsError>>,
}

struct Worker {
    scope: TaskLifetime,
    filesystem: Arc<dyn FileSystem>,
    outbox: Outbox,
    budget: Arc<Semaphore>,
    timing: Timing,
}

impl Worker {
    async fn run(self, mut input: watch::Receiver<Input>) {
        let mut panes: [Pane; 2] = Default::default();
        let mut visible = true;
        let mut reads = JoinSet::new();
        loop {
            let now = Instant::now();
            for (pane, state) in [PaneId::Left, PaneId::Right].into_iter().zip(&mut panes) {
                if let Some(read) = state.due(now, visible, self.timing) {
                    let target = state.target.clone().expect("a due read has a target");
                    state.first = false;
                    state.owed = false;
                    state.last = Some(now);
                    state.reading = Some(self.start(pane, target, read, &mut reads));
                }
            }
            let deadline = panes
                .iter()
                .filter_map(|pane| pane.deadline(visible, self.timing))
                .min();
            tokio::select! {
                changed = input.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let input = input.borrow_and_update();
                    visible = input.visible;
                    for (pane, input) in panes.iter_mut().zip(&input.panes) {
                        pane.update(input);
                    }
                }
                done = reads.join_next(), if !reads.is_empty() => {
                    let done = match done {
                        Some(Ok(done)) => done,
                        Some(Err(error)) => {
                            // Aborted for a newer target, or failed: the pane may read again.
                            for pane in &mut panes {
                                if pane.reading.as_ref().is_some_and(|read| read.id() == error.id()) {
                                    pane.reading = None;
                                    pane.last = Some(Instant::now());
                                }
                            }
                            continue;
                        }
                        None => continue,
                    };
                    let pane = &mut panes[done.pane.index()];
                    let Some(target) = pane.target.as_ref()
                        .filter(|target| target.navigation == done.navigation) else { continue; };
                    pane.reading = None;
                    pane.last = Some(Instant::now());
                    if target.cancel.is_cancelled() {
                        continue;
                    }
                    if let Some(kind) = done.kind {
                        pane.kind = Some(kind);
                    }
                    let Some(Ok(space)) = done.space else { continue; };
                    if pane.sent == Some(space) {
                        continue;
                    }
                    pane.sent = Some(space);
                    let event = Event::SpaceUpdated { pane: done.pane, navigation: done.navigation, space };
                    if !self.outbox.send(event).await {
                        break;
                    }
                }
                () = tokio::time::sleep_until(deadline.unwrap_or(now)), if deadline.is_some() => {}
            }
        }
    }

    fn start(
        &self,
        pane: PaneId,
        target: Target,
        read: Read,
        reads: &mut JoinSet<Done>,
    ) -> AbortHandle {
        let filesystem = self.filesystem.clone();
        let outbox = self.outbox.clone();
        let budget = self.budget.clone();
        let worker_scope = self.scope.clone();
        let Target {
            navigation,
            path,
            cancel,
        } = target;
        reads.spawn(self.scope.future(async move {
            let mut done = Done {
                pane,
                navigation,
                kind: None,
                space: None,
            };
            let Ok(permit) = budget.acquire_owned().await else {
                return done;
            };
            if cancel.is_cancelled() {
                return done;
            }
            // Both halves own the permit: the native call keeps it until it returns,
            // also when this task is aborted by a newer navigation meanwhile.
            let permit = Arc::new(permit);
            let worker_permit = permit.clone();
            let _permit = permit;
            let result = tokio::task::spawn_blocking(worker_scope.blocking(move || {
                let _permit = worker_permit;
                let kind = (read == Read::First).then(|| {
                    let kind = filesystem.storage_kind(&path).unwrap_or_default();
                    // Sent before the space query, which can block on a network share.
                    outbox.blocking_send(
                        Event::StorageKindUpdated {
                            pane,
                            navigation,
                            kind,
                        },
                        &cancel,
                    );
                    kind
                });
                (kind, filesystem.space_info(&path))
            }))
            .await;
            if let Ok((kind, space)) = result {
                done.kind = kind;
                done.space = Some(space);
            }
            done
        }))
    }
}

#[cfg(test)]
#[path = "free_space_tests.rs"]
mod tests;
