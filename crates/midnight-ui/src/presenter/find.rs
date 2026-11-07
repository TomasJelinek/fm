//! Modeless Find presentation state; native work is dispatched through commands.
use super::*;

pub(crate) struct FindForm {
    pub masks: String,
    pub recursive: bool,
    pub name_case: bool,
    pub content: Option<FindContentQuery>,
}
impl Default for FindForm {
    fn default() -> Self {
        Self {
            masks: "*.*".into(),
            recursive: true,
            name_case: false,
            content: None,
        }
    }
}

#[derive(Default)]
pub(crate) struct FindState {
    pub sessions: HashMap<FindSessionId, FindSession>,
    next_session: u64,
    next_validation: u64,
}
pub(crate) struct FindSession {
    pub revision: u64,
    pub origin: PaneId,
    pub roots: Vec<PathBuf>,
    pub generation: FindGeneration,
    pub results: Vec<Arc<FindResult>>,
    pub running: bool,
    pub stopping: bool,
    pub progress: FindProgress,
    pub summary: Option<FindSummary>,
    pub error: String,
    pending: Option<FindValidationRequest>,
}

impl FindState {
    fn session_mut(&mut self, id: &FindSessionId) -> Option<&mut FindSession> {
        let session = self.sessions.get_mut(id)?;
        session.revision = session.revision.wrapping_add(1);
        Some(session)
    }
}

impl Presenter {
    pub(crate) fn create_find(&mut self, pane: PaneId) -> Option<FindSessionId> {
        if self.finds.sessions.len() >= MAX_FIND_SESSIONS {
            self.desktop.report(
                "Find files",
                "Close one of the four Find windows first.".into(),
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
                roots: vec![self.panes[pane.index()].display_path().to_owned()],
                generation: FindGeneration(0),
                results: vec![],
                running: false,
                stopping: false,
                progress: FindProgress::default(),
                summary: None,
                error: String::new(),
                pending: None,
            },
        );
        Some(id)
    }

    pub(crate) fn add_find_root(&mut self, id: FindSessionId, text: &str) {
        let Some(s) = self.finds.session_mut(&id) else {
            return;
        };
        if s.running {
            return;
        }
        let path = PathBuf::from(text);
        let mut roots = s.roots.clone();
        if !roots.contains(&path) {
            roots.push(path);
        }
        let request = FindRequest::new(id, s.generation, roots);
        match request.validate() {
            Ok(()) => {
                s.roots = request.roots;
                s.error.clear();
            }
            Err(error) => s.error = error.into(),
        }
    }

    pub(crate) fn remove_find_root(&mut self, id: FindSessionId, row: i32) {
        let Some(s) = self.finds.session_mut(&id) else {
            return;
        };
        if !s.running
            && let Ok(row) = usize::try_from(row)
            && row < s.roots.len()
        {
            s.roots.remove(row);
        }
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
        let mut request = FindRequest::new(id, generation, s.roots.clone());
        request.masks = form.masks;
        request.recursive = form.recursive;
        request.name_case_sensitive = form.name_case;
        request.content = form.content;
        if let Err(error) = request.validate() {
            s.error = error.into();
            return false;
        }
        if !send(Command::StartFind(Box::new(request))) {
            s.error = "Search queue is busy. Try again.".into();
            return false;
        }
        s.generation = generation;
        s.running = true;
        s.stopping = false;
        s.error.clear();
        s.pending = None;
        s.progress = FindProgress::default();
        self.retired.push(Box::new(mem::take(&mut s.results)));
        self.retired.push(Box::new(s.summary.take()));
        true
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
            s.error.clear();
        } else {
            s.error = "Cancellation queue is busy. Try Stop again.".into();
        }
    }

    pub(crate) fn close_find(&mut self, id: FindSessionId, send: &dyn Fn(Command) -> bool) -> bool {
        let Some(s) = self.finds.session_mut(&id) else {
            return true;
        };
        if s.generation.0 > 0 && !send(Command::CloseFind(id)) {
            s.error = "Request queue is busy. Try closing again.".into();
            return false;
        }
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
        let Some(s) = self.finds.session_mut(&request.session) else {
            self.retired.push(Box::new(result));
            return;
        };
        if s.generation != request.generation
            || s.pending
                .is_none_or(|p| p.token != request.token || p.result != request.result)
        {
            self.retired.push(Box::new(result));
            return;
        }
        s.pending = None;
        let entry = match result.result {
            Ok(entry) => entry,
            Err(error) => {
                s.error = crate::error_presentation::describe(&error);
                return;
            }
        };
        if !s
            .results
            .iter()
            .any(|r| r.id == request.result && entry.matches_capture(&r.entry))
        {
            s.error = "Search result changed. Run the search again.".into();
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

    pub(super) fn apply_find(&mut self, event: FindEvent) {
        let Some(s) = self
            .finds
            .session_mut(&event.session)
            .filter(|s| s.generation == event.generation)
        else {
            self.retired.push(Box::new(event));
            return;
        };
        match event.update {
            FindUpdate::Started => {}
            FindUpdate::Batch(batch) => {
                // Both boundaries enforce the protocol budget; at most 64 shared rows per event.
                let remaining = FindLimits::default()
                    .max_results
                    .saturating_sub(s.results.len());
                s.results.extend(batch.iter().take(remaining).cloned());
                self.retired.push(Box::new(batch));
            }
            FindUpdate::Progress(progress) => s.progress = progress,
            FindUpdate::Finished(summary) => {
                s.running = false;
                s.stopping = false;
                s.progress = summary.progress.clone();
                self.retired.push(Box::new(s.summary.replace(summary)));
            }
        }
    }

    pub(super) fn reject_find(&mut self, rejection: FindRejected) {
        if let Some(s) = self
            .finds
            .session_mut(&rejection.session)
            .filter(|s| s.generation == rejection.generation)
        {
            s.running = false;
            s.stopping = false;
            s.error = crate::error_presentation::describe(&rejection.error);
        }
        self.retired.push(Box::new(rejection));
    }
}
