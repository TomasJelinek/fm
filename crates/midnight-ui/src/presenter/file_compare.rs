//! Capture immutable UI identities; selection enumeration and file access run in the comparator.
use super::*;

#[cfg(test)]
#[path = "file_compare_tests.rs"]
mod tests;

impl Presenter {
    pub fn file_compare_launch(&self, active: PaneId) -> Result<FileCompareLaunch, String> {
        let disabled_plugins = self
            .viewer_plugins
            .startup_disabled
            .clone()
            .ok_or("Viewer preferences are still loading.")?;
        let active_pane = &self.panes[active.index()];
        let panes: &[PaneId] = if compare_selection_len(active_pane) == 2 {
            std::slice::from_ref(&active)
        } else {
            &[PaneId::Left, PaneId::Right]
        };
        let mut sources = Vec::with_capacity(panes.len());
        for id in panes {
            let pane = &self.panes[id.index()];
            let counterpart = panes.len() == 2 && *id != active && compare_selection_len(pane) == 0;
            if pane.tree_view().is_some() {
                sources.extend(tree_sources(pane, counterpart, panes.len() == 2));
                continue;
            }
            if !pane.native_capabilities()
                || pane.loading
                || pane.search_view_waiting()
                || (pane.selected.is_empty() && !counterpart && !pane.search_allows_activation())
                || pane.search.pending
                || pane.selection_rebase_pending
                || pane.selection_applied != pane.selection_revision
                || pane.selection_deferred.any()
                || (panes.len() == 2 && pane.selected.len() > 1)
            {
                sources.push(FileCompareSource::Unspecified);
                continue;
            }
            if !pane.selected.is_empty() {
                sources.push(FileCompareSource::Selected {
                    entries: pane.entries.clone(),
                    view: pane.display_index().clone(),
                    selection: pane.selected.clone(),
                });
            } else if counterpart {
                sources.push(FileCompareSource::SameName {
                    directory: pane.path.clone(),
                    entries: pane.entries.clone(),
                    view: pane.display_index().clone(),
                });
            } else {
                let entry = pane
                    .cursor
                    .filter(|_| !pane.parent_cursor)
                    .and_then(|id| pane.entry_by_id(id))
                    .filter(|entry| entry.kind == EntryKind::File && entry.native_path().is_some());
                sources.push(entry.map_or(FileCompareSource::Unspecified, |entry| {
                    FileCompareSource::Focused(entry.path.clone())
                }));
            }
        }
        Ok(FileCompareLaunch {
            sources,
            disabled_plugins,
        })
    }
}

fn compare_selection_len(pane: &Pane) -> usize {
    pane.tree
        .publication
        .as_ref()
        .map_or(pane.selected.len(), |publication| {
            publication.selection.len()
        })
}

fn tree_sources(pane: &Pane, counterpart: bool, across_panes: bool) -> Vec<FileCompareSource> {
    let unspecified = || vec![FileCompareSource::Unspecified];
    let Some(publication) = &pane.tree.publication else {
        return unspecified();
    };
    let view = &publication.view;
    let count = publication.selection.len();
    if !pane.native_capabilities()
        || pane.loading
        || !pane.tree_input_ready()
        || !pane.tree.preview.is_empty()
        || pane.tree.revision != pane.tree.resolved
        || (count == 0 && !counterpart && !pane.search_allows_activation())
        || count > 2
        || (across_panes && count > 1)
        || !Arc::ptr_eq(publication.selection.basis(), view)
    {
        return unspecified();
    }
    if count != 0 {
        let paths: Option<Vec<_>> = publication.selection.small_selection().and_then(|rows| {
            rows.iter()
                .map(|row| root_file(view, *row).map(FileCompareSource::Focused))
                .collect()
        });
        return paths.unwrap_or_else(unspecified);
    }
    if counterpart {
        return vec![FileCompareSource::RootSameName {
            directory: pane.path.clone(),
            entries: view.root_actions.entries.clone(),
            view: view.root_actions.index.clone(),
            visible: view.index.native_root_files.clone(),
        }];
    }
    pane.tree_cursor_row()
        .and_then(|row| root_file(view, row.id))
        .map_or_else(unspecified, |path| vec![FileCompareSource::Focused(path)])
}

fn root_file(view: &TreeProjection, row: TreeRowId) -> Option<PathBuf> {
    let row = view.rows.get(*view.index.row_positions.get(&row)?)?;
    if row.depth != 0
        || row.parent.is_some()
        || !row.selectable
        || row.context_only
        || row.error.is_some()
    {
        return None;
    }
    let alias = row.original_root_entry?;
    let entry = view
        .root_actions
        .entries
        .get(*view.root_actions.index.by_id.get(&alias)?)?;
    (entry.kind == EntryKind::File)
        .then(|| entry.native_path().map(Path::to_owned))
        .flatten()
}
