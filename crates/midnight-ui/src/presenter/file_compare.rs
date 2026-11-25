//! Capture immutable UI identities; selection enumeration and file access run in the comparator.
use super::*;
use midnight_core::{ArchiveCompareFolder, ArchiveCompareMember};

#[cfg(test)]
#[path = "file_compare_tests.rs"]
mod tests;

impl Presenter {
    pub fn file_compare_launch(&self, active: PaneId) -> Result<FileCompareLaunch, String> {
        let active_pane = &self.panes[active.index()];
        let panes: &[PaneId] = if compare_selection_len(active_pane) == 2 {
            std::slice::from_ref(&active)
        } else {
            &[PaneId::Left, PaneId::Right]
        };
        // A compared panel's selection and focus belong to the folder hidden
        // under search results fed to it.
        if panes.iter().any(|pane| self.feed(*pane).is_some()) {
            return Err(find::feed::refusal("Compare files"));
        }
        let disabled_plugins = self
            .viewer_plugins
            .startup_disabled
            .clone()
            .ok_or(self.viewer_plugins.policy_unavailable())?;
        let comparators = self
            .comparators
            .accepted
            .as_ref()
            .ok_or(if self.preferences.loaded {
                "Comparator settings are unavailable."
            } else {
                "Comparator settings are still loading."
            })?;
        let mut sources = Vec::with_capacity(panes.len());
        for id in panes {
            let pane = &self.panes[id.index()];
            let counterpart = panes.len() == 2 && *id != active && compare_selection_len(pane) == 0;
            if pane.tree_view().is_some() {
                sources.extend(tree_sources(pane, counterpart, panes.len() == 2));
                continue;
            }
            if let Some(archive) = &pane.archive {
                sources.extend(archive_sources(
                    pane,
                    archive,
                    counterpart,
                    panes.len() == 2,
                ));
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
            comparators: comparators.preferences.clone(),
            comparator_policy: comparators.id,
        })
    }
}

fn compare_selection_len(pane: &Pane) -> usize {
    pane.tree.publication.as_ref().map_or_else(
        || {
            pane.archive
                .as_ref()
                .map_or(pane.selected.len(), |a| a.selected.len())
        },
        |publication| publication.selection.len(),
    )
}

fn archive_folder(archive: &archives::ArchivePane) -> Option<ArchiveCompareFolder> {
    let location = &archive.location;
    Some(ArchiveCompareFolder {
        mount: location.mount.clone()?,
        directory: location.directory,
        source_version: location.address.source_version.clone(),
    })
}

fn archive_file(folder: &ArchiveCompareFolder, entry: &Entry) -> Option<FileCompareSource> {
    let node = entry.virtual_node.as_deref()?;
    (entry.kind == EntryKind::File
        && node.id.entry == entry.id.0
        && node.readable
        && node.parent == folder.directory)
        .then(|| {
            FileCompareSource::ArchiveMember(ArchiveCompareMember {
                folder: folder.clone(),
                node: node.clone(),
            })
        })
}

fn archive_sources(
    pane: &Pane,
    archive: &archives::ArchivePane,
    counterpart: bool,
    across: bool,
) -> Vec<FileCompareSource> {
    let unspecified = || vec![FileCompareSource::Unspecified];
    let Some(folder) = archive_folder(archive) else {
        return unspecified();
    };
    let count = archive.selected.len();
    if pane.loading
        || pane.search_view_waiting()
        || pane.search.pending
        || archive.selection_pending
        || count > 2
        || (across && count > 1)
        || (count == 0 && !counterpart && !pane.search_allows_activation())
    {
        return unspecified();
    }
    let view = pane.display_index();
    if count == 0 && counterpart {
        return vec![FileCompareSource::ArchiveSameName {
            folder,
            entries: pane.entries.clone(),
            view: view.clone(),
            visible: None,
        }];
    }
    if count == 0 {
        return pane
            .cursor
            .filter(|_| !pane.parent_cursor)
            .filter(|id| view.by_id.contains_key(id))
            .and_then(|id| pane.entry_by_id(id))
            .and_then(|entry| archive_file(&folder, entry))
            .map_or_else(unspecified, |source| vec![source]);
    }
    vec![FileCompareSource::ArchiveSelected {
        folder,
        entries: pane.entries.clone(),
        index: pane.index.clone(),
        view: view.clone(),
        selection: archive.selected.clone(),
    }]
}

fn tree_sources(pane: &Pane, counterpart: bool, across_panes: bool) -> Vec<FileCompareSource> {
    let unspecified = || vec![FileCompareSource::Unspecified];
    let Some(publication) = &pane.tree.publication else {
        return unspecified();
    };
    let view = &publication.view;
    let count = publication.selection.len();
    let archive = pane.archive.as_ref().and_then(archive_folder);
    if (!pane.native_capabilities() && archive.is_none())
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
                .map(|row| root_file(view, *row, archive.as_ref()))
                .collect()
        });
        return paths.unwrap_or_else(unspecified);
    }
    if counterpart {
        if let Some(folder) = archive {
            return vec![FileCompareSource::ArchiveSameName {
                folder,
                entries: view.root_actions.entries.clone(),
                view: view.root_actions.index.clone(),
                visible: Some(view.index.comparison_root_files.clone()),
            }];
        }
        return vec![FileCompareSource::RootSameName {
            directory: pane.path.clone(),
            entries: view.root_actions.entries.clone(),
            view: view.root_actions.index.clone(),
            visible: view.index.comparison_root_files.clone(),
        }];
    }
    pane.tree_cursor_row()
        .and_then(|row| root_file(view, row.id, archive.as_ref()))
        .map_or_else(unspecified, |source| vec![source])
}

fn root_file(
    view: &TreeProjection,
    row: TreeRowId,
    archive: Option<&ArchiveCompareFolder>,
) -> Option<FileCompareSource> {
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
    if let Some(folder) = archive {
        return archive_file(folder, entry);
    }
    (entry.kind == EntryKind::File)
        .then(|| {
            entry
                .native_path()
                .map(|path| FileCompareSource::Focused(path.to_owned()))
        })
        .flatten()
}
