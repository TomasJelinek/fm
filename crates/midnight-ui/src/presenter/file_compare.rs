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
        if active_pane.selected.len() > 2 {
            return Err("Select at most two files in the active pane.".into());
        }
        let panes: &[PaneId] = if active_pane.selected.len() == 2 {
            std::slice::from_ref(&active)
        } else {
            &[PaneId::Left, PaneId::Right]
        };
        let mut sources = Vec::with_capacity(panes.len());
        for id in panes {
            let pane = &self.panes[id.index()];
            if pane.archive.is_some() {
                return Err("File comparison currently requires native files. Extract archive members before comparing them.".into());
            }
            if pane.loading
                || !pane.search_allows_activation()
                || pane.search.pending
                || pane.selection_rebase_pending
                || pane.selection_applied != pane.selection_revision
                || pane.selection_deferred
            {
                return Err("Wait for the panel and selection to finish updating.".into());
            }
            if panes.len() == 2 && pane.selected.len() > 1 {
                return Err(
                    "Select one file in each pane, or two files in the active pane.".into(),
                );
            }
            if !pane.selected.is_empty() {
                sources.push(FileCompareSource::Selected {
                    entries: pane.entries.clone(),
                    view: pane.display_index().clone(),
                    selection: pane.selected.clone(),
                });
            } else {
                let entry = pane.cursor.filter(|_| !pane.parent_cursor).and_then(|id| pane.entry_by_id(id)).filter(|entry| entry.kind == EntryKind::File && entry.virtual_node.is_none()).ok_or("Focus a regular file in each pane, or select two files in the active pane.")?;
                sources.push(FileCompareSource::Focused(entry.path.clone()));
            }
        }
        Ok(FileCompareLaunch {
            sources,
            disabled_plugins,
        })
    }
}
