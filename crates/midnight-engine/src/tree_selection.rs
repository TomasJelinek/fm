//! Worker-owned selection for one exact inline-tree display projection.
use crate::providers::ProviderError;
use midnight_core::{
    SelectionRevision, TreeProjection, TreeRootSelection, TreeSelectionAction,
    TreeSelectionRequest, TreeSelectionSnapshot,
};
use std::sync::Arc;

pub(crate) struct Selection {
    pub(crate) revision: SelectionRevision,
    pub(crate) snapshot: Arc<TreeSelectionSnapshot>,
}

impl Selection {
    pub(crate) fn from_root(
        view: Arc<TreeProjection>,
        revision: SelectionRevision,
        selected: &TreeRootSelection,
    ) -> Result<Self, ProviderError> {
        selected.validate_for(&view.root).map_err(invalid)?;
        let mut bits = vec![0; view.rows.len().div_ceil(64)];
        for (position, row) in view.rows.iter().enumerate() {
            if !row.selectable || row.context_only {
                continue;
            }
            let Some(id) = row.original_root_entry else {
                continue;
            };
            let is_selected = match selected {
                TreeRootSelection::Empty => false,
                TreeRootSelection::Native(selection) => {
                    let original = view
                        .root
                        .index
                        .by_id
                        .get(&id)
                        .ok_or(ProviderError::StaleReference)?;
                    selection.bits()[original / 64] & (1 << (original % 64)) != 0
                }
                TreeRootSelection::Archive(selection) => selection.contains(&id),
            };
            set(&mut bits, position, is_selected);
        }
        Ok(Self {
            revision,
            snapshot: Arc::new(TreeSelectionSnapshot::from_bits(view, bits).map_err(invalid)?),
        })
    }

    pub(crate) fn rebased(&self, view: Arc<TreeProjection>) -> Result<Self, ProviderError> {
        Ok(Self {
            revision: self.revision,
            snapshot: Arc::new(self.snapshot.rebase(view).map_err(invalid)?),
        })
    }

    /// Validate the entire gesture before replacing the published snapshot.
    pub(crate) fn apply(&mut self, request: &TreeSelectionRequest) -> Result<(), ProviderError> {
        if !Arc::ptr_eq(self.snapshot.basis(), &request.view) || request.revision <= self.revision {
            return Err(ProviderError::StaleReference);
        }
        let base = if request.seed_revision > self.revision {
            let seed = request.seed.as_ref().ok_or(ProviderError::StaleReference)?;
            if request.seed_revision >= request.revision
                || !Arc::ptr_eq(seed.basis(), &request.view)
            {
                return Err(ProviderError::StaleReference);
            }
            seed
        } else {
            &self.snapshot
        };
        let mut bits = base.bits().to_vec();
        apply_action(&request.view, &mut bits, &request.action)?;
        let snapshot =
            TreeSelectionSnapshot::from_bits(request.view.clone(), bits).map_err(invalid)?;
        self.snapshot = Arc::new(snapshot);
        self.revision = request.revision;
        Ok(())
    }
}

fn apply_action(
    view: &TreeProjection,
    bits: &mut [u64],
    action: &TreeSelectionAction,
) -> Result<(), ProviderError> {
    let position = |row| {
        view.index
            .row_positions
            .get(&row)
            .copied()
            .ok_or(ProviderError::StaleReference)
    };
    if let TreeSelectionAction::Replace(selection) = action {
        if !selection.matches_view(view) {
            return Err(ProviderError::StaleReference);
        }
        bits.copy_from_slice(selection.bits());
        return Ok(());
    }
    if let TreeSelectionAction::Remove(rows) = action {
        if rows.len() > 64
            || rows.len() != rows.iter().collect::<std::collections::HashSet<_>>().len()
        {
            return Err(ProviderError::ResourceLimited);
        }
        for row in rows.iter() {
            let index = position(*row)?;
            if !view.rows[index].selectable || view.rows[index].context_only {
                return Err(ProviderError::Unsupported);
            }
        }
        for row in rows.iter() {
            set(bits, position(*row)?, false);
        }
        return Ok(());
    }
    let range = match action {
        TreeSelectionAction::Toggle(row) => {
            let index = position(*row)?;
            let row = &view.rows[index];
            if !row.selectable || row.context_only {
                return Err(ProviderError::Unsupported);
            }
            bits[index / 64] ^= 1 << (index % 64);
            return Ok(());
        }
        TreeSelectionAction::Range { anchor, target, .. } => {
            let (anchor, target) = (position(*anchor)?, position(*target)?);
            Some(anchor.min(target)..=anchor.max(target))
        }
        TreeSelectionAction::Ranges { ranges, .. } if ranges.len() != view.rows.len() => {
            return Err(ProviderError::StaleReference);
        }
        _ => None,
    };
    for (index, row) in view.rows.iter().enumerate() {
        if !row.selectable || row.context_only {
            continue;
        }
        let selected = bits[index / 64] & (1 << (index % 64)) != 0;
        let next = match action {
            TreeSelectionAction::Range { additive, .. } => {
                range.as_ref().is_some_and(|range| range.contains(&index))
                    || (*additive && selected)
            }
            TreeSelectionAction::Ranges { ranges, mode } => {
                mode.selected(ranges.contains(index), selected)
            }
            TreeSelectionAction::Bulk(operation) => row.bulk_selected(*operation, selected),
            TreeSelectionAction::Replace(_)
            | TreeSelectionAction::Rebase
            | TreeSelectionAction::Toggle(_)
            | TreeSelectionAction::Remove(_) => selected,
        };
        set(bits, index, next);
    }
    Ok(())
}

fn set(bits: &mut [u64], position: usize, selected: bool) {
    let mask = 1 << (position % 64);
    if selected {
        bits[position / 64] |= mask;
    } else {
        bits[position / 64] &= !mask;
    }
}

fn invalid(message: &'static str) -> ProviderError {
    ProviderError::Failed(message.into())
}
