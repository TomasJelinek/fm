use super::{Event, Lane, Observation, Request, cache, publish, published, reject, scan};
use crate::{Outbox, Snapshots};
use midnight_core::{
    ArchiveMount, ArchiveSizeSnapshot, Cancellation, FolderSizeScanId, NavigationId, PaneId,
};
use std::{
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

const INTERVAL: Duration = Duration::from_millis(250);
struct Throttle(Instant);
impl Throttle {
    fn new(now: Instant) -> Self {
        Self(now)
    }
    fn due(&mut self, now: Instant) -> bool {
        if now.saturating_duration_since(self.0) < INTERVAL {
            return false;
        }
        self.0 = now;
        true
    }
}
struct Publication {
    mount: Weak<dyn ArchiveMount>,
    pane: PaneId,
    scan: Option<FolderSizeScanId>,
    navigation: NavigationId,
}

/// ACK ownership follows successful enqueue even if wakeup triggers cancellation.
/// Outbox::blocking_send instead reports whether the caller should continue work.
fn queue_snapshot(
    outbox: &Outbox,
    event: Event,
    cancel: &Cancellation,
    snapshots: &Snapshots,
    pane: PaneId,
    publication: published::Published,
) -> bool {
    let mut event = midnight_core::Event::ArchiveSizes(Box::new(event));
    let mut publication = Some(publication);
    loop {
        if cancel.is_cancelled() {
            return false;
        }
        // A consumer may receive before wakeup. Serialize its backend lookup
        // with enqueue and recording, without holding the lock across waits.
        let sent = {
            let mut states = snapshots.lock().unwrap_or_else(|e| e.into_inner());
            let sent = outbox.sender.try_send(event);
            if sent.is_ok() {
                states[pane.index()].archive_sort_sizes = publication.take();
            }
            sent
        };
        match sent {
            Ok(()) => {
                (outbox.wakeup)();
                return true;
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => return false,
            Err(tokio::sync::mpsc::error::TrySendError::Full(pending)) => event = pending,
        }
        std::thread::park_timeout(Duration::from_millis(10));
    }
}
pub(super) struct Worker {
    reports: crate::ReportArena,
    lane: Arc<Lane>,
    outbox: Outbox,
    snapshots: Snapshots,
    store: cache::Store,
    observer: Option<Observation>,
    version: u64,
    active_mount: Option<u64>,
    latest_navigation: NavigationId,
    latest_mount: Option<Weak<dyn ArchiveMount>>,
    deferred: Option<Publication>,
}
impl Worker {
    pub fn new(
        lane: Arc<Lane>,
        outbox: Outbox,
        snapshots: Snapshots,
        reports: crate::ReportArena,
    ) -> Self {
        Self {
            reports,
            lane,
            outbox,
            snapshots,
            store: cache::Store::default(),
            observer: None,
            version: 0,
            active_mount: None,
            latest_navigation: NavigationId(0),
            latest_mount: None,
            deferred: None,
        }
    }
    fn invalidate(&mut self, mount: &Arc<dyn ArchiveMount>, pane: PaneId) {
        published::invalidate(&self.snapshots, pane, mount);
        self.store.get(mount, self.active_mount).values.clear();
        if self
            .deferred
            .as_ref()
            .is_some_and(|pending| pending.mount.ptr_eq(&Arc::downgrade(mount)))
        {
            self.deferred = None;
        }
        publish(
            &self.outbox,
            Event::Invalidated {
                pane,
                mount: mount.id(),
            },
            &self.lane.stop,
        );
    }
    pub fn run(mut self) {
        loop {
            let request = {
                let mut pending = self.lane.pending.lock().unwrap_or_else(|e| e.into_inner());
                while !pending.closed
                    && pending.queue.is_empty()
                    && pending.observer.is_none()
                    && !pending.flush
                {
                    pending = self
                        .lane
                        .wake
                        .wait(pending)
                        .unwrap_or_else(|e| e.into_inner());
                }
                if pending.closed {
                    let retired = std::mem::take(&mut pending.queue);
                    let observer = pending.observer.take();
                    drop(pending);
                    drop(retired);
                    drop(observer);
                    break;
                }
                pending.flush = false;
                pending.queue.pop_front().map(|request| {
                    let cancel = Cancellation::default();
                    pending.active = Some((request.scan, cancel.clone()));
                    (request, cancel)
                })
            };
            self.active_mount = request
                .as_ref()
                .and_then(|(r, _)| r.ready.archive()?.mount.as_ref().map(|m| m.id()));
            self.observe();
            self.flush(&self.lane.stop.clone());
            if let Some((request, cancel)) = request {
                self.compute(request, &cancel);
                self.lane
                    .pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .active = None;
            }
            self.active_mount = None;
        }
    }
    fn observe(&mut self) {
        let next = self
            .lane
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .observer
            .take();
        let Some(next) = next else {
            return;
        };
        self.observer = None;
        let Some(ready) = next.ready.upgrade() else {
            return;
        };
        let pane = ready.snapshot().pane;
        if let Err(message) = cache::targets(&ready, Some(&next.selected)) {
            reject(&self.outbox, pane, None, message, &self.lane.stop);
            return;
        }
        let Some(mount) = ready.archive().and_then(|l| l.mount.clone()) else {
            return;
        };
        if mount.validate().is_err() {
            self.invalidate(&mount, pane);
            reject(
                &self.outbox,
                pane,
                None,
                "Archive source changed; cached sizes are unavailable.",
                &self.lane.stop,
            );
            return;
        }
        self.observer = Some(next);
        self.snapshot(
            &mount,
            pane,
            None,
            ready.snapshot().navigation,
            &self.lane.stop.clone(),
        );
    }
    fn snapshot(
        &mut self,
        mount: &Arc<dyn ArchiveMount>,
        pane: PaneId,
        scan: Option<FolderSizeScanId>,
        mut navigation: NavigationId,
        cancel: &Cancellation,
    ) -> bool {
        if cancel.is_cancelled() {
            return false;
        }
        if mount.validate().is_err() {
            self.invalidate(mount, pane);
            return false;
        }
        if navigation < self.latest_navigation {
            if !self
                .latest_mount
                .as_ref()
                .is_some_and(|source| source.ptr_eq(&Arc::downgrade(mount)))
            {
                return true;
            }
            // A resort changes view generation, not the retained scan authority.
            // Continue publishing its newer cache under the current same-mount view.
            navigation = self.latest_navigation;
        }
        self.latest_navigation = navigation;
        self.latest_mount = Some(Arc::downgrade(mount));
        // While the previous immutable map is in flight, retain only this small
        // descriptor and mutable cache. No additional full map is cloned.
        self.deferred = Some(Publication {
            mount: Arc::downgrade(mount),
            pane,
            scan,
            navigation,
        });
        self.flush(cancel)
    }
    fn flush(&mut self, cancel: &Cancellation) -> bool {
        if cancel.is_cancelled() {
            return false;
        }
        if self
            .lane
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .in_flight
            .is_some()
        {
            return true;
        }
        let Some(publication) = self.deferred.take() else {
            return true;
        };
        let Some(mount) = publication.mount.upgrade() else {
            return true;
        };
        if publication.navigation < self.latest_navigation {
            return true;
        }
        if mount.validate().is_err() {
            self.invalidate(&mount, publication.pane);
            return false;
        }
        let Some(version) = self.version.checked_add(1) else {
            return false;
        };
        let cache = self.store.get(&mount, self.active_mount);
        let sizes = Arc::new(ArchiveSizeSnapshot::new(
            mount.id(),
            version,
            cache.values.clone(),
        ));
        let selected = self.observer.as_ref().and_then(|observer| {
            let ready = observer.ready.upgrade()?;
            let source = ready.archive()?.mount.as_ref()?;
            if !Arc::ptr_eq(source, &mount) {
                return None;
            }
            Some(Event::Selected {
                request: observer.request,
                pane: publication.pane,
                navigation: ready.snapshot().navigation,
                revision: observer.revision,
                version,
                summary: cache::summary(observer, cache),
            })
        });
        if cancel.is_cancelled() {
            return false;
        }
        if mount.validate().is_err() {
            self.invalidate(&mount, publication.pane);
            return false;
        }
        let token = (mount.id(), version);
        self.lane
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .in_flight = Some(token);
        self.version = version;
        let publication_ref = published::Published::new(&mount, &sizes);
        if !queue_snapshot(
            &self.outbox,
            Event::Snapshot {
                pane: publication.pane,
                scan: publication.scan,
                sizes,
            },
            cancel,
            &self.snapshots,
            publication.pane,
            publication_ref,
        ) {
            let mut pending = self.lane.pending.lock().unwrap_or_else(|e| e.into_inner());
            if pending.in_flight == Some(token) {
                pending.in_flight = None;
            }
            return false;
        }
        selected.is_none_or(|event| publish(&self.outbox, event, cancel))
    }
    fn compute(&mut self, request: Request, cancel: &Cancellation) {
        let pane = request.ready.snapshot().pane;
        let result = self.run_scan(&request, cancel);
        if let Err(message) = &result {
            reject(
                &self.outbox,
                pane,
                Some(request.scan),
                message,
                &self.lane.stop,
            );
        }
        publish(
            &self.outbox,
            Event::Finished {
                pane,
                scan: request.scan,
                cancelled: cancel.is_cancelled() || result.is_err(),
            },
            &self.lane.stop,
        );
    }
    fn run_scan(&mut self, request: &Request, cancel: &Cancellation) -> Result<(), String> {
        let pane = request.ready.snapshot().pane;
        let navigation = request.ready.snapshot().navigation;
        let location = request
            .ready
            .archive()
            .ok_or("Archive source is unavailable.")?;
        let mount = location
            .mount
            .as_ref()
            .ok_or("Archive source is unavailable.")?
            .clone();
        let mut roots = cache::targets(&request.ready, request.selected.as_deref())?;
        if request.single && roots.len() != 1 {
            return Err("Space size requires one captured ordinary archive directory.".into());
        }
        if cancel.is_cancelled() {
            return Ok(());
        }
        if let Err(error) = mount.validate() {
            self.invalidate(&mount, pane);
            return Err(format!(
                "Archive source changed; cached sizes are unavailable. {error}"
            ));
        }
        roots.retain(|root| {
            !self
                .store
                .get(&mount, self.active_mount)
                .values
                .get(&root.id)
                .is_some_and(|size| size.finished && size.errors == 0)
        });
        let mut throttle = Throttle::new(Instant::now());
        let ancestors: Vec<_> = location.trail.iter().map(|(id, _)| *id).collect();
        let reserved: Vec<_> = request
            .ready
            .snapshot()
            .entries
            .iter()
            .map(|entry| entry.id.0)
            .collect();
        let reports = self.reports.clone();
        let scanned = scan::compute_checked(
            mount.as_ref(),
            scan::ScanScope {
                address: &location.address,
                ancestors: &ancestors,
                reserved: &reserved,
            },
            &roots,
            cancel,
            &mut |id, size| {
                let cache = self.store.get(&mount, self.active_mount);
                if cache.values.len() >= scan::MAX_NODES && !cache.values.contains_key(&id) {
                    return false;
                }
                cache.values.insert(id, size);
                if throttle.due(Instant::now()) {
                    self.observe();
                    return self.snapshot(&mount, pane, Some(request.scan), navigation, cancel);
                }
                !cancel.is_cancelled()
            },
            &reports,
        );
        let ok = match scanned {
            Ok(ok) => ok,
            Err(error) => {
                // The scanner has already replaced its provisional root with
                // the exact partial diagnostic. Invalidate stale sort/cache data
                // and report the retained error without querying the source again.
                self.invalidate(&mount, pane);
                return Err(format!(
                    "Archive source changed; calculated sizes are unavailable. {error}"
                ));
            }
        };
        if let Err(error) = mount.validate() {
            self.invalidate(&mount, pane);
            return Err(format!(
                "Archive source changed; calculated sizes are unavailable. {error}"
            ));
        }
        self.observe();
        if !self.snapshot(
            &mount,
            pane,
            Some(request.scan),
            navigation,
            &self.lane.stop.clone(),
        ) {
            return Err("Archive size results could not be published.".into());
        }
        if ok || cancel.is_cancelled() {
            Ok(())
        } else {
            Err("Archive size calculation stopped with partial results.".into())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_is_coalesced_to_four_publications_per_second() {
        let now = Instant::now();
        let mut throttle = Throttle::new(now);
        let mut count = 0;
        for millis in 0..=1000 {
            if throttle.due(now + Duration::from_millis(millis)) {
                count += 1;
            }
        }
        assert_eq!(count, 4);
        assert!(!throttle.due(now + Duration::from_millis(999)));
    }

    #[derive(Debug)]
    struct Source;
    impl ArchiveMount for Source {
        fn id(&self) -> u64 {
            7
        }
        fn list(
            &self,
            _: u64,
            _: &Cancellation,
        ) -> Result<Vec<midnight_core::ArchiveNode>, midnight_core::FsError> {
            panic!("no listing needed")
        }
        fn read(
            &self,
            _: u64,
            _: u64,
            _: u32,
            _: &Cancellation,
        ) -> Result<Vec<u8>, midnight_core::FsError> {
            panic!("no content")
        }
        fn validate(&self) -> Result<(), midnight_core::FsError> {
            Ok(())
        }
    }
    #[test]
    fn cancellation_from_wakeup_does_not_release_a_successfully_queued_snapshot() {
        let cancel = Cancellation::default();
        let wake_cancel = cancel.clone();
        let (sender, mut events) = tokio::sync::mpsc::channel(4);
        let lane = Arc::new(Lane::default());
        let snapshots = crate::test_snapshots(Default::default());
        let wake_snapshots = snapshots.clone();
        let mount: Arc<dyn ArchiveMount> = Arc::new(Source);
        let wake_mount = mount.clone();
        let mut worker = Worker::new(
            lane.clone(),
            Outbox {
                sender,
                wakeup: Arc::new(move || {
                    assert!(
                        published::snapshot(&wake_snapshots, PaneId::Left, &wake_mount).is_some()
                    );
                    wake_cancel.cancel();
                }),
            },
            snapshots,
            crate::ReportArena::testing(),
        );
        assert!(worker.snapshot(
            &mount,
            PaneId::Left,
            Some(FolderSizeScanId(1)),
            NavigationId(1),
            &cancel
        ));
        assert!(cancel.is_cancelled());
        assert_eq!(lane.pending.lock().unwrap().in_flight, Some((7, 1)));
        assert!(worker.snapshot(
            &mount,
            PaneId::Left,
            Some(FolderSizeScanId(1)),
            NavigationId(1),
            &lane.stop
        ));
        assert!(
            matches!(events.try_recv().unwrap(),midnight_core::Event::ArchiveSizes(event)if matches!(*event,Event::Snapshot{..}))
        );
        assert!(
            events.try_recv().is_err(),
            "final snapshot must wait for the already queued map's ACK"
        );
        assert_eq!(lane.pending.lock().unwrap().in_flight, Some((7, 1)));
    }

    #[test]
    fn failed_publication_keeps_previous_sort_snapshot_and_invalidation_clears_it() {
        let (sender, mut events) = tokio::sync::mpsc::channel(1);
        let lane = Arc::new(Lane::default());
        let snapshots = crate::test_snapshots(Default::default());
        let mut worker = Worker::new(
            lane.clone(),
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            snapshots.clone(),
            crate::ReportArena::testing(),
        );
        let mount: Arc<dyn ArchiveMount> = Arc::new(Source);
        assert!(worker.snapshot(&mount, PaneId::Left, None, NavigationId(1), &lane.stop));
        let midnight_core::Event::ArchiveSizes(event) = events.try_recv().unwrap() else {
            panic!("unexpected event")
        };
        let Event::Snapshot { sizes, .. } = *event else {
            panic!("unexpected size event")
        };
        lane.pending.lock().unwrap().in_flight = None;
        events.close();
        assert!(!worker.snapshot(&mount, PaneId::Left, None, NavigationId(1), &lane.stop));
        assert!(Arc::ptr_eq(
            &sizes,
            &published::snapshot(&snapshots, PaneId::Left, &mount).unwrap()
        ));
        let replacement: Arc<dyn ArchiveMount> = Arc::new(Source);
        published::invalidate(&snapshots, PaneId::Left, &replacement);
        assert!(published::snapshot(&snapshots, PaneId::Left, &mount).is_some());
        worker.invalidate(&mount, PaneId::Left);
        assert!(published::snapshot(&snapshots, PaneId::Left, &mount).is_none());
    }
}
