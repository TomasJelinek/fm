use midnight_core::*;
mod archives;
mod bookmarks;
pub(crate) mod checksums;
#[cfg(test)]
mod checksums_tests;
mod choices;
mod comparison;
mod copies;
mod copy_dialog;
mod desktop;
pub(crate) mod drag_drop;
mod editor;
mod entry_sizes;
mod file_actions;
mod file_compare;
mod filename_colors;
mod filter;
pub(crate) mod find;
#[cfg(test)]
mod find_tests;
#[cfg(all(test, unix))]
mod find_unix_tests;
#[cfg(all(test, windows))]
mod find_windows_tests;
mod fonts;
pub(crate) mod keyboard;
pub(crate) mod keyboard_editor;
mod metadata;
mod operations;
mod panel_details;
mod panel_navigation;
mod preferences;
mod search;
mod selection;
mod selection_actions;
mod selection_preview;
pub(crate) mod selection_rectangle;
mod selection_sizes;
mod shortcuts;
mod tabs;
mod terminal;
pub(crate) mod user_command_settings;
#[cfg(test)]
mod user_command_settings_tests;
mod user_commands;
mod viewer_plugins;
mod volumes;
pub(crate) use file_actions::MenuAnchor;
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
    storage_kind: Option<StorageKind>,
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
    selection: Option<Arc<SelectionSnapshot>>,
}

impl SizeScanScope {
    fn contains(&self, path: &Path) -> bool {
        self.index
            .by_path
            .get(path)
            .and_then(|id| self.index.by_id.get(id))
            .and_then(|index| self.entries.get(*index))
            .is_some_and(|entry| entry.kind == EntryKind::Directory)
            && self
                .selection
                .as_ref()
                .is_none_or(|selection| selection.contains_path(path))
    }
}

pub(crate) struct Pane {
    archive: Option<archives::ArchivePane>,
    archive_request: bool,
    /// Last successful location. Provisional rows belong to `display_path()`.
    pub(crate) path: PathBuf,
    pub(crate) entries: Arc<[Entry]>,
    pub(crate) cursor: Option<EntryId>,
    pub(crate) parent_cursor: bool,
    pub(crate) folder_sizes: HashMap<PathBuf, FolderSize>,
    pub(crate) size_scan: Option<FolderSizeScanId>,
    pub(crate) size_scan_scope: Option<SizeScanScope>,
    pub(crate) size_scan_attempted: bool,
    selection_sizes: selection_sizes::State,
    entry_size_scans: Vec<entry_sizes::EntrySizeScope>,
    pub(crate) preserve_viewport: bool,
    size_scan_generation: u64,
    pub(crate) selected: Arc<SelectionSnapshot>,
    selection_preview: Vec<selection_preview::PendingSelection>,
    rectangle_preview: Option<selection_rectangle::Preview>,
    selection_anchor: Option<PathBuf>,
    selection_anchor_id: Option<EntryId>,
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
    pub(crate) storage_kind: StorageKind,
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
    archive_formats: Vec<ArchiveFormat>,
    pub(crate) panes: [Pane; 2],
    pub(crate) volumes: Vec<Volume>,
    pub(crate) volumes_revision: i32,
    pub(crate) volume_details: HashMap<String, (PathBuf, VolumeDetails)>,
    pub(crate) volumes_error: String,
    pub(crate) active: PaneId,
    pub(crate) desktop: desktop::DesktopState,
    pub(crate) thumbnails: crate::thumbnail_presentation::ThumbnailState,
    pub(crate) preferences: preferences::PreferencesState,
    pub(crate) editor: editor::EditorState,
    pub(crate) viewer_plugins: viewer_plugins::ViewerPluginsState,
    pub(crate) fonts: fonts::FontsState,
    pub(crate) keyboard: keyboard::KeyboardState,
    pub(crate) operations: operations::OperationsState,
    pub(crate) metadata: metadata::MetadataState,
    pub(crate) comparison: comparison::ComparisonState,
    pub(crate) filter_dialog: filter::FilterDialogState,
    pub(crate) colors: filename_colors::ColorState,
    pub(crate) copies: copies::CopiesState,
    pub(crate) finds: find::FindState,
    pub(crate) checksums: checksums::ChecksumState,
    pub(crate) command_launches: user_commands::CommandLaunches,
    pub(crate) user_command_settings: user_command_settings::State,
    pub(crate) command_toolbar: crate::user_command_presentation::ToolbarState,
    pub(crate) number_locale: NumberLocale,
    tabs: tabs::TabsState,
    choices: choices::ChoicesState,
    selection_actions: [Option<selection_actions::PendingSelectionAction>; 2],
    file_actions: file_actions::FileActionsState,
    pub(crate) drag_drop: drag_drop::DragDropState,
    search_revisions: [SearchRevision; 2],
    search_cancellations: [Option<SearchRevision>; 2],
    retired: Vec<DeferredDrop>,
}

impl Presenter {
    /// Create initial pane locations without sending commands or performing filesystem work.
    pub fn new(left: PathBuf, right: PathBuf) -> Self {
        Self {
            archive_formats: Vec::new(),
            tabs: tabs::TabsState::new(),
            choices: choices::ChoicesState::default(),
            selection_actions: [None, None],
            file_actions: file_actions::FileActionsState::default(),
            drag_drop: drag_drop::DragDropState::default(),
            search_revisions: [SearchRevision(0); 2],
            search_cancellations: [None, None],
            panes: [Pane::new(left), Pane::new(right)],
            volumes: vec![],
            volumes_revision: 0,
            volume_details: HashMap::new(),
            volumes_error: String::new(),
            active: PaneId::Left,
            desktop: desktop::DesktopState::default(),
            thumbnails: crate::thumbnail_presentation::ThumbnailState::default(),
            preferences: preferences::PreferencesState::default(),
            editor: editor::EditorState::default(),
            viewer_plugins: viewer_plugins::ViewerPluginsState::default(),
            fonts: fonts::FontsState::default(),
            keyboard: keyboard::KeyboardState::default(),
            operations: operations::OperationsState::default(),
            metadata: metadata::MetadataState::default(),
            comparison: comparison::ComparisonState::default(),
            filter_dialog: filter::FilterDialogState::default(),
            colors: filename_colors::ColorState::default(),
            copies: copies::CopiesState::default(),
            finds: find::FindState::default(),
            checksums: checksums::ChecksumState::default(),
            command_launches: user_commands::CommandLaunches::default(),
            user_command_settings: user_command_settings::State::default(),
            command_toolbar: crate::user_command_presentation::ToolbarState::default(),
            number_locale: NumberLocale::default(),
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
        if p.loading || p.archive.is_some() {
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
            .push(Box::new(mem::take(&mut self.command_launches.jobs)));
        self.retired
            .push(Box::new(mem::take(&mut self.finds.sessions)));
        self.retired
            .push(Box::new(mem::take(&mut self.checksums.sessions)));
        self.retire_copy_draft();
        self.retire_metadata();
        self.retire_drag_drop();
        self.retired
            .push(Box::new(mem::take(&mut self.choices.items)));
        self.retire_selection_actions();
        self.retire_file_actions();
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
            self.retired
                .push(Box::new(mem::take(&mut p.entry_size_scans)));
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
        drag_drop::invalidate(&self.drag_drop.targets);
        if self
            .drag_drop
            .lifecycle
            .active
            .is_some_and(|g| g.source.pane == pane.index())
        {
            self.cancel_file_drag(send);
        }
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
        } else if p.archive.is_none() && path == p.path && p.successful_generation.is_some() {
            Command::Refresh {
                pane,
                navigation,
                path: path.clone(),
                sort: p.sort,
                scan: p
                    .size_scan
                    .or_else(|| p.entry_size_scans.last().map(|scope| scope.scan)),
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
        p.archive_request = false;
        self.navigation_accepted_impl(pane, navigation, path, history, resort, send);
        self.tab_navigation_accepted(pane, send);
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
            self.retired
                .push(Box::new(mem::take(&mut p.entry_size_scans)));
        }
        if !same_location || (!resort && p.size_scan.is_none() && p.entry_size_scans.is_empty()) {
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
            storage_kind: None,
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
        if let Event::FindResultValidated(result) = event {
            self.find_validated(*result, send);
            return;
        }
        let Some(event) = self.dispatch_drag_event(event, send) else {
            return;
        };
        let Some(event) = self.dispatch_file_action_event(event, send) else {
            return;
        };
        if matches!(&event, Event::NetworkConnectionsFinished(Ok(()))) {
            let _ = send(Command::ListVolumes);
        }
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
        self.dispatch_selection_actions(&event, send);
        self.apply(event);
        self.refresh_operation_paths(send);
        if let Some(pane) = committed {
            self.panes[pane.index()].selection_rebase_pending = true;
        }
        self.flush_filters(send);
        self.flush_filename_colors(send);
        self.flush_search(send);
        self.flush_selection(send);
        self.flush_drag_commands(send);
        self.flush_comparison(send);
        self.dispatch_open_directories(send);
        self.flush_loaded_preferences(send);
    }

    /// Apply a backend event, retiring stale payloads and preserving the last usable location.
    pub fn apply(&mut self, event: Event) {
        if let Event::Checksum(event) = event {
            self.apply_checksum(*event);
            return;
        }
        if let Event::ChecksumRejected(event) = event {
            self.reject_checksum(*event);
            return;
        }
        if let Event::ChecksumManifest(event) = event {
            self.apply_checksum_manifest(*event);
            return;
        }
        if let Event::Find(event) = event {
            self.apply_find(*event);
            return;
        }
        if let Event::FindRejected(rejection) = event {
            self.reject_find(*rejection);
            return;
        }
        if matches!(&event, Event::FindResultValidated(_)) {
            // A validated navigation needs the sender; retire unhandled payloads.
            self.retired.push(Box::new(event));
            return;
        }
        if matches!(
            event,
            Event::ClipboardFilesRead { .. } | Event::ShellMenuFinished { .. } | Event::DragDrop(_)
        ) {
            // These actions require the sender; a plain apply must neither
            // dispatch an action nor destroy an owned clipboard list on UI.
            self.retired.push(Box::new(event));
            return;
        }
        if let Event::LocaleLoaded(result) = event {
            match result {
                Ok(locale) => self.number_locale = locale,
                Err(error) => self.desktop.report(
                    "Regional format unavailable",
                    crate::error_presentation::describe(&error),
                ),
            }
            return;
        }
        if let Event::NetworkConnectionsFinished(result) = event {
            if let Err(error) = result {
                self.desktop.report(
                    "Network connections",
                    crate::error_presentation::describe(&error),
                );
            }
            return;
        }
        let Some(event) = self.apply_comparison_event(event) else {
            return;
        };
        let Some(event) = self.apply_colors_event(event) else {
            return;
        };
        let Some(event) = self.apply_filter_event(event) else {
            return;
        };
        let Some(event) = self.apply_copy_event(event) else {
            return;
        };
        let Some(event) = self.apply_user_command_event(event) else {
            return;
        };
        let Some(event) = self.apply_operation(event) else {
            return;
        };
        let Some(event) = self.apply_entry_size_event(event) else {
            return;
        };
        let Some(event) = self.apply_search_event(event) else {
            return;
        };
        let Some(event) = self.apply_selection_event(event) else {
            return;
        };
        let Some(event) = self.apply_selection_sizes(event) else {
            return;
        };
        let Some(event) = self.apply_desktop(event) else {
            return;
        };
        let Some(event) = self.apply_archive_event(event) else {
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
                    self.volumes_error = crate::error_presentation::describe(&error);
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
            }
            | Event::StorageKindUpdated {
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
            self.cancel_selection_actions(pane);
        }
        let p = &mut self.panes[pane.index()];
        if navigation.0 != p.generation
            || (!matches!(
                &event,
                Event::SpaceUpdated { .. } | Event::StorageKindUpdated { .. }
            ) && p.pending.is_none())
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
                self.retired.push(Box::new(p.archive.take()));
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
                p.storage_kind = request.storage_kind.unwrap_or(if same_location {
                    p.storage_kind
                } else {
                    StorageKind::Unknown
                });
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
                    p.selection_preview.clear();
                    p.selection_deferred = false;
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
                p.error = crate::error_presentation::describe(&error);
            }
            Event::SpaceUpdated { space, .. } => {
                if let Some(request) = &mut p.pending {
                    request.space = Some(space);
                } else if p.successful_generation == Some(navigation) {
                    p.space = Some(space);
                }
            }
            Event::StorageKindUpdated { kind, .. } => {
                if let Some(request) = &mut p.pending {
                    request.storage_kind = Some(kind);
                } else if p.successful_generation == Some(navigation) {
                    p.storage_kind = kind;
                }
            }
            _ => unreachable!(),
        }
    }

    pub(crate) fn up(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) -> bool {
        if self.panes[pane.index()].archive.is_some() {
            return self.archive_up(pane, send);
        }
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
        if self.panes[pane.index()].archive.is_some() {
            return self.archive_history(pane, delta, send);
        }
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
        if self.panes[pane.index()].archive.is_some() {
            if text == ".." {
                return self.archive_up(pane, send);
            }
            let path = PathBuf::from(text);
            if path.is_absolute() {
                return self.navigate(pane, path, None, send);
            }
            self.desktop.report("Archive path", "Navigate archive folders through their rows or parent entry; display text is not a native path.".into());
            return false;
        }
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
        if self.panes[pane.index()].loading {
            return false;
        }
        if self.panes[pane.index()].archive.is_some() {
            return self.archive_open(pane, row, send);
        }
        if self.panes[pane.index()].has_parent() && row == 0 {
            return self.up(pane, send);
        }
        let Some(row) = row.checked_sub(self.panes[pane.index()].parent_offset()) else {
            return false;
        };
        let entry = self.panes[pane.index()].display_entry(row).map(|entry| {
            (
                entry.kind,
                entry.path.clone(),
                self.can_browse_archive(entry),
            )
        });
        match entry {
            Some((EntryKind::Directory | EntryKind::DirectoryLink, path, _)) => {
                self.navigate(pane, path, None, send)
            }
            Some((EntryKind::File | EntryKind::Symlink, path, archive)) => {
                if archive {
                    return self.archive_navigate(pane, ArchiveAction::Open { path }, send);
                }
                self.open_native_entry(pane, path, send)
            }
            Some((EntryKind::Other, _, _)) => {
                self.desktop.report(
                    "Cannot open this entry",
                    "Only folders, regular files and links to them can be opened.".into(),
                );
                false
            }
            None => false,
        }
    }
    pub fn select_row(
        &mut self,
        pane: PaneId,
        row: usize,
        toggle: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.select_row_modified(pane, row, toggle, false, send);
    }
    pub(crate) fn move_cursor(&mut self, pane: PaneId, key: i32, send: &dyn Fn(Command) -> bool) {
        if self.panes[pane.index()].archive.is_some() && matches!(key, 8 | 9) {
            let row = self.panes[pane.index()].cursor_index().unwrap_or(0);
            self.archive_select_row(pane, row, true, false, send);
            let last = self.panes[pane.index()].display_len();
            self.focus_display_row(pane, row.saturating_add(1).min(last));
            return;
        }
        let p = &self.panes[pane.index()];
        if matches!(key, 8 | 9) && !p.search_allows_activation() {
            return;
        }
        let count = p.display_len() + p.parent_offset();
        if count == 0 {
            return;
        }
        let current = p.cursor_index().unwrap_or(0);
        let toggle = ((key == 8 || key == 9) && !p.parent_cursor)
            .then(|| {
                p.cursor
                    .and_then(|id| p.entry_by_id(id))
                    .map(|entry| (entry.path.clone(), entry.id))
            })
            .flatten();
        if let Some((path, entry)) = toggle {
            if !self.send_selection(pane, SelectionAction::Toggle(path), send) {
                return;
            }
            if key == 8 {
                self.measure_entry(pane, entry, send);
            }
        }
        let row = match key {
            2 => current.saturating_sub(1),
            3 | 8 | 9 => (current + 1).min(count - 1),
            4 => current.saturating_sub(20),
            5 => (current + 20).min(count - 1),
            6 => 0,
            7 => count - 1,
            _ => current,
        };
        self.focus_display_row(pane, row);
    }

    pub(crate) fn focus_display_row(&mut self, pane: PaneId, row: usize) {
        let p = &mut self.panes[pane.index()];
        let count = p.display_len() + p.parent_offset();
        if count == 0 {
            return;
        }
        let row = row.min(count - 1);
        p.preserve_viewport = false;
        if let Some(request) = &mut p.pending {
            request.cursor_touched = true;
        }
        if p.has_parent() && row == 0 {
            p.cursor = None;
            p.parent_cursor = true;
        } else {
            p.cursor = p
                .display_entry(row - p.parent_offset())
                .map(|entry| entry.id);
            p.parent_cursor = false;
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
        if let Some(archive) = &p.archive {
            let location = archive.location.clone();
            if self.archive_navigate(pane, ArchiveAction::List(location), send) {
                return true;
            }
            self.panes[pane.index()].sort = old;
            return false;
        }
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
            archive: None,
            archive_request: false,
            parent_cursor: parent_path(&path).is_some(),
            path,
            entries: Arc::from([]),
            cursor: None,
            folder_sizes: HashMap::new(),
            size_scan: None,
            size_scan_scope: None,
            size_scan_attempted: false,
            entry_size_scans: Vec::new(),
            preserve_viewport: false,
            size_scan_generation: 0,
            selection_sizes: selection_sizes::State::default(),
            selected: Arc::default(),
            selection_preview: Vec::new(),
            rectangle_preview: None,
            selection_anchor: None,
            selection_anchor_id: None,
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
            storage_kind: StorageKind::Unknown,
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
        self.archive.is_some() || parent_path(self.display_path()).is_some()
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
    pub(crate) fn visible_storage_kind(&self) -> StorageKind {
        self.pending.as_ref().map_or(self.storage_kind, |request| {
            request
                .storage_kind
                .unwrap_or(if request.path == self.path {
                    self.storage_kind
                } else {
                    StorageKind::Unknown
                })
        })
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
