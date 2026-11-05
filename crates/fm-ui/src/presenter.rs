use fm_core::*;
mod bookmarks;
mod choices;
mod desktop;
mod fonts;
mod preferences;
mod search;
mod selection;
mod shortcuts;
mod tabs;
mod terminal;
mod volumes;
use std::{
    collections::{HashMap, HashSet},
    mem,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
pub use tabs::TabSummary;

/// Owned allocations returned to the application's background reclamation queue.
pub type DeferredDrop = Box<dyn Send>;

struct Request {
    navigation: NavigationId,
    path: PathBuf,
    history: Option<usize>,
    space: Option<SpaceInfo>,
    previous_cursor: Option<EntryId>,
    previous_parent_cursor: bool,
    previous_selected: Arc<SelectionSnapshot>,
    previous_selection_origin: Option<SelectionOrigin>,
    wanted_cursor: Option<PathBuf>,
    cursor_touched: bool,
    /// Refreshes and cached resorts stage privately behind the committed view.
    keep_rows: bool,
}

/// The scan owns its original identities independently of listing/sort generations.
pub(crate) struct SizeScanScope {
    navigation: NavigationId,
    path: PathBuf,
    entries: Arc<[Entry]>,
    index: Arc<DirectoryIndex>,
}

pub(crate) struct Pane {
    /// Last successful location. Provisional rows belong to `display_path()`.
    pub(crate) path: PathBuf,
    pub(crate) entries: Arc<[Entry]>,
    pub(crate) cursor: Option<EntryId>,
    pub(crate) parent_cursor: bool,
    pub(crate) folder_sizes: HashMap<PathBuf, FolderSize>,
    pub(crate) size_scan: Option<FolderSizeScanId>,
    pub(crate) size_scan_scope: Option<SizeScanScope>,
    pub(crate) size_scan_attempted: bool,
    pub(crate) preserve_viewport: bool,
    size_scan_generation: u64,
    pub(crate) selected: Arc<SelectionSnapshot>,
    selection_context: NavigationId,
    selection_committed_context: NavigationId,
    selection_revision: SelectionRevision,
    selection_applied: SelectionRevision,
    selection_resolved: SelectionRevision,
    selection_reset: bool,
    selection_restore: Option<SelectionOrigin>,
    selection_pending_bulk: bool,
    selection_dirty: bool,
    selection_rebase_pending: bool,
    pub(crate) selection_deferred: bool,
    pub(crate) history: Vec<PathBuf>,
    pub(crate) history_index: Option<usize>,
    pub(crate) error: String,
    pub(crate) loading: bool,
    pub(crate) sort: SortSpec,
    pub(crate) space: Option<SpaceInfo>,
    pub(crate) streaming: Vec<Entry>,
    pub(crate) metadata: HashMap<EntryId, EntryMetadata>,
    pub(crate) index: Arc<DirectoryIndex>,
    pub(crate) volume_paths: HashMap<PathBuf, PathBuf>,
    streaming_index: HashMap<EntryId, usize>,
    pending: Option<Request>,
    pub(crate) generation: u64,
    successful_generation: Option<NavigationId>,
    committed_sort: SortSpec,
    pub(crate) search: search::SearchState,
}

/// In-memory state for two independent panes and their current navigation generations.
pub struct Presenter {
    pub(crate) panes: [Pane; 2],
    pub(crate) volumes: Vec<Volume>,
    pub(crate) volumes_revision: i32,
    pub(crate) volume_details: HashMap<String, (PathBuf, VolumeDetails)>,
    pub(crate) volumes_error: String,
    pub(crate) active: PaneId,
    pub(crate) desktop: desktop::DesktopState,
    pub(crate) preferences: preferences::PreferencesState,
    pub(crate) fonts: fonts::FontsState,
    tabs: tabs::TabsState,
    choices: choices::ChoicesState,
    selection_properties: [Option<selection::PendingProperties>; 2],
    search_revisions: [SearchRevision; 2],
    search_cancellations: [Option<SearchRevision>; 2],
    retired: Vec<DeferredDrop>,
}

impl Presenter {
    /// Create initial pane locations without sending commands or performing filesystem work.
    pub fn new(left: PathBuf, right: PathBuf) -> Self {
        Self {
            tabs: tabs::TabsState::new(),
            choices: choices::ChoicesState::default(),
            selection_properties: [None, None],
            search_revisions: [SearchRevision(0); 2],
            search_cancellations: [None, None],
            panes: [Pane::new(left), Pane::new(right)],
            volumes: vec![],
            volumes_revision: 0,
            volume_details: HashMap::new(),
            volumes_error: String::new(),
            active: PaneId::Left,
            desktop: desktop::DesktopState::default(),
            preferences: preferences::PreferencesState::default(),
            fonts: fonts::FontsState::default(),
            retired: vec![],
        }
    }

    /// The caller must transfer these values to a background owner before dropping them.
    pub fn take_retired(&mut self) -> Vec<DeferredDrop> {
        mem::take(&mut self.retired)
    }

    /// Share the actually committed snapshot for backend watches and cached
    /// operations. A provisional or still-loading directory is never acknowledged.
    pub fn visible_directory(&self, pane: PaneId) -> Option<VisibleDirectory> {
        let p = &self.panes[pane.index()];
        if p.loading {
            return None;
        }
        Some(VisibleDirectory {
            pane,
            navigation: p.successful_generation?,
            path: p.path.clone(),
            entries: p.entries.clone(),
        })
    }

    /// Refresh a watch hint only while its exact committed generation is visible.
    /// The caller's bounded sender may reject the request without changing rows.
    pub fn refresh_if_changed(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &self.panes[pane.index()];
        if p.loading || p.successful_generation != Some(navigation) {
            return false;
        }
        let path = p.path.clone();
        let accepted = self.navigate(pane, path, None, send);
        if accepted {
            self.panes[pane.index()].preserve_viewport = true;
        }
        accepted
    }

    /// Detach live state after the window closes; reclaim `take_retired()` on a worker.
    pub fn retire_all(&mut self) {
        self.retired
            .push(Box::new(mem::take(&mut self.choices.items)));
        self.retire_selection_properties();
        self.tabs.retire(&mut self.retired);
        for p in &mut self.panes {
            self.retired.push(Box::new(p.search.result.take()));
            self.retired.push(Box::new(mem::take(&mut p.search.query)));
            self.retired
                .push(Box::new(mem::replace(&mut p.entries, Arc::from([]))));
            self.retired.push(Box::new(mem::replace(
                &mut p.index,
                Arc::new(DirectoryIndex::from_entries(&[])),
            )));
            self.retired.push(Box::new(mem::take(&mut p.selected)));
            self.retired.push(Box::new(mem::take(&mut p.history)));
            self.retired.push(Box::new(mem::take(&mut p.volume_paths)));
            self.retired.push(Box::new(p.pending.take()));
            p.retire_stream(&mut self.retired);
            p.cursor = None;
            p.parent_cursor = false;
            self.retired.push(Box::new(mem::take(&mut p.folder_sizes)));
            p.size_scan = None;
            self.retired.push(Box::new(p.size_scan_scope.take()));
            p.size_scan_attempted = false;
            p.history_index = None;
            p.loading = false;
        }
        self.retired.push(Box::new(mem::take(&mut self.volumes)));
        self.retired
            .push(Box::new(mem::take(&mut self.volume_details)));
        self.retired
            .push(Box::new(mem::take(&mut self.fonts.catalog)));
        self.retired
            .push(Box::new(mem::take(&mut self.preferences.values.bookmarks)));
    }

    pub(crate) fn navigate(
        &mut self,
        pane: PaneId,
        path: PathBuf,
        history: Option<usize>,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        self.navigate_impl(pane, path, history, false, send)
    }

    fn navigate_impl(
        &mut self,
        pane: PaneId,
        path: PathBuf,
        history: Option<usize>,
        resort: bool,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &mut self.panes[pane.index()];
        let navigation = NavigationId(p.generation + 1);
        let command = if resort {
            let source = p
                .successful_generation
                .filter(|_| p.path == path)
                .map(|navigation| VisibleDirectory {
                    pane,
                    navigation,
                    path: p.path.clone(),
                    entries: p.entries.clone(),
                });
            Command::Resort {
                pane,
                navigation,
                path: path.clone(),
                sort: p.sort,
                source,
            }
        } else if path == p.path && p.successful_generation.is_some() {
            Command::Refresh {
                pane,
                navigation,
                path: path.clone(),
                sort: p.sort,
                scan: p.size_scan,
            }
        } else {
            Command::Navigate {
                pane,
                navigation,
                path: path.clone(),
                sort: p.sort,
            }
        };
        if !send(command) {
            p.error = "Navigation queue is busy. Try again.".into();
            return false;
        }
        self.navigation_accepted_impl(pane, navigation, path, history, resort, send);
        true
    }

    /// Commit a navigation only after its bounded command was accepted.
    fn navigation_accepted(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
        history: Option<usize>,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.navigation_accepted_impl(pane, navigation, path, history, false, send);
    }

    fn navigation_accepted_impl(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
        history: Option<usize>,
        resort: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        let new_location = path != self.panes[pane.index()].path
            || self.panes[pane.index()].successful_generation.is_none();
        self.search_navigation_accepted(pane, new_location, send);
        self.selection_navigation_accepted(pane, navigation, &path);
        let p = &mut self.panes[pane.index()];
        let same_location = path == p.path && p.successful_generation.is_some();
        if !same_location && let Some(scan) = p.size_scan.take() {
            // Navigate is also a cancellation boundary in the backend; this explicit
            // request keeps intent visible when the navigation queue has room.
            send(Command::CancelFolderSizes { pane, scan });
        }
        if !same_location {
            self.retired.push(Box::new(p.size_scan_scope.take()));
        }
        if !same_location || (!resort && p.size_scan.is_none()) {
            self.retired.push(Box::new(mem::take(&mut p.folder_sizes)));
            p.size_scan_attempted = false;
        }
        if let Some(mut superseded) = p.pending.take() {
            if !superseded.keep_rows {
                self.retired.push(Box::new(mem::replace(
                    &mut p.selected,
                    mem::take(&mut superseded.previous_selected),
                )));
                p.cursor = superseded.previous_cursor;
                p.parent_cursor = superseded.previous_parent_cursor;
            }
            self.retired.push(Box::new(superseded));
        }
        p.retire_stream(&mut self.retired);
        let keep_rows = path == p.path && p.successful_generation.is_some();
        let previous_cursor = if keep_rows { p.cursor } else { p.cursor.take() };
        let previous_parent_cursor = p.parent_cursor;
        p.parent_cursor =
            parent_path(&path).is_some() && (path != p.path || previous_parent_cursor);
        let wanted_cursor = previous_cursor
            .and_then(|id| p.index.by_id.get(&id))
            .and_then(|i| p.entries.get(*i))
            .map(|e| e.path.clone());
        p.pending = Some(Request {
            navigation,
            path,
            history,
            space: None,
            previous_cursor,
            previous_parent_cursor,
            previous_selected: if keep_rows {
                Arc::default()
            } else {
                mem::take(&mut p.selected)
            },
            previous_selection_origin: p.successful_generation.map(|_| SelectionOrigin {
                navigation: p.selection_committed_context,
                location: p.path.clone(),
            }),
            wanted_cursor,
            cursor_touched: false,
            keep_rows,
        });
        p.generation = navigation.0;
        p.loading = true;
        p.error.clear();
        p.preserve_viewport = false;
    }

    /// Apply a backend event and dispatch any validated directory-link followup.
    /// The sender must be bounded and nonblocking, like the regular UI callbacks.
    pub fn apply_with_sender(&mut self, event: Event, send: &dyn Fn(Command) -> bool) {
        let committed = match &event {
            Event::ListingFinished {
                pane, navigation, ..
            }
            | Event::ListingFailed {
                pane, navigation, ..
            } if self.panes[pane.index()].generation == navigation.0
                && self.panes[pane.index()].pending.is_some() =>
            {
                Some(*pane)
            }
            _ => None,
        };
        self.dispatch_selection_properties(&event, send);
        self.apply(event);
        if let Some(pane) = committed {
            self.panes[pane.index()].selection_rebase_pending = true;
        }
        self.flush_search(send);
        self.flush_selection(send);
        self.dispatch_open_directories(send);
        self.flush_loaded_preferences(send);
    }

    /// Apply a backend event, retiring stale payloads and preserving the last usable location.
    pub fn apply(&mut self, event: Event) {
        let Some(event) = self.apply_search_event(event) else {
            return;
        };
        let Some(event) = self.apply_selection_event(event) else {
            return;
        };
        let Some(event) = self.apply_desktop(event) else {
            return;
        };
        let event =
            match event {
                Event::VolumesUpdated(volumes) => {
                    if self.volumes.len() != volumes.len()
                        || self.volumes.iter().zip(&volumes).any(|(before, after)| {
                            before.id != after.id || before.root != after.root
                        })
                    {
                        self.volumes_revision = self.volumes_revision.wrapping_add(1);
                    }
                    self.retain_volume_details(&volumes);
                    self.retired
                        .push(Box::new(mem::replace(&mut self.volumes, volumes)));
                    for p in &mut self.panes {
                        if p.history_index.is_some()
                            && let Some(volume) = self
                                .volumes
                                .iter()
                                .filter(|v| p.path.starts_with(&v.root))
                                .max_by_key(|v| v.root.components().count())
                        {
                            p.volume_paths.insert(volume.root.clone(), p.path.clone());
                        }
                    }
                    self.volumes_error.clear();
                    self.choices.authoritative_volumes = true;
                    self.sync_navigation_choices();
                    return;
                }
                Event::VolumesFailed(error) => {
                    self.volumes_error = error.to_string();
                    return;
                }
                Event::VolumeDetailsUpdated {
                    volume_id,
                    root,
                    details,
                } => {
                    self.volume_details_updated(volume_id, root, details);
                    return;
                }
                event => event,
            };
        let (pane, navigation) = match &event {
            Event::ListingStarted {
                pane, navigation, ..
            }
            | Event::EntriesBatch {
                pane, navigation, ..
            }
            | Event::MetadataBatch {
                pane, navigation, ..
            }
            | Event::ListingFinished {
                pane, navigation, ..
            }
            | Event::ListingFailed {
                pane, navigation, ..
            }
            | Event::SpaceUpdated {
                pane, navigation, ..
            } => (*pane, *navigation),
            _ => unreachable!(),
        };
        if matches!(&event, Event::ListingFailed { .. })
            && self.panes[pane.index()].generation == navigation.0
            && self.panes[pane.index()]
                .pending
                .as_ref()
                .is_some_and(|request| !request.keep_rows)
        {
            self.cancel_selection_properties(pane);
        }
        let p = &mut self.panes[pane.index()];
        if navigation.0 != p.generation
            || (!matches!(&event, Event::SpaceUpdated { .. }) && p.pending.is_none())
        {
            self.retired.push(Box::new(event));
            return;
        }
        match event {
            Event::ListingStarted { .. } => {}
            Event::EntriesBatch { entries, .. } => {
                let request = p.pending.as_ref().expect("validated pending generation");
                let same_location = request.path == p.path;
                for (offset, entry) in entries.iter().enumerate() {
                    p.streaming_index
                        .insert(entry.id, p.streaming.len() + offset);
                    if !request.keep_rows
                        && !p.parent_cursor
                        && !request.cursor_touched
                        && request.wanted_cursor.as_ref() == Some(&entry.path)
                    {
                        p.cursor = Some(entry.id);
                    }
                }
                if !request.keep_rows && same_location && !p.selection_dirty {
                    self.retired.push(Box::new(mem::replace(
                        &mut p.selected,
                        request.previous_selected.clone(),
                    )));
                }
                if !request.keep_rows && p.cursor.is_none() && !p.parent_cursor {
                    p.cursor = entries.first().map(|e| e.id);
                }
                p.streaming.extend(entries);
            }
            Event::MetadataBatch { entries, .. } => {
                p.metadata.extend(entries);
            }
            Event::ListingFinished { entries, index, .. } => {
                let mut request = p.pending.take().expect("validated pending generation");
                debug_assert_eq!(request.navigation, navigation);
                let same_location = p.path == request.path;
                let (source_entries, source_index, source_cursor) = if request.keep_rows {
                    (p.entries.as_ref(), &p.index.by_id, p.cursor)
                } else if !p.streaming.is_empty() {
                    (p.streaming.as_slice(), &p.streaming_index, p.cursor)
                } else if same_location {
                    (p.entries.as_ref(), &p.index.by_id, request.previous_cursor)
                } else {
                    (p.streaming.as_slice(), &p.streaming_index, p.cursor)
                };
                let raw_path = |id: &EntryId| {
                    source_index
                        .get(id)
                        .and_then(|i| source_entries.get(*i))
                        .map(|e| &e.path)
                };
                let cursor = source_cursor
                    .as_ref()
                    .and_then(raw_path)
                    .and_then(|path| index.by_path.get(path))
                    .copied()
                    .or_else(|| {
                        (!request.cursor_touched)
                            .then_some(request.wanted_cursor.as_ref())
                            .flatten()
                            .and_then(|path| index.by_path.get(path))
                            .copied()
                    });
                if !request.keep_rows
                    && p.streaming.is_empty()
                    && same_location
                    && !p.selection_dirty
                {
                    self.retired.push(Box::new(mem::replace(
                        &mut p.selected,
                        request.previous_selected.clone(),
                    )));
                }
                p.parent_cursor = parent_path(&request.path).is_some() && p.parent_cursor;
                p.cursor = if p.parent_cursor {
                    None
                } else {
                    cursor.or_else(|| entries.first().map(|e| e.id))
                };
                p.space = request.space.or(if same_location { p.space } else { None });
                p.path = mem::take(&mut request.path);
                if let Some(volume) = self
                    .volumes
                    .iter()
                    .filter(|v| p.path.starts_with(&v.root))
                    .max_by_key(|v| v.root.components().count())
                {
                    p.volume_paths.insert(volume.root.clone(), p.path.clone());
                }
                if let Some(location) = self
                    .desktop
                    .locations
                    .iter()
                    .filter(|location| {
                        location.kind == LocationKind::Volume && p.path.starts_with(&location.path)
                    })
                    .max_by_key(|location| location.path.components().count())
                {
                    p.volume_paths.insert(location.path.clone(), p.path.clone());
                }
                if let Some(history) = request.history {
                    p.history_index = Some(history);
                } else if p.history_index.and_then(|i| p.history.get(i)) != Some(&p.path) {
                    p.history.truncate(p.history_index.map_or(0, |i| i + 1));
                    p.history.push(p.path.clone());
                    p.history_index = Some(p.history.len() - 1);
                }
                self.retired
                    .push(Box::new(mem::replace(&mut p.entries, entries)));
                self.retired
                    .push(Box::new(mem::replace(&mut p.index, index)));
                self.retired.push(Box::new(request));
                p.retire_stream(&mut self.retired);
                p.successful_generation = Some(navigation);
                p.selection_committed_context = p.selection_context;
                p.committed_sort = p.sort;
                p.loading = false;
                p.error.clear();
            }
            Event::ListingFailed { error, .. } => {
                let mut request = p.pending.take().expect("validated pending generation");
                if !request.keep_rows {
                    p.selection_reset = true;
                    // The failed destination still shares this navigation ID
                    // with the restored view. Retire its pending selection
                    // acknowledgments before scheduling the source rebase.
                    p.selection_applied = p.selection_revision;
                    p.selection_resolved = p.selection_revision;
                    p.selection_restore = request.previous_selection_origin.take();
                    p.selection_context = p
                        .selection_restore
                        .as_ref()
                        .map_or(NavigationId(0), |origin| origin.navigation);
                    self.retired.push(Box::new(mem::replace(
                        &mut p.selected,
                        mem::take(&mut request.previous_selected),
                    )));
                    p.cursor = request.previous_cursor;
                    p.parent_cursor = request.previous_parent_cursor;
                }
                p.sort = p.committed_sort;
                self.retired.push(Box::new(request));
                p.retire_stream(&mut self.retired);
                p.loading = false;
                p.error = error.to_string();
            }
            Event::SpaceUpdated { space, .. } => {
                if let Some(request) = &mut p.pending {
                    request.space = Some(space);
                } else if p.successful_generation == Some(navigation) {
                    p.space = Some(space);
                }
            }
            _ => unreachable!(),
        }
    }

    pub(crate) fn up(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) -> bool {
        let Some(path) = self.panes[pane.index()]
            .display_path()
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_owned)
        else {
            return false;
        };
        self.navigate(pane, path, None, send)
    }
    pub(crate) fn history(
        &mut self,
        pane: PaneId,
        delta: isize,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &self.panes[pane.index()];
        let Some(index) = p
            .history_index
            .and_then(|i| i.checked_add_signed(delta))
            .filter(|i| *i < p.history.len())
        else {
            return false;
        };
        self.navigate(pane, p.history[index].clone(), Some(index), send)
    }
    /// Whether either pane has an accepted navigation that has not completed.
    pub fn is_loading(&self) -> bool {
        self.panes.iter().any(|p| p.loading)
    }
    /// Whether volume discovery or either pane currently has a displayed error.
    pub fn has_errors(&self) -> bool {
        !self.desktop.error.is_empty()
            || !self.volumes_error.is_empty()
            || self.panes.iter().any(|p| !p.error.is_empty())
    }
    pub(crate) fn navigate_text(
        &mut self,
        pane: PaneId,
        text: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let current = self.panes[pane.index()].display_path();
        if text == current.to_string_lossy() {
            return self.navigate(pane, current.to_owned(), None, send);
        }
        let input = PathBuf::from(text);
        let path = logical_path(if input.is_absolute() {
            input
        } else {
            current.join(input)
        });
        self.navigate(pane, path, None, send)
    }
    pub(crate) fn open(
        &mut self,
        pane: PaneId,
        row: usize,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if self.panes[pane.index()].has_parent() && row == 0 {
            return self.up(pane, send);
        }
        let Some(row) = row.checked_sub(self.panes[pane.index()].parent_offset()) else {
            return false;
        };
        let entry = self.panes[pane.index()]
            .display_entry(row)
            .map(|entry| (entry.kind, entry.path.clone()));
        match entry {
            Some((EntryKind::Directory | EntryKind::DirectoryLink, path)) => {
                self.navigate(pane, path, None, send)
            }
            Some((EntryKind::File | EntryKind::Symlink, path)) => {
                self.open_native_entry(pane, path, send)
            }
            Some((EntryKind::Other, _)) => {
                self.desktop.report(
                    "Cannot open this entry",
                    "Only folders, regular files and links to them can be opened.".into(),
                );
                false
            }
            None => false,
        }
    }
    pub(crate) fn select_row(
        &mut self,
        pane: PaneId,
        row: usize,
        toggle: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        let p = &mut self.panes[pane.index()];
        p.preserve_viewport = false;
        if p.has_parent() && row == 0 {
            p.cursor = None;
            p.parent_cursor = true;
            if let Some(request) = &mut p.pending {
                request.cursor_touched = true;
            }
            return;
        }
        let Some(row) = row.checked_sub(p.parent_offset()) else {
            return;
        };
        if let Some(id) = p.display_entry(row).map(|e| e.id) {
            p.cursor = Some(id);
            p.parent_cursor = false;
            if let Some(request) = &mut p.pending {
                request.cursor_touched = true;
            }
            if toggle {
                let path = p
                    .display_entry(row)
                    .expect("validated display row")
                    .path
                    .clone();
                self.send_selection(pane, SelectionAction::Toggle(path), send);
            }
        }
    }
    pub(crate) fn move_cursor(&mut self, pane: PaneId, key: i32, send: &dyn Fn(Command) -> bool) {
        let p = &mut self.panes[pane.index()];
        p.preserve_viewport = false;
        let count = p.display_len() + p.parent_offset();
        if count == 0 {
            return;
        }
        if let Some(request) = &mut p.pending {
            request.cursor_touched = true;
        }
        let current = p.cursor_index().unwrap_or(0);
        let toggle_path = ((key == 8 || key == 9) && !p.parent_cursor)
            .then(|| {
                p.cursor
                    .and_then(|id| p.entry_by_id(id))
                    .map(|entry| entry.path.clone())
            })
            .flatten();
        let row = match key {
            2 => current.saturating_sub(1),
            3 | 9 => (current + 1).min(count - 1),
            4 => current.saturating_sub(20),
            5 => (current + 20).min(count - 1),
            6 => 0,
            7 => count - 1,
            _ => current,
        };
        if p.has_parent() && row == 0 {
            p.cursor = None;
            p.parent_cursor = true;
        } else {
            p.cursor = p
                .display_entry(row - p.parent_offset())
                .map(|entry| entry.id);
            p.parent_cursor = false;
        }
        if let Some(path) = toggle_path {
            self.send_selection(pane, SelectionAction::Toggle(path), send);
        }
    }
    pub(crate) fn sort(
        &mut self,
        pane: PaneId,
        column: SortColumn,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &mut self.panes[pane.index()];
        let old = p.sort;
        p.sort = SortSpec {
            column,
            descending: p.sort.column == column && !p.sort.descending,
        };
        let path = p.display_path().to_owned();
        if self.navigate_impl(pane, path, None, true, send) {
            true
        } else {
            self.panes[pane.index()].sort = old;
            false
        }
    }
    pub(crate) fn choose_volume(
        &mut self,
        pane: PaneId,
        volume: usize,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(volume) = self.volumes.get(volume) else {
            return false;
        };
        let path = self.panes[pane.index()]
            .volume_paths
            .get(&volume.root)
            .unwrap_or(&volume.root)
            .clone();
        self.navigate(pane, path, None, send)
    }
}

impl Pane {
    fn new(path: PathBuf) -> Self {
        Self {
            parent_cursor: parent_path(&path).is_some(),
            path,
            entries: Arc::from([]),
            cursor: None,
            folder_sizes: HashMap::new(),
            size_scan: None,
            size_scan_scope: None,
            size_scan_attempted: false,
            preserve_viewport: false,
            size_scan_generation: 0,
            selected: Arc::default(),
            selection_context: NavigationId(0),
            selection_committed_context: NavigationId(0),
            selection_revision: SelectionRevision(0),
            selection_applied: SelectionRevision(0),
            selection_resolved: SelectionRevision(0),
            selection_reset: true,
            selection_restore: None,
            selection_pending_bulk: false,
            selection_dirty: false,
            selection_rebase_pending: false,
            selection_deferred: false,
            history: vec![],
            history_index: None,
            error: String::new(),
            loading: false,
            sort: SortSpec::default(),
            space: None,
            streaming: vec![],
            metadata: HashMap::new(),
            pending: None,
            generation: 0,
            successful_generation: None,
            index: Arc::new(DirectoryIndex::from_entries(&[])),
            volume_paths: HashMap::new(),
            streaming_index: HashMap::new(),
            committed_sort: SortSpec::default(),
            search: search::SearchState::default(),
        }
    }

    pub(crate) fn displayed_generation(&self) -> NavigationId {
        if self.showing_stream() {
            NavigationId(self.generation)
        } else {
            self.successful_generation
                .unwrap_or(NavigationId(self.generation))
        }
    }

    pub(crate) fn entry_by_id(&self, id: EntryId) -> Option<&Entry> {
        let index = if self.showing_stream() {
            self.streaming_index.get(&id)
        } else {
            self.index.by_id.get(&id)
        }?;
        self.rows().get(*index)
    }
    pub(crate) fn has_parent(&self) -> bool {
        parent_path(self.display_path()).is_some()
    }
    pub(crate) fn parent_offset(&self) -> usize {
        usize::from(self.has_parent())
    }
    pub(crate) fn display_path(&self) -> &Path {
        self.pending
            .as_ref()
            .map_or(self.path.as_path(), |request| request.path.as_path())
    }
    pub(crate) fn visible_space(&self) -> Option<SpaceInfo> {
        self.pending
            .as_ref()
            .map_or(self.space, |request| request.space)
    }
    pub(crate) fn rows(&self) -> &[Entry] {
        if self.showing_stream() {
            &self.streaming
        } else {
            &self.entries
        }
    }
    pub(crate) fn cursor_index(&self) -> Option<usize> {
        if self.parent_cursor && self.has_parent() {
            return Some(0);
        }
        if self.search_view_waiting() {
            return None;
        }
        self.cursor
            .and_then(|id| {
                if self.showing_stream() {
                    self.streaming_index.get(&id)
                } else {
                    self.display_index().by_id.get(&id)
                }
            })
            .map(|i| *i + self.parent_offset())
    }
    pub(crate) fn showing_stream(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|request| !request.keep_rows)
    }
    fn retire_stream(&mut self, retired: &mut Vec<DeferredDrop>) {
        retired.push(Box::new(mem::take(&mut self.streaming)));
        retired.push(Box::new(mem::take(&mut self.streaming_index)));
        retired.push(Box::new(mem::take(&mut self.metadata)));
    }
}

fn parent_path(path: &Path) -> Option<&Path> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
}

/// Navigation uses logical parents: `link/..` goes to the link's lexical parent,
/// without resolving filesystem targets. Raw OsStr components remain lossless.
fn logical_path(path: PathBuf) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(result.components().next_back(), Some(Component::Normal(_))) {
                    result.pop();
                } else if !result.has_root() {
                    result.push(component.as_os_str());
                }
            }
            component => result.push(component.as_os_str()),
        }
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    result
}

#[cfg(test)]
#[path = "presenter/tests/mod.rs"]
mod tests;
