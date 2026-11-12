//! Bounded immediate feedback over occurrence-based backend selection.
use super::*;

impl Pane {
    pub(super) fn tree_selection_visible(&self, entry: &Entry) -> Option<bool> {
        let publication = self.tree.publication.as_ref()?;
        let view = &publication.view;
        let position = *view.index.directory.by_id.get(&entry.id)?;
        let row = view.rows.get(position)?;
        if !row.selectable || row.context_only {
            return Some(false);
        }
        let mut selected = publication.selection.contains_position(position);
        for (_, action) in &self.tree.preview {
            match action {
                TreeSelectionAction::Replace(replacement) => {
                    selected = replacement.contains_position(position, row.id)
                }
                TreeSelectionAction::Remove(rows) if rows.contains(&row.id) => selected = false,
                TreeSelectionAction::Toggle(id) if *id == row.id => selected = !selected,
                TreeSelectionAction::Range {
                    anchor,
                    target,
                    additive,
                } => {
                    let hit = view
                        .index
                        .row_positions
                        .get(anchor)
                        .zip(view.index.row_positions.get(target))
                        .is_some_and(|(a, b)| (*a.min(b)..=*a.max(b)).contains(&position));
                    selected = hit || (*additive && selected);
                }
                TreeSelectionAction::Ranges { ranges, mode } => {
                    selected = mode.selected(ranges.contains(position), selected)
                }
                TreeSelectionAction::Bulk(operation) => {
                    selected = row.bulk_selected(*operation, selected);
                }
                _ => {}
            }
        }
        Some(
            self.rectangle_preview
                .as_ref()
                .map_or(selected, |preview| preview.visible(self, entry, selected)),
        )
    }
}

impl Presenter {
    pub(super) fn send_tree_selection(
        &mut self,
        pane: PaneId,
        action: TreeSelectionAction,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &mut self.panes[pane.index()];
        let Some(publication) = &p.tree.publication else {
            return false;
        };
        if p.loading
            || !p.tree_input_ready()
            || p.tree.preview.len() >= selection_preview::PREVIEW_CAPACITY
        {
            return false;
        }
        let Some(revision) = p.tree.revision.0.checked_add(1).map(SelectionRevision) else {
            return false;
        };
        let request = TreeSelectionRequest {
            view: publication.view.clone(),
            revision,
            seed: Some(publication.selection.clone()),
            seed_revision: p.tree.applied,
            action: action.clone(),
        };
        if !send(Command::Tree(Box::new(TreeCommand::Select(request)))) {
            p.set_error("Selection queue is busy. Try again.", &mut self.retired);
            return false;
        }
        p.tree.revision = revision;
        p.tree.preview.push((revision, action));
        true
    }

    pub(super) fn select_tree_row(
        &mut self,
        pane: PaneId,
        row: usize,
        control: bool,
        shift: bool,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &mut self.panes[pane.index()];
        if p.loading {
            return false;
        }
        if p.has_parent() && row == 0 {
            p.cursor = None;
            p.parent_cursor = true;
            if !shift {
                p.tree.anchor = None;
            }
            return true;
        }
        let Some(row) = row
            .checked_sub(p.parent_offset())
            .and_then(|row| p.tree_row(row))
        else {
            return false;
        };
        let id = row.id;
        let entry = row.entry.id;
        let selectable = row.selectable && !row.context_only;
        let anchor = p
            .tree
            .anchor
            .filter(|id| {
                p.tree_view()
                    .is_some_and(|view| view.index.row_positions.contains_key(id))
            })
            .unwrap_or(id);
        p.cursor = Some(entry);
        p.parent_cursor = false;
        p.preserve_viewport = false;
        if !selectable {
            return false;
        }
        let action = if shift {
            Some(TreeSelectionAction::Range {
                anchor,
                target: id,
                additive: control,
            })
        } else if control {
            Some(TreeSelectionAction::Toggle(id))
        } else {
            None
        };
        if let Some(action) = action
            && !self.send_tree_selection(pane, action, send)
        {
            return false;
        }
        if !shift {
            self.panes[pane.index()].tree.anchor = Some(id);
        }
        true
    }
}
