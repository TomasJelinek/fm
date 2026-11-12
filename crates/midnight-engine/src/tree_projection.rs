//! Backend sorting/filtering and display identity for inline branches.
use crate::providers::{
    ProviderError, ProviderTarget,
    tree::{BranchId, DirectoryTree, Disclosure, TreeRow},
};
use midnight_core::{
    Cancellation, Entry, EntryId, EntryKind, LocaleService, PanelFilter, SortSpec, TreeDisclosure,
    TreeDisplayRow, TreeLayoutRevision, TreeProjection, TreeRevision, TreeRootSnapshot, TreeRowId,
    TreeSearchOptions, TreeSessionId, TreeViewStamp, prepare_sort_keys, sort_entries_with_sizes,
};
use midnight_platform_common::filter::CompiledPanelFilter;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use unicase::UniCase;

#[derive(Clone, Default)]
pub(crate) struct ProjectionOptions {
    pub(crate) sizes: Arc<HashMap<TreeRowId, midnight_core::FolderSize>>,
    pub(crate) sort: SortSpec,
    pub(crate) filter: PanelFilter,
    pub(crate) search: TreeSearchOptions,
    pub(crate) locale: Option<Arc<dyn LocaleService>>,
}

#[derive(Clone)]
struct Record {
    id: TreeRowId,
    entry: Entry,
    original_root_entry: Option<EntryId>,
}

#[derive(Clone)]
pub(crate) struct ProjectionState {
    records: HashMap<BranchId, Record>,
    branches: HashMap<TreeRowId, BranchId>,
    next_row: TreeRowId,
    next_alias: Option<u64>,
    root_actions: TreeRootSnapshot,
}

impl ProjectionState {
    pub(crate) fn retained_rows(&self) -> HashSet<TreeRowId> {
        self.branches.keys().copied().collect()
    }
    pub(crate) fn new(
        tree: &DirectoryTree,
        root: &TreeRootSnapshot,
    ) -> Result<Self, ProviderError> {
        let rows = tree.rows();
        if rows.len() != root.entries.len() {
            return Err(ProviderError::InvalidResponse(
                "captured root row count changed",
            ));
        }
        let mut state = Self {
            records: HashMap::new(),
            branches: HashMap::new(),
            next_row: TreeRowId(1),
            next_alias: root
                .entries
                .iter()
                .map(|entry| entry.id.0)
                .max()
                .unwrap_or(0)
                .checked_add(1),
            root_actions: root.clone(),
        };
        // RootSeed emits the retained backend listing in its original order.
        // This binds exact Entry values without enumerating or resolving them again.
        for (row, entry) in rows.iter().zip(root.entries.iter()) {
            if row.depth != 0
                || row.entry.kind != entry.kind
                || row.entry.display_name != entry.display_name
                || row.entry.metadata != entry.metadata
            {
                return Err(ProviderError::InvalidResponse(
                    "captured root row identity changed",
                ));
            }
            let id = state.allocate()?;
            let mut display = entry.clone();
            display.id = EntryId(id.0);
            state.records.insert(
                row.id,
                Record {
                    id,
                    entry: display,
                    original_root_entry: Some(entry.id),
                },
            );
            state.branches.insert(id, row.id);
        }
        Ok(state)
    }

    fn allocate(&mut self) -> Result<TreeRowId, ProviderError> {
        let current = self.next_row;
        self.next_row = current
            .checked_next()
            .ok_or(ProviderError::ResourceLimited)?;
        Ok(current)
    }

    /// Failed unpublished projections consume their allocated identities too.
    pub(crate) fn advance_high_water(&mut self, other: &Self) {
        self.next_row = TreeRowId(self.next_row.0.max(other.next_row.0));
        self.next_alias = match (self.next_alias, other.next_alias) {
            (Some(current), Some(next)) => Some(current.max(next)),
            _ => None,
        };
    }

    pub(crate) fn branch(&self, row: TreeRowId) -> Option<BranchId> {
        self.branches.get(&row).copied()
    }

    /// Materialize only from exact current native node targets on the model worker.
    pub(crate) fn refresh_root_actions(
        &mut self,
        tree: &DirectoryTree,
        cancel: &Cancellation,
    ) -> Result<(), ProviderError> {
        let target = tree.root_target(cancel)?;
        let ProviderTarget::Native(parent) = target.value()? else {
            return Err(ProviderError::Unsupported);
        };
        let mut entries = Vec::new();
        for row in tree.rows().into_iter().filter(|row| row.depth == 0) {
            check_cancel(cancel)?;
            let (id, alias) = if let Some(record) = self.records.get(&row.id) {
                (
                    record.id,
                    record
                        .original_root_entry
                        .ok_or(ProviderError::StaleReference)?,
                )
            } else {
                let id = self.allocate()?;
                let alias = self.next_alias.ok_or(ProviderError::ResourceLimited)?;
                self.next_alias = alias.checked_add(1);
                (id, EntryId(alias))
            };
            let entry = display_entry(tree, &row, id, cancel)?;
            if entry
                .native_path()
                .is_none_or(|path| path.parent() != Some(parent.as_path()))
            {
                return Err(ProviderError::InvalidResponse(
                    "native root action is outside its captured root",
                ));
            }
            self.records.insert(
                row.id,
                Record {
                    id,
                    entry: entry.clone(),
                    original_root_entry: Some(alias),
                },
            );
            self.branches.insert(id, row.id);
            let mut action = entry;
            action.id = alias;
            entries.push(action);
        }
        let actions = TreeRootSnapshot {
            pane: self.root_actions.pane,
            navigation: self.root_actions.navigation,
            index: Arc::new(midnight_core::DirectoryIndex::from_entries(&entries)),
            entries: entries.into(),
        };
        if !actions.within_payload_budget() {
            return Err(ProviderError::ResourceLimited);
        }
        self.root_actions = actions;
        Ok(())
    }

    pub(crate) fn build(
        &mut self,
        tree: &DirectoryTree,
        root: &TreeRootSnapshot,
        session: TreeSessionId,
        previous: Option<&TreeProjection>,
        options: &ProjectionOptions,
        cancel: &Cancellation,
    ) -> Result<Arc<TreeProjection>, ProviderError> {
        check_cancel(cancel)?;
        self.records
            .retain(|branch, _| tree.state(*branch).is_some());
        self.branches
            .retain(|_, branch| self.records.contains_key(branch));
        let model_rows = tree.rows();
        for row in &model_rows {
            check_cancel(cancel)?;
            if !self.records.contains_key(&row.id) {
                let id = self.allocate()?;
                let entry = display_entry(tree, row, id, cancel)?;
                self.records.insert(
                    row.id,
                    Record {
                        id,
                        entry,
                        original_root_entry: None,
                    },
                );
                self.branches.insert(id, row.id);
            }
            let record = self
                .records
                .get_mut(&row.id)
                .ok_or(ProviderError::StaleReference)?;
            record.entry.metadata = row.entry.metadata.clone();
            record
                .entry
                .display_name
                .clone_from(&row.entry.display_name);
            if let Some(locale) = &options.locale {
                prepare_sort_keys(
                    std::slice::from_mut(&mut record.entry),
                    locale.as_ref(),
                    cancel,
                )
                .map_err(|error| ProviderError::Failed(error.to_string()))?;
            }
        }
        let rows = self.sorted_rows(model_rows, options, cancel)?;
        let revision = match previous {
            Some(previous) => previous
                .stamp
                .revision
                .checked_next()
                .ok_or(ProviderError::ResourceLimited)?,
            None => TreeRevision(1),
        };
        let layout = previous.map_or(TreeLayoutRevision(1), |previous| previous.layout_revision);
        let mut projection = TreeProjection::new_with_root_actions(
            root.clone(),
            self.root_actions.clone(),
            TreeViewStamp {
                pane: root.pane,
                navigation: root.navigation,
                session,
                revision,
            },
            layout,
            rows,
        )
        .map_err(|error| match error {
            // This classification applies only to our complete projection's
            // local admission guards, never arbitrary provider response errors.
            "tree projection exceeds its row budget"
            | "tree projection exceeds its payload budget" => ProviderError::ResourceLimited,
            _ => ProviderError::InvalidResponse(error),
        })?;
        if let Some(previous) = previous
            && !previous.same_structure(&projection)
        {
            projection.layout_revision = layout
                .checked_next()
                .ok_or(ProviderError::ResourceLimited)?;
        }
        check_cancel(cancel)?;
        Ok(Arc::new(projection))
    }

    fn sorted_rows(
        &self,
        model_rows: Vec<TreeRow>,
        options: &ProjectionOptions,
        cancel: &Cancellation,
    ) -> Result<Vec<TreeDisplayRow>, ProviderError> {
        options
            .search
            .validate_bounds()
            .map_err(ProviderError::InvalidResponse)?;
        let query = UniCase::unicode(&options.search.query).to_folded_case();
        let filter = CompiledPanelFilter::new(&options.filter)
            .map_err(|error| ProviderError::Failed(error.message))?;
        let mut rows = HashMap::with_capacity(model_rows.len());
        let mut siblings: HashMap<Option<TreeRowId>, Vec<Entry>> = HashMap::new();
        let mut matching = HashSet::new();
        for row in model_rows {
            check_cancel(cancel)?;
            let record = &self.records[&row.id];
            let parent = if row.depth == 0 {
                None
            } else {
                Some(
                    self.records
                        .get(&row.parent.ok_or(ProviderError::StaleReference)?)
                        .ok_or(ProviderError::StaleReference)?
                        .id,
                )
            };
            let passes_filter = filter.matches_entry(&record.entry);
            let prefix = if passes_filter && !query.is_empty() {
                crate::search::entry_prefix_end(&record.entry, &query, &|| cancel.is_cancelled())
                    .map_err(|()| ProviderError::Cancelled)?
            } else {
                None
            };
            if passes_filter
                && (query.is_empty() || !options.search.filter_matches || prefix.is_some())
            {
                matching.insert(record.id);
            }
            siblings
                .entry(parent)
                .or_default()
                .push(record.entry.clone());
            let error = row.error.map(|error| {
                let error = error.to_string();
                error[..error.floor_char_boundary(4096)].to_owned()
            });
            rows.insert(
                record.id,
                TreeDisplayRow {
                    id: record.id,
                    entry: record.entry.clone(),
                    parent,
                    depth: row.depth,
                    disclosure: disclosure(row.disclosure),
                    // The model retains expansion intent for later refreshes;
                    // a confirmed empty row has no expanded display affordance.
                    expanded: row.expanded && row.disclosure != Disclosure::Empty,
                    selectable: true,
                    navigable: matches!(
                        record.entry.kind,
                        EntryKind::Directory | EntryKind::DirectoryLink
                    ) || row.disclosure != Disclosure::None,
                    context_only: false,
                    error,
                    error_details: row.error_details,
                    original_root_entry: record.original_root_entry,
                    prefix_end: prefix.unwrap_or(0),
                    search_match: prefix.is_some(),
                },
            );
        }
        let mut keep = matching.clone();
        for id in &matching {
            let mut ancestor = rows[id].parent;
            while let Some(parent) = ancestor {
                keep.insert(parent);
                ancestor = rows[&parent].parent;
            }
        }
        for entries in siblings.values_mut() {
            sort_entries_with_sizes(entries, options.sort, |entry| {
                if entry.kind == EntryKind::Directory {
                    options
                        .sizes
                        .get(&TreeRowId(entry.id.0))
                        .map(|size| size.bytes)
                } else {
                    entry.metadata.size
                }
            });
        }
        let mut pending = siblings
            .get(&None)
            .into_iter()
            .flatten()
            .rev()
            .map(|entry| TreeRowId(entry.id.0))
            .collect::<Vec<_>>();
        let mut result = Vec::with_capacity(keep.len());
        while let Some(id) = pending.pop() {
            check_cancel(cancel)?;
            if !keep.contains(&id) {
                continue;
            }
            let mut row = rows.remove(&id).ok_or(ProviderError::StaleReference)?;
            row.context_only = !matching.contains(&id);
            row.selectable = !row.context_only;
            if row.context_only {
                row.prefix_end = 0;
                row.search_match = false;
            }
            pending.extend(
                siblings
                    .get(&Some(id))
                    .into_iter()
                    .flatten()
                    .rev()
                    .map(|entry| TreeRowId(entry.id.0)),
            );
            result.push(row);
        }
        Ok(result)
    }
}

fn display_entry(
    tree: &DirectoryTree,
    row: &TreeRow,
    id: TreeRowId,
    cancel: &Cancellation,
) -> Result<Entry, ProviderError> {
    let target = tree.resolve_entry_target(row.id, cancel)?;
    let mut entry = match target.value()? {
        ProviderTarget::Native(path) => Entry::new(EntryId(id.0), path.clone(), row.entry.kind),
        ProviderTarget::Archive(target) => {
            let original = target
                .original
                .as_ref()
                .ok_or(ProviderError::InvalidResponse(
                    "archive display target lost its original record",
                ))?;
            if original.kind != row.entry.kind
                || original.raw_name != row.entry.raw_name
                || original.display_name != row.entry.display_name
            {
                return Err(ProviderError::InvalidResponse(
                    "archive display record changed",
                ));
            }
            Entry::from_archive(original.clone())
        }
    };
    entry.id = EntryId(id.0);
    entry.metadata = row.entry.metadata.clone();
    entry.display_name.clone_from(&row.entry.display_name);
    Ok(entry)
}

fn check_cancel(cancel: &Cancellation) -> Result<(), ProviderError> {
    if cancel.is_cancelled() {
        Err(ProviderError::Cancelled)
    } else {
        Ok(())
    }
}

fn disclosure(value: Disclosure) -> TreeDisclosure {
    match value {
        Disclosure::None => TreeDisclosure::None,
        Disclosure::Unknown => TreeDisclosure::Unknown,
        Disclosure::Loading => TreeDisclosure::Loading,
        Disclosure::Collapsed => TreeDisclosure::Collapsed,
        Disclosure::Expanded => TreeDisclosure::Expanded,
        Disclosure::Empty => TreeDisclosure::Empty,
        Disclosure::Failed => TreeDisclosure::Failed,
    }
}
