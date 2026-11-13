//! Worker-owned occurrence size cache and bounded scan scheduling.
use crate::{
    folder_sizes::{self, IoBudget},
    providers::{ProviderError, ProviderTarget},
    tree_actor::CapturedSelection,
};
use midnight_core::*;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Notify;

const INTERVAL: Duration = Duration::from_millis(250);
struct Pending {
    request: TreeRequestId,
    captured: CapturedSelection,
    cancel: Cancellation,
}
#[derive(Default)]
struct Progress {
    values: HashMap<TreeRowId, FolderSize>,
    done: bool,
}
struct Active {
    retired: bool,
    stamp: TreeViewStamp,
    request: TreeRequestId,
    rows: Vec<TreeRowId>,
    progress: Arc<Mutex<Progress>>,
    cancel: Cancellation,
}

pub(crate) struct State {
    reports: crate::ReportArena,
    filesystem: Arc<dyn FileSystem>,
    budget: Arc<IoBudget>,
    wake: Arc<Notify>,
    pending: VecDeque<Pending>,
    active: Option<Active>,
    values: HashMap<TreeRowId, FolderSize>,
    version: u64,
    in_flight: Option<(TreeViewStamp, u64)>,
    last_map: Option<Arc<TreeSizeSnapshot>>,
    last_publish: Option<Instant>,
    dirty: bool,
    enabled: bool,
}
impl State {
    pub fn new(
        filesystem: Arc<dyn FileSystem>,
        wake: Arc<Notify>,
        reports: crate::ReportArena,
    ) -> Self {
        Self {
            reports,
            filesystem,
            wake,
            budget: Arc::new(IoBudget::new()),
            pending: VecDeque::new(),
            active: None,
            values: HashMap::new(),
            version: 0,
            in_flight: None,
            last_map: None,
            last_publish: None,
            dirty: false,
            enabled: false,
        }
    }
    pub fn busy(&self) -> bool {
        self.active.is_some() || !self.pending.is_empty() || self.dirty
    }
    pub fn room(&self) -> bool {
        self.pending.len() < 8
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn enqueue(
        &mut self,
        request: TreeRequestId,
        captured: CapturedSelection,
        cancel: &Cancellation,
    ) {
        self.enabled = true;
        self.pending.push_back(Pending {
            request,
            captured,
            cancel: cancel.child(),
        });
        self.dirty = true;
    }
    pub fn reset(&mut self) {
        if let Some(active) = &mut self.active {
            active.cancel.cancel();
            active.retired = true;
        }
        self.pending.clear();
        self.values.clear();
        self.in_flight = None;
        self.last_map = None;
        self.version = 0;
        self.last_publish = None;
        self.dirty = false;
        self.enabled = false;
    }
    pub fn invalidate(&mut self) -> Vec<TreeSizeEvent> {
        let mut events = Vec::new();
        if let Some(active) = &mut self.active {
            active.cancel.cancel();
            active.retired = true;
        }
        for pending in self.pending.drain(..) {
            pending.cancel.cancel();
            events.push(TreeSizeEvent::Finished {
                stamp: pending.captured.view.stamp,
                request: pending.request,
                cancelled: true,
            });
        }
        self.values.clear();
        self.dirty = true;
        events
    }
    pub fn acknowledge(&mut self, stamp: TreeViewStamp, version: u64) {
        if self.in_flight == Some((stamp, version)) {
            self.in_flight = None;
        }
    }
    pub fn cancel(&mut self, stamp: TreeViewStamp, request: TreeRequestId) -> Vec<TreeSizeEvent> {
        let mut events = Vec::new();
        if let Some(active) = &self.active
            && same_session(active.stamp, stamp)
            && active.request == request
        {
            active.cancel.cancel();
        }
        self.pending.retain(|pending| {
            if same_session(pending.captured.view.stamp, stamp) && pending.request == request {
                pending.cancel.cancel();
                events.push(TreeSizeEvent::Finished {
                    stamp: pending.captured.view.stamp,
                    request,
                    cancelled: true,
                });
                false
            } else {
                true
            }
        });
        self.dirty |= !events.is_empty();
        events
    }
    pub fn sort_sizes(&self) -> Arc<HashMap<TreeRowId, FolderSize>> {
        Arc::new(self.values.clone())
    }
    pub fn retain(&mut self, mut retained: impl FnMut(TreeRowId) -> bool) {
        let before = self.values.len();
        self.values.retain(|row, _| retained(*row));
        self.dirty |= before != self.values.len();
    }
    pub fn selected(&self, publication: &TreePublication) -> TreeSizeEvent {
        let mut summary = SelectedFolderSizes {
            complete_bytes: Some(0),
            partial_bytes: Some(0),
            ..Default::default()
        };
        let (mut complete, mut partial) = (0u128, 0u128);
        for (position, row) in publication.view.rows.iter().enumerate() {
            if !publication.selection.contains_position(position) {
                continue;
            }
            if matches!(
                row.entry.kind,
                EntryKind::Directory | EntryKind::DirectoryLink
            ) {
                summary.folders += 1;
            }
            if !matches!(
                row.entry.kind,
                EntryKind::Directory | EntryKind::DirectoryLink
            ) {
                continue;
            }
            if let Some(size) = self.values.get(&row.id) {
                if size.finished && size.errors == 0 {
                    summary.complete += 1;
                    complete += u128::from(size.bytes);
                } else {
                    summary.partial += 1;
                    partial += u128::from(size.bytes);
                }
            }
        }
        summary.complete_bytes = u64::try_from(complete).ok();
        summary.partial_bytes = u64::try_from(partial).ok();
        TreeSizeEvent::Selected {
            view: publication.view.clone(),
            selection_revision: publication.selection_revision,
            selection: publication.selection.clone(),
            version: self.version,
            summary,
        }
    }
    pub fn tick(&mut self, stamp: Option<TreeViewStamp>) -> Vec<TreeSizeEvent> {
        let mut events = Vec::new();
        if let Some(active) = &self.active {
            let mut progress = active.progress.lock().unwrap_or_else(|e| e.into_inner());
            let current = stamp.is_some_and(|stamp| same_session(stamp, active.stamp));
            let updates = std::mem::take(&mut progress.values);
            if current && !active.retired && !updates.is_empty() {
                self.values.extend(updates);
                self.dirty = true;
            }
            if progress.done {
                drop(progress);
                let active = self.active.take().expect("completed scan");
                if current {
                    // Ending the job clears its running rows. Cancellation does not
                    // complete its estimates or turn them into filesystem errors.
                    self.dirty = true;
                    events.push(TreeSizeEvent::Finished {
                        stamp: active.stamp,
                        request: active.request,
                        cancelled: active.cancel.is_cancelled(),
                    });
                }
            }
        }
        if self.active.is_none()
            && let Some(pending) = self.pending.pop_front()
        {
            let rows = pending
                .captured
                .targets
                .iter()
                .map(|target| target.row)
                .collect();
            let progress = Arc::new(Mutex::new(Progress::default()));
            self.active = Some(Active {
                retired: false,
                stamp: pending.captured.view.stamp,
                request: pending.request,
                rows,
                progress: progress.clone(),
                cancel: pending.cancel.clone(),
            });
            let filesystem = self.filesystem.clone();
            let reports = self.reports.clone();
            let budget = self.budget.clone();
            let wake = self.wake.clone();
            tokio::task::spawn_blocking(move || {
                scan(
                    filesystem.as_ref(),
                    &budget,
                    &pending.captured,
                    &pending.cancel,
                    &progress,
                    &wake,
                    &reports,
                );
                // Captured provider leases retire here before completion admission.
                drop(pending);
                progress.lock().unwrap_or_else(|e| e.into_inner()).done = true;
                wake.notify_one();
            });
        }
        let Some(stamp) = stamp else {
            return events;
        };
        if self.dirty
            && self.in_flight.is_none()
            && self
                .last_publish
                .is_none_or(|last| last.elapsed() >= INTERVAL)
        {
            let mut running = HashSet::new();
            if let Some(active) = &self.active
                && same_session(active.stamp, stamp)
                && !active.retired
            {
                running.extend(active.rows.iter().copied());
            }
            for pending in &self.pending {
                running.extend(pending.captured.targets.iter().map(|target| target.row));
            }
            if let Some(version) = self.version.checked_add(1)
                && let Ok(snapshot) =
                    TreeSizeSnapshot::new(stamp, version, self.values.clone(), running)
            {
                let sizes = Arc::new(snapshot);
                self.version = version;
                self.in_flight = Some((stamp, version));
                self.last_map = Some(sizes.clone());
                self.last_publish = Some(Instant::now());
                self.dirty = false;
                events.push(TreeSizeEvent::Snapshot { sizes });
            }
        }
        events
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.reset();
    }
}
fn same_session(a: TreeViewStamp, b: TreeViewStamp) -> bool {
    a.pane == b.pane && a.navigation == b.navigation && a.session == b.session
}

fn scan(
    filesystem: &dyn FileSystem,
    budget: &Arc<IoBudget>,
    captured: &CapturedSelection,
    cancel: &Cancellation,
    progress: &Mutex<Progress>,
    wake: &Notify,
    reports: &crate::ReportArena,
) {
    let mut native = Vec::new();
    for occurrence in &captured.targets {
        if cancel.is_cancelled() {
            return;
        }
        match occurrence.captured.target.value() {
            Ok(ProviderTarget::Native(path)) => native.push(Entry::new(
                EntryId(occurrence.row.0),
                path.clone(),
                captured.view.rows[captured.view.index.row_positions[&occurrence.row]]
                    .entry
                    .kind,
            )),
            Ok(ProviderTarget::Archive(target)) => {
                let mut latest = FolderSize::default();
                let result = (|| {
                    let mount = target
                        .location
                        .mount
                        .as_ref()
                        .ok_or(ProviderError::Unavailable)?;
                    let root = target
                        .original
                        .as_ref()
                        .ok_or(ProviderError::InvalidResponse(
                            "archive size target has no original entry",
                        ))?;
                    if root.kind != EntryKind::Directory
                        || root.id.entry != target.location.directory
                    {
                        return Err(ProviderError::InvalidResponse(
                            "archive size target is not its captured directory",
                        ));
                    }
                    let mut ancestors: Vec<_> =
                        target.location.trail.iter().map(|(id, _)| *id).collect();
                    if ancestors.pop() != Some(root.id.entry) {
                        return Err(ProviderError::InvalidResponse(
                            "archive size ancestor trail does not end at the target",
                        ));
                    }
                    let mut parent_address = target.location.address.clone();
                    if parent_address.components.pop().as_deref() != Some(root.raw_name.as_ref()) {
                        return Err(ProviderError::InvalidResponse(
                            "archive size address does not identify the raw target component",
                        ));
                    }
                    let ok = crate::archive_sizes::scan::compute_checked(
                        mount.as_ref(),
                        crate::archive_sizes::scan::ScanScope {
                            address: &parent_address,
                            ancestors: &ancestors,
                            reserved: &[],
                        },
                        std::slice::from_ref(root),
                        cancel,
                        &mut |id, size| {
                            if id == root.id {
                                latest = size.clone();
                                progress
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .values
                                    .insert(occurrence.row, size);
                                wake.notify_one();
                            }
                            !cancel.is_cancelled()
                        },
                        reports,
                    )
                    .map_err(|error| ProviderError::Failed(error.to_string()))?;
                    if ok {
                        Ok(())
                    } else {
                        Err(ProviderError::InvalidResponse(
                            "archive size traversal stopped with partial results",
                        ))
                    }
                })();
                if let Err(error) = result
                    && !cancel.is_cancelled()
                {
                    partial(
                        progress,
                        occurrence.row,
                        wake,
                        latest,
                        format_args!("Captured archive address {:?}", target.location.address),
                        &error.to_string(),
                        reports,
                    );
                }
            }
            Err(error) if !cancel.is_cancelled() && error != ProviderError::Cancelled => {
                partial(
                    progress,
                    occurrence.row,
                    wake,
                    FolderSize::default(),
                    format_args!(
                        "Captured tree occurrence {:?} in {:?}",
                        occurrence.row, captured.view.stamp
                    ),
                    &error.to_string(),
                    reports,
                );
            }
            Err(_) => {}
        }
    }
    if !native.is_empty() {
        folder_sizes::compute_batched(
            filesystem,
            &native,
            cancel,
            &mut |id, size| {
                progress
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .values
                    .insert(TreeRowId(id.0), size);
                wake.notify_one();
                !cancel.is_cancelled()
            },
            budget,
            reports,
        );
    }
}
fn partial(
    progress: &Mutex<Progress>,
    row: TreeRowId,
    wake: &Notify,
    mut size: FolderSize,
    location: impl std::fmt::Display,
    reason: &str,
    reports: &crate::ReportArena,
) {
    // Build storage and diagnostic authority before taking the progress mutex.
    // Existing scanner diagnostics already describe the failed partial result.
    if size.errors == 0 {
        let log = crate::size_warnings::ReportLog::for_scan(reports);
        let range = log.mark().report();
        log.record(
            location,
            "resolve tree directory size",
            std::io::ErrorKind::Other,
            reason,
        );
        range.finish(false);
        size.errors = 1;
        size.warnings = range.handle();
    }
    size.finished = true;
    progress
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values
        .insert(row, size);
    wake.notify_one();
}
