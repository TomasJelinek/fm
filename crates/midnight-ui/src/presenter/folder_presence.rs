//! Bounded flat-row presentation hints. Probing never changes flat action authority.
use super::*;
use std::collections::VecDeque;

const CACHE_ROWS: usize = 1024;

/// Display provenance is independent of the current request's action authority.
#[derive(Clone)]
pub(super) struct Source {
    navigation: NavigationId,
    path: PathBuf,
    entries: Arc<[Entry]>,
    index: Arc<DirectoryIndex>,
    pub(super) viewport: (usize, usize),
}
impl Source {
    pub(super) fn capture(pane: &Pane) -> Option<Self> {
        if !pane.native_capabilities() {
            return None;
        }
        Some(Self {
            navigation: pane.successful_generation?,
            path: pane.path.clone(),
            entries: pane.entries.clone(),
            index: pane.index.clone(),
            viewport: pane.tree.viewport,
        })
    }
    pub(super) fn matches(&self, pane: &Pane) -> bool {
        pane.successful_generation == Some(self.navigation)
            && pane.path == self.path
            && Arc::ptr_eq(&self.entries, &pane.entries)
            && Arc::ptr_eq(&self.index, &pane.index)
    }
    fn entry(&self, id: EntryId) -> Option<&Entry> {
        self.index
            .by_id
            .get(&id)
            .and_then(|position| self.entries.get(*position))
    }
}

/// One bounded diagnostic basis survives failed navigation back to these rows.
pub(super) struct Retained {
    source: Source,
    values: HashMap<EntryId, TreeDiagnostic>,
    order: VecDeque<EntryId>,
}
impl Retained {
    fn new(source: Source) -> Self {
        Self {
            source,
            values: HashMap::new(),
            order: VecDeque::new(),
        }
    }
    fn insert(&mut self, id: EntryId, error: TreeDiagnostic, retired: &mut Vec<DeferredDrop>) {
        if !self
            .source
            .entry(id)
            .is_some_and(|entry| entry.kind == EntryKind::Directory && entry.virtual_node.is_none())
        {
            retired.push(Box::new(error));
            return;
        }
        if !self.values.contains_key(&id) {
            while self.values.len() >= CACHE_ROWS {
                if let Some(old) = self.order.pop_front() {
                    retired.push(Box::new(self.values.remove(&old)));
                }
            }
            self.order.push_back(id);
        }
        if let Some(previous) = self.values.insert(id, error) {
            retired.push(Box::new(previous));
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    root: Option<TreeRootSnapshot>,
    revision: FolderPresenceRevision,
    viewport: Vec<EntryId>,
    values: HashMap<EntryId, FolderPresence>,
    order: VecDeque<EntryId>,
    pending: Option<(Arc<FolderPresenceRequest>, bool)>,
}
impl State {
    fn retire(&mut self) -> Self {
        if let Some((request, _)) = &self.pending {
            request.cancel.cancel();
        }
        let old = std::mem::take(self);
        self.revision = old.revision;
        old
    }
    fn same_root(&self, pane: &Pane) -> bool {
        self.root.as_ref().is_some_and(|root| {
            root.navigation == NavigationId(pane.generation)
                && Arc::ptr_eq(&root.entries, &pane.entries)
                && Arc::ptr_eq(&root.index, &pane.index)
        })
    }
}
impl Pane {
    pub(super) fn retain_folder_errors(&mut self, retired: &mut Vec<DeferredDrop>) {
        let Some(source) = Source::capture(self) else {
            return;
        };
        if self
            .folder_errors
            .as_ref()
            .is_some_and(|errors| !errors.source.matches(self))
        {
            retired.push(Box::new(self.folder_errors.take()));
        }
        if self.folder_presence.same_root(self) {
            for (id, value) in &self.folder_presence.values {
                if let FolderPresence::Unavailable(error) = value {
                    self.folder_errors
                        .get_or_insert_with(|| Retained::new(source.clone()))
                        .insert(*id, error.clone(), retired);
                }
            }
        }
    }
    pub(super) fn record_folder_failure(
        &mut self,
        navigation: NavigationId,
        path: &Path,
        error: &FsError,
        retired: &mut Vec<DeferredDrop>,
    ) {
        if self.generation != navigation.0 || self.loading || self.pending.is_some() {
            return;
        }
        self.retain_folder_errors(retired);
        let Some(source) = Source::capture(self) else {
            return;
        };
        let Some(id) = source.index.by_path.get(path).copied() else {
            return;
        };
        let errors = self
            .folder_errors
            .get_or_insert_with(|| Retained::new(source));
        if error.kind == std::io::ErrorKind::Interrupted && errors.values.contains_key(&id) {
            return;
        }
        errors.insert(
            id,
            TreeDiagnostic::new(&crate::error_presentation::describe(error)),
            retired,
        );
    }
    fn retained_folder_diagnostic(&self, entry: &Entry) -> Option<&TreeDiagnostic> {
        if !self.native_capabilities() || self.tree_view().is_some() {
            return None;
        }
        let errors = self.folder_errors.as_ref()?;
        let source_entry = errors.source.entry(entry.id)?;
        if errors.source.matches(self)
            && source_entry.path == entry.path
            && std::ptr::eq(source_entry, entry)
        {
            errors.values.get(&entry.id)
        } else {
            None
        }
    }
    pub(crate) fn root_presence_disclosure(&self, entry: &Entry) -> i32 {
        if self.retained_folder_diagnostic(entry).is_some() {
            return 6;
        }
        if entry.kind != EntryKind::Directory || !self.root_disclosure_available() {
            return 0;
        }
        match self
            .folder_presence
            .same_root(self)
            .then(|| self.folder_presence.values.get(&entry.id))
            .flatten()
        {
            Some(FolderPresence::Empty) => 5,
            Some(FolderPresence::HasFolders) => 3,
            Some(FolderPresence::Unavailable(_)) => 6,
            None => 1,
        }
    }
    pub(crate) fn root_presence_error(&self, id: EntryId) -> &str {
        self.root_presence_diagnostic(id)
            .map_or("", TreeDiagnostic::as_str)
    }
    pub(super) fn root_presence_diagnostic(&self, id: EntryId) -> Option<&TreeDiagnostic> {
        // A streamed row may reuse a committed ID. Prefer that actual row; only
        // an ID absent from the stream can still describe the retained source.
        let entry = self
            .showing_stream()
            .then(|| {
                self.streaming_index
                    .get(&id)
                    .and_then(|position| self.streaming.get(*position))
            })
            .flatten()
            .or_else(|| {
                self.index
                    .by_id
                    .get(&id)
                    .and_then(|position| self.entries.get(*position))
            });
        if let Some(error) = entry.and_then(|entry| self.retained_folder_diagnostic(entry)) {
            return Some(error);
        }
        if self.folder_presence.same_root(self)
            && let Some(FolderPresence::Unavailable(error)) = self.folder_presence.values.get(&id)
        {
            Some(error)
        } else {
            None
        }
    }
}
impl Presenter {
    pub(super) fn clear_folder_presence(&mut self, pane: PaneId) {
        self.retired
            .push(Box::new(self.panes[pane.index()].folder_presence.retire()));
    }
    pub(super) fn flush_folder_presence(&mut self, send: &dyn Fn(Command) -> bool) {
        if self.operations.closing {
            for pane in [PaneId::Left, PaneId::Right] {
                self.clear_folder_presence(pane);
                self.retired
                    .push(Box::new(self.panes[pane.index()].folder_errors.take()));
            }
            return;
        }
        for side in [PaneId::Left, PaneId::Right] {
            let p = &mut self.panes[side.index()];
            if p.tree_view().is_some() || !p.root_disclosure_available() {
                if p.folder_presence.root.is_some() {
                    self.retired.push(Box::new(p.folder_presence.retire()));
                }
                continue;
            }
            if !p.folder_presence.same_root(p) {
                self.retired.push(Box::new(p.folder_presence.retire()));
                p.folder_presence.root = Some(TreeRootSnapshot {
                    pane: side,
                    navigation: NavigationId(p.generation),
                    entries: p.entries.clone(),
                    index: p.index.clone(),
                });
            }
            let (first, end) = p.tree.viewport;
            let first = first.saturating_sub(p.parent_offset()).min(p.entries.len());
            let end = end
                .saturating_sub(p.parent_offset())
                .min(p.entries.len())
                .min(first.saturating_add(FolderPresenceRequest::MAX_VISIBLE));
            let visible: Vec<_> = p.entries[first..end.max(first)]
                .iter()
                .filter(|entry| entry.kind == EntryKind::Directory)
                .map(|entry| entry.id)
                .collect();
            if visible != p.folder_presence.viewport {
                if let Some((request, _)) = p.folder_presence.pending.take() {
                    request.cancel.cancel();
                    self.retired.push(Box::new(request));
                }
                p.folder_presence.viewport = visible;
            }
            if p.folder_presence.pending.is_none() {
                let unknown: Vec<_> = p
                    .folder_presence
                    .viewport
                    .iter()
                    .filter(|id| !p.folder_presence.values.contains_key(id))
                    .copied()
                    .collect();
                if !unknown.is_empty()
                    && let Some(revision) = p.folder_presence.revision.0.checked_add(1)
                {
                    p.folder_presence.revision = FolderPresenceRevision(revision);
                    let request = Arc::new(FolderPresenceRequest {
                        root: p.folder_presence.root.clone().expect("current flat root"),
                        revision: p.folder_presence.revision,
                        visible: unknown,
                        cancel: Cancellation::default(),
                    });
                    p.folder_presence.pending = Some((request, false));
                }
            }
            if let Some((request, sent)) = &mut p.folder_presence.pending
                && !*sent
            {
                *sent = send(Command::ProbeFolderPresence(request.clone()));
            }
        }
    }
    pub(super) fn apply_folder_presence(&mut self, result: Box<FolderPresenceSnapshot>) {
        let p = &mut self.panes[result.root.pane.index()];
        let valid = !p.loading
            && p.tree_view().is_none()
            && p.root_disclosure_available()
            && p.folder_presence.same_root(p)
            && result.values.len() <= FolderPresenceRequest::MAX_VISIBLE
            && p.folder_presence
                .pending
                .as_ref()
                .is_some_and(|(request, sent)| {
                    *sent
                        && !request.cancel.is_cancelled()
                        && request.revision == result.revision
                        && request.root.same_snapshot(&result.root)
                        && result.values.len() == request.visible.len()
                        && result
                            .values
                            .iter()
                            .all(|(id, _)| request.visible.contains(id))
                });
        if valid
            && result
                .values
                .iter()
                .map(|(id, _)| *id)
                .collect::<HashSet<_>>()
                .len()
                == result.values.len()
        {
            for (id, value) in result.values.iter() {
                if !p.folder_presence.values.contains_key(id) {
                    while p.folder_presence.values.len() >= CACHE_ROWS {
                        if let Some(old) = p.folder_presence.order.pop_front() {
                            self.retired
                                .push(Box::new(p.folder_presence.values.remove(&old)));
                        }
                    }
                    p.folder_presence.order.push_back(*id);
                }
                if let Some(previous) = p.folder_presence.values.insert(*id, value.clone()) {
                    self.retired.push(Box::new(previous));
                }
            }
            self.retired
                .push(Box::new(p.folder_presence.pending.take()));
        }
        self.retired.push(Box::new(result));
    }
}
