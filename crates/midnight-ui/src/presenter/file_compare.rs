//! Capture immutable UI identities; selection enumeration and file access run in the comparator.
use super::*;

#[cfg(test)]
#[path = "file_compare_tests.rs"]
mod tests;

impl Presenter {
    pub fn file_compare_launch(&self, active: PaneId) -> Result<FileCompareLaunch, String> {
        if self.panes.iter().any(|pane| pane.tree_view().is_some()) {
            return Err("Open the folder before comparing files. Captured branch actions are not connected yet.".into());
        }
        let disabled_plugins = self
            .viewer_plugins
            .startup_disabled
            .clone()
            .ok_or("Viewer preferences are still loading.")?;
        let active_pane = &self.panes[active.index()];
        let panes: &[PaneId] = if active_pane.selected.len() == 2 {
            std::slice::from_ref(&active)
        } else {
            &[PaneId::Left, PaneId::Right]
        };
        let mut sources = Vec::with_capacity(panes.len());
        for id in panes {
            let pane = &self.panes[id.index()];
            let counterpart = panes.len() == 2 && *id != active && pane.selected.is_empty();
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
