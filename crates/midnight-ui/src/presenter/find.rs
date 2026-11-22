//! Modeless Find presentation state; native work is dispatched through commands.
use super::*;
#[path = "find_actions.rs"]
mod actions;
#[path = "find_feed.rs"]
pub(crate) mod feed;
#[path = "find_selection.rs"]
pub(crate) mod result_selection;
#[path = "find_roots.rs"]
mod roots;
pub(crate) use actions::FileAction;
pub(crate) use result_selection::{FindCopy, FindSelect};
pub(crate) use roots::FindRoot;

pub(crate) struct FindForm {
    pub masks: String,
    pub name_mode: FilterMode,
    pub recursive: bool,
    pub name_case: bool,
    pub content: Option<FindContentQuery>,
    pub modified_within: Option<std::time::Duration>,
    pub duplicates: bool,
    /// Search inside archives.
    pub archives: bool,
}
impl Default for FindForm {
    fn default() -> Self {
        Self {
            masks: "*.*".into(),
            name_mode: FilterMode::Masks,
            recursive: true,
            name_case: false,
            content: None,
            modified_within: None,
            duplicates: false,
            archives: false,
        }
    }
}

#[derive(Default)]
pub(crate) struct FindState {
    pub sessions: HashMap<FindSessionId, FindSession>,
    // Window refresh runs under an immutable Presenter borrow. Retain replaced
    // row-order owners here until the normal backend reclamation drain, without
    // borrowing the whole Presenter mutably from a synchronous Slint callback.
    retired_orders: std::cell::RefCell<Vec<Arc<FindOrder>>>,
    next_session: u64,
    next_validation: u64,
    /// Go to file of an archive member: once that archive folder is listed, the
    /// cursor goes to the member with this raw name.
    archive_cursor: Option<(PaneId, NavigationId, Vec<u8>)>,
    /// Feed to panel: the search results each panel shows instead of its folder.
    pub(crate) feeds: [Option<feed::FeedPane>; 2],
    /// A viewer requested from search results in a panel, opened by the application.
    feed_viewer: Option<ViewerLaunch>,
    /// Identifies each delete captured from search results.
    feed_serial: u64,
    /// Rows a page key moves in each panel's result list, as last reported by the
    /// window (0: not reported yet). The list exists while hidden, so it reports
    /// before a feed opens.
    pub(crate) feed_pages: [usize; 2],
}
pub(crate) struct FindSession {
    pub revision: u64,
    pub origin: PaneId,
    /// Every listed folder keeps its captured native path; unticked ones are not searched.
    pub roots: Vec<FindRoot>,
    /// The latest accepted Add, counted, with the row of its folder (new or listed).
    pub added_root: (u64, usize),
    pub generation: FindGeneration,
    pub results: Vec<Arc<FindResult>>,
    pub order: Option<Arc<FindOrder>>,
    pub sort: FindSortSpec,
    pub sort_revision: u64,
    pub current_directory: Option<Arc<PathBuf>>,
    pub running: bool,
    pub stopping: bool,
    pub duplicates: bool,
    pub progress: FindProgress,
    pub summary: Option<FindSummary>,
    pub error: String,
    /// The selected results, the cursor and the anchor, by result identity (slot = ID
    /// minus one). A new search clears it; streamed results and sorting keep it.
    pub selection: crate::list_selection::ListSelection,
    /// Changes with every change of the selection or the cursor.
    pub selection_revision: u64,
    /// The bytes of the selected files, for the status line.
    pub selection_bytes: u64,
    pending: Option<FindValidationRequest>,
    pending_file: Option<actions::PendingFile>,
    /// The native folder chooser opened by Add, bound to this session.
    picker: Option<DesktopRequestId>,
    file_cancel: Cancellation,
    viewer: Option<ViewerLaunch>,
}

impl FindState {
    pub(super) fn take_retired_orders(&mut self) -> Vec<Arc<FindOrder>> {
        std::mem::take(self.retired_orders.get_mut())
    }

    fn session_mut(&mut self, id: &FindSessionId) -> Option<&mut FindSession> {
        self.session_mut_if(id, |_| true)
    }

    /// Only an accepted event marks its session changed; a stale backend tail
    /// must not force a full Find-window refresh.
    fn session_mut_if(
        &mut self,
        id: &FindSessionId,
        accept: impl FnOnce(&FindSession) -> bool,
    ) -> Option<&mut FindSession> {
        let session = self
            .sessions
            .get_mut(id)
            .filter(|session| accept(session))?;
        session.revision = session.revision.wrapping_add(1);
        Some(session)
    }
}

impl Presenter {
    pub(crate) fn retire_find_order(&self, order: Arc<FindOrder>) {
        self.finds.retired_orders.borrow_mut().push(order);
    }
    pub(crate) fn create_find(&mut self, pane: PaneId) -> Option<FindSessionId> {
        if self.finds.sessions.len() >= MAX_FIND_SESSIONS {
            self.desktop.report(
                "Find files",
                format!("Close one of the {MAX_FIND_SESSIONS} open Find windows first."),
            );
            return None;
        }
        if !self.panes[pane.index()].native_capabilities() {
            self.desktop.report("Find files", "Recursive search is available in native folders.\nArchive search is not available yet.".into());
            return None;
        }
        let id = FindSessionId(self.finds.next_session.checked_add(1)?);
        self.finds.next_session = id.0;
        self.finds.sessions.insert(
            id,
            FindSession {
                revision: 0,
                origin: pane,
                roots: vec![FindRoot::new(
                    self.panes[pane.index()].display_path().to_owned(),
                )],
                added_root: (0, 0),
                generation: FindGeneration(0),
                results: vec![],
                order: None,
                sort: FindSortSpec::default(),
                sort_revision: 0,
                current_directory: None,
                running: false,
                stopping: false,
                duplicates: false,
                progress: FindProgress::default(),
                summary: None,
                error: String::new(),
                selection: Default::default(),
                selection_revision: 0,
                selection_bytes: 0,
                pending: None,
                pending_file: None,
                picker: None,
                file_cancel: Cancellation::default(),
                viewer: None,
            },
        );
        Some(id)
    }

    pub(crate) fn start_find(
        &mut self,
        id: FindSessionId,
        form: FindForm,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(s) = self.finds.session_mut(&id) else {
            return false;
        };
        let Some(generation) = s.generation.0.checked_add(1).map(FindGeneration) else {
            return false;
        };
        let roots: Vec<_> = s
            .roots
            .iter()
            .filter(|root| root.searched)
            .map(|root| root.path.clone())
            .collect();
        if roots.is_empty() && !s.roots.is_empty() {
            s.error = "Tick at least one search folder.".into();
            return false;
        }
        let mut request = FindRequest::new(id, generation, roots);
        request.masks = form.masks;
        request.name_mode = form.name_mode;
        request.sort = s.sort;
        request.recursive = form.recursive;
        request.name_case_sensitive = form.name_case;
        request.content = form.content;
        request.modified_within = form.modified_within;
        request.duplicates = form.duplicates;
        request.archives = form.archives && !form.duplicates;
        if let Err(error) = request.validate() {
            s.error = error.into();
            return false;
        }
        if !send(Command::StartFind(Box::new(request))) {
            s.error = "Search queue is busy. Try again.".into();
            return false;
        }
        s.file_cancel.cancel();
        s.file_cancel = Cancellation::default();
        self.retired.push(Box::new(s.pending_file.take()));
        self.retired.push(Box::new(s.viewer.take()));
        s.generation = generation;
        s.running = true;
        s.duplicates = form.duplicates;
        s.stopping = false;
        s.error.clear();
        s.pending = None;
        s.progress = FindProgress::default();
        s.current_directory = None;
        s.sort_revision = 0;
        s.reset_selection();
        self.retired.push(Box::new(s.order.take()));
        self.retired.push(Box::new(mem::take(&mut s.results)));
        self.retired.push(Box::new(s.summary.take()));
        true
    }

    pub(crate) fn sort_find(
        &mut self,
        id: FindSessionId,
        column: FindSortColumn,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(s) = self.finds.session_mut(&id) else {
            return;
        };
        let Some(revision) = s.sort_revision.checked_add(1) else {
            return;
        };
        let sort = FindSortSpec {
            column,
            descending: s.sort.column == column && !s.sort.descending,
        };
        if send(Command::SortFind(FindSortRequest {
            session: id,
            generation: s.generation,
            revision,
            sort,
        })) {
            s.sort_revision = revision;
            s.sort = sort;
        } else {
            s.error = "Sort queue is busy. Try again.".into();
        }
    }

    pub(crate) fn stop_find(&mut self, id: FindSessionId, send: &dyn Fn(Command) -> bool) {
        let Some(s) = self.finds.session_mut(&id) else {
            return;
        };
        if !s.running || s.stopping {
            return;
        }
        if send(Command::CancelFind {
            session: id,
            generation: s.generation,
        }) {
            s.stopping = true;
            self.retired.push(Box::new(s.current_directory.take()));
            s.error.clear();
        } else {
            s.error = "Cancellation queue is busy. Try Stop again.".into();
        }
    }

    pub(crate) fn close_find(&mut self, id: FindSessionId, send: &dyn Fn(Command) -> bool) -> bool {
        let Some(s) = self.finds.session_mut(&id) else {
            return true;
        };
        // A chooser still open for this window must not add a folder after it closes.
        if let Some(request) = s.picker {
            if !send(Command::CancelPathPicker { request }) {
                s.error = "Request queue is busy. Try closing again.".into();
                return false;
            }
            s.picker = None;
        }
        if s.generation.0 > 0 && !send(Command::CloseFind(id)) {
            s.error = "Request queue is busy. Try closing again.".into();
            return false;
        }
        s.file_cancel.cancel();
        self.retired.push(Box::new(self.finds.sessions.remove(&id)));
        true
    }

    pub(crate) fn reveal_find(
        &mut self,
        id: FindSessionId,
        row_token: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(token) = self.finds.next_validation.checked_add(1) else {
            return false;
        };
        self.finds.next_validation = token;
        let Some(s) = self.finds.session_mut(&id) else {
            return false;
        };
        let Some((generation, result_id)) = row_token
            .split_once(':')
            .and_then(|(g, r)| Some((g.parse::<u64>().ok()?, r.parse::<u64>().ok()?)))
        else {
            return false;
        };
        if generation != s.generation.0 {
            return false;
        }
        let Some(result) = s.results.iter().find(|r| r.id.0 == result_id) else {
            return false;
        };
        let request = FindValidationRequest {
            session: id,
            generation: s.generation,
            result: result.id,
            token,
        };
        if !send(Command::ValidateFindResult(request)) {
            s.error = "Result validation queue is busy. Try again.".into();
            return false;
        }
        s.pending = Some(request);
        s.error.clear();
        true
    }

    pub(super) fn find_validated(
        &mut self,
        result: FindValidationResult,
        send: &dyn Fn(Command) -> bool,
    ) {
        let request = result.request;
        let Some(s) = self.finds.session_mut_if(&request.session, |s| {
            s.generation == request.generation
                && s.pending
                    .is_some_and(|p| p.token == request.token && p.result == request.result)
        }) else {
            self.retired.push(Box::new(result));
            return;
        };
        s.pending = None;
        let entry = match result.result {
            Ok(entry) => entry,
            Err(error) => {
                s.error = crate::error_presentation::describe(&error);
                return;
            }
        };
        let Some(found) = s
            .results
            .iter()
            .find(|r| r.id == request.result && entry.matches_capture(&r.entry))
        else {
            s.error = "Search result changed. Run the search again.".into();
            return;
        };
        if let Some(member) = found.archive.clone() {
            let pane = s.origin;
            self.reveal_archive_member(request.session, pane, &member, send);
            return;
        }
        let Some(parent) = entry.path.parent().map(Path::to_owned) else {
            return;
        };
        let pane = s.origin;
        // Navigation is read-only. This event is never used to execute, edit or delete a path.
        if self.navigate(pane, parent, None, send) {
            self.active = pane;
            if let Some(pending) = &mut self.panes[pane.index()].pending {
                pending.wanted_cursor = Some(entry.path.clone());
            }
        } else if let Some(s) = self.finds.session_mut(&request.session) {
            s.error = "Panel navigation queue is busy. Try Go to file again.".into();
        }
    }

    /// Opens the archive folder that holds `member` in `pane`; the cursor goes to the
    /// member when that listing arrives. The provider address, not a displayed
    /// path, names the folder, and the archive must still be the searched revision.
    fn reveal_archive_member(
        &mut self,
        session: FindSessionId,
        pane: PaneId,
        member: &FindArchiveMember,
        send: &dyn Fn(Command) -> bool,
    ) {
        let error = if let Err(error) = member.folder.validate() {
            Some(error.to_owned())
        } else if self.archive_navigate(
            pane,
            ArchiveAction::List(ArchiveLocation::unavailable(member.folder.clone())),
            send,
        ) {
            self.active = pane;
            let navigation = NavigationId(self.panes[pane.index()].generation);
            self.retired
                .push(Box::new(self.finds.archive_cursor.replace((
                    pane,
                    navigation,
                    member.name.clone(),
                ))));
            None
        } else {
            Some("Panel navigation queue is busy. Try Go to file again.".to_owned())
        };
        if let (Some(error), Some(s)) = (error, self.finds.session_mut(&session)) {
            s.error = error;
        }
    }

    /// After an event: search results shown in panels, and a revealed archive member.
    pub(super) fn flush_find_panels(&mut self) {
        self.flush_feeds();
        self.flush_find_archive_cursor();
    }

    /// Put the cursor on a revealed archive member once its folder is listed. A
    /// later navigation of that panel forgets it.
    fn flush_find_archive_cursor(&mut self) {
        let Some((pane, navigation, _)) = &self.finds.archive_cursor else {
            return;
        };
        let p = &mut self.panes[pane.index()];
        if p.generation == navigation.0 && p.loading {
            return;
        }
        let Some((_, navigation, name)) = self.finds.archive_cursor.take() else {
            return;
        };
        if p.generation == navigation.0
            && p.successful_generation == Some(navigation)
            && p.archive.is_some()
            && let Some(entry) = p.entries.iter().find(|entry| {
                entry
                    .virtual_node
                    .as_ref()
                    .is_some_and(|node| *node.raw_name == *name)
            })
        {
            p.cursor = Some(entry.id);
            p.parent_cursor = false;
        }
    }

    pub(super) fn apply_find(&mut self, event: FindEvent) {
        let Some(s) = self
            .finds
            .session_mut_if(&event.session, |s| s.generation == event.generation)
        else {
            self.retired.push(Box::new(event));
            return;
        };
        match event.update {
            FindUpdate::Started => {}
            FindUpdate::Batch(batch) => {
                // Ordinary batches contain at most 64 rows. A verified duplicate
                // group is atomic and can occupy the complete retained-row budget.
                let remaining = FindLimits::default()
                    .max_results
                    .saturating_sub(s.results.len());
                if batch.len() <= remaining {
                    s.results.extend(batch.iter().cloned());
                } else {
                    s.error = "Search result batch exceeded the retained result limit.".into();
                }
                self.retired.push(Box::new(batch));
            }
            FindUpdate::Progress(progress) => s.progress = progress,
            FindUpdate::CurrentDirectory(path) => {
                if s.running && !s.stopping {
                    self.retired
                        .push(Box::new(s.current_directory.replace(path)));
                } else {
                    self.retired.push(Box::new(path));
                }
            }
            FindUpdate::Sorted(order) => {
                if order.revision == s.sort_revision && order.ids.len() == s.results.len() {
                    self.retired.push(Box::new(s.order.replace(order)));
                } else {
                    self.retired.push(Box::new(order));
                }
            }
            FindUpdate::SortFailed { revision, error } => {
                if revision == s.sort_revision {
                    s.error = error.to_string();
                }
            }
            FindUpdate::Finished(summary) => {
                s.running = false;
                s.stopping = false;
                s.progress = summary.progress.clone();
                self.retired.push(Box::new(s.current_directory.take()));
                self.retired.push(Box::new(s.summary.replace(summary)));
            }
        }
    }

    pub(super) fn reject_find(&mut self, rejection: FindRejected) {
        if let Some(s) = self
            .finds
            .session_mut_if(&rejection.session, |s| s.generation == rejection.generation)
        {
            s.running = false;
            s.stopping = false;
            s.error = crate::error_presentation::describe(&rejection.error);
        }
        self.retired.push(Box::new(rejection));
    }
}
