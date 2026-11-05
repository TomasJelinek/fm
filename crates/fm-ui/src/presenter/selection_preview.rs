//! At most 32 accepted intents predict visible marks; backend snapshots own all work.
use super::*;

pub(super) const PREVIEW_CAPACITY: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
struct ViewKey {
    displayed: NavigationId,
    search: u64,
    index: usize,
}
impl ViewKey {
    fn of(pane: &Pane) -> Self {
        Self {
            displayed: pane.displayed_generation(),
            search: pane.search.view_revision,
            index: Arc::as_ptr(pane.display_index()) as usize,
        }
    }
}

pub(super) struct PendingSelection {
    pub(super) revision: SelectionRevision,
    action: SelectionAction,
    view: Option<ViewKey>,
    range: Option<(usize, usize)>,
}
impl PendingSelection {
    pub(super) fn new(pane: &Pane, revision: SelectionRevision, action: SelectionAction) -> Self {
        let range = if let SelectionAction::Range { anchor, target, .. } = &action {
            pane.selection_position(anchor)
                .zip(pane.selection_position(target))
                .map(|(a, b)| (a.min(b), a.max(b)))
        } else {
            None
        };
        let view = (!matches!(action, SelectionAction::Toggle(_))).then(|| ViewKey::of(pane));
        Self {
            revision,
            action,
            view,
            range,
        }
    }
    fn applies(&self, pane: &Pane) -> bool {
        self.view.is_none_or(|view| view == ViewKey::of(pane))
    }
}

impl Pane {
    fn selection_position(&self, path: &Path) -> Option<usize> {
        if self.showing_stream() {
            self.cursor
                .into_iter()
                .chain(self.selection_anchor_id)
                .find(|id| {
                    self.entry_by_id(*id)
                        .is_some_and(|entry| entry.path == path)
                })
                .and_then(|id| self.streaming_index.get(&id).copied())
        } else {
            self.display_index()
                .by_path
                .get(path)
                .and_then(|id| self.display_index().by_id.get(id))
                .copied()
        }
    }
    /// Constant directory work: a single path lookup and at most 32 tiny intents.
    pub(crate) fn selection_visible(&self, entry: &Entry) -> bool {
        // A deferred All/Invert result records explicit exceptions in its basis.
        // Paths absent from that basis use the worker's acknowledged default until
        // the complete-view rebase arrives, including rows appended meanwhile.
        let mut selected = self.selected.contains_path(&entry.path)
            || (self.selection_deferred
                && !self.selected.basis().by_path.contains_key(&entry.path));
        for pending in &self.selection_preview {
            if !pending.applies(self) {
                continue;
            }
            match &pending.action {
                SelectionAction::Toggle(path) if *path == entry.path => selected = !selected,
                SelectionAction::Bulk(operation) => {
                    selected = match operation {
                        SelectionOperation::All => true,
                        SelectionOperation::Clear => false,
                        SelectionOperation::Invert => !selected,
                    }
                }
                SelectionAction::Range { additive, .. } => {
                    let position = if self.showing_stream() {
                        self.streaming_index.get(&entry.id)
                    } else {
                        self.display_index().by_id.get(&entry.id)
                    };
                    let in_range = position
                        .zip(pending.range)
                        .is_some_and(|(row, (start, end))| (start..=end).contains(row));
                    selected = in_range || (*additive && selected);
                }
                _ => {}
            }
        }
        selected
    }
}

impl Presenter {
    /// Modifier-aware row gesture; ranges carry raw endpoints and an immutable backend view.
    pub fn select_row_modified(
        &mut self,
        pane: PaneId,
        row: usize,
        control: bool,
        shift: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        let p = &mut self.panes[pane.index()];
        p.preserve_viewport = false;
        if p.has_parent() && row == 0 {
            p.cursor = None;
            p.parent_cursor = true;
            if !control && !shift {
                p.selection_anchor = None;
                p.selection_anchor_id = None;
            }
            if let Some(request) = &mut p.pending {
                request.cursor_touched = true;
            }
            return;
        }
        let Some(index) = row.checked_sub(p.parent_offset()) else {
            return;
        };
        let Some((id, path)) = p
            .display_entry(index)
            .map(|entry| (entry.id, entry.path.clone()))
        else {
            return;
        };
        if p.search_view_waiting() {
            return;
        }
        let anchor = p
            .selection_anchor
            .as_ref()
            .filter(|path| p.selection_position(path).is_some())
            .cloned()
            .unwrap_or_else(|| path.clone());
        let anchor_id = if p.showing_stream() {
            p.selection_anchor_id
                .filter(|id| p.entry_by_id(*id).is_some_and(|entry| entry.path == anchor))
        } else {
            p.display_index().by_path.get(&anchor).copied()
        }
        .unwrap_or(id);
        p.cursor = Some(id);
        p.parent_cursor = false;
        if let Some(request) = &mut p.pending {
            request.cursor_touched = true;
        }
        let action = if shift {
            Some(SelectionAction::Range {
                anchor: anchor.clone(),
                target: path.clone(),
                additive: control,
            })
        } else if control {
            Some(SelectionAction::Toggle(path.clone()))
        } else {
            None
        };
        if let Some(action) = action
            && !self.send_selection(pane, action, send)
        {
            return;
        }
        let p = &mut self.panes[pane.index()];
        if !shift {
            p.selection_anchor = Some(path);
            p.selection_anchor_id = Some(id);
        } else {
            p.selection_anchor = Some(anchor);
            p.selection_anchor_id = Some(anchor_id);
        }
    }
}
