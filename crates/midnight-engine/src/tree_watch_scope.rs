//! Capture watch authority only from an exactly acknowledged model publication.
use super::*;
use crate::{
    providers::ProviderTarget,
    watch::{
        signature::Signature,
        tree::{ScopeTarget, TreeScope},
    },
};
use midnight_core::{Entry, EntryId, EntryKind};
use std::collections::HashSet;
use std::time::{Duration, Instant};

pub(super) struct Failure {
    listings: HashMap<BranchId, Option<u64>>,
    error: ProviderError,
    diagnostic: midnight_core::TreeDiagnostic,
    retry_at: Instant,
}

impl Session {
    pub(crate) fn watch_failure_diagnostic(
        &self,
        error: &ProviderError,
    ) -> midnight_core::TreeDiagnostic {
        match &self.watch_failure {
            Some(failure) if &failure.error == error => failure.diagnostic.clone(),
            _ => crate::tree_diagnostics::diagnostic(&self.reports, error),
        }
    }

    pub(crate) fn watch_eligibility(&self) -> HashSet<BranchId> {
        let mut eligible = HashSet::from([self.tree.root()]);
        for row in self.projection.rows.iter() {
            if matches!(row.entry.kind, EntryKind::Directory | EntryKind::DirectoryLink)
                && let Some(branch) = self.projection_state.branch(row.id)
                // Empty rows hide the disclosure affordance but retain explicit
                // expansion intent. Continue observing them for newly added children.
                && self.tree.state(branch).is_some_and(|state| state.expanded)
                && self.tree.loaded_native_branch(branch)
            {
                eligible.insert(branch);
            }
        }
        eligible
    }

    pub(crate) fn watch_scope(&mut self) -> Result<Option<TreeScope>, ProviderError> {
        self.check_view(&self.projection)?;
        let eligible = self.watch_eligibility();
        let listings = eligible
            .iter()
            .map(|branch| {
                (
                    *branch,
                    self.tree.watch_listing(*branch).map(|(version, _)| version),
                )
            })
            .collect::<HashMap<_, _>>();
        if let Some(failure) = &self.watch_failure
            && failure.listings == listings
            && Instant::now() < failure.retry_at
        {
            return Err(failure.error.clone());
        }
        let result = self.build_watch_scope(eligible);
        self.watch_failure = result.as_ref().err().map(|error| Failure {
            listings,
            error: error.clone(),
            diagnostic: crate::tree_diagnostics::diagnostic(&self.reports, error),
            retry_at: Instant::now() + Duration::from_secs(15),
        });
        result
    }

    fn build_watch_scope(
        &mut self,
        eligible: HashSet<BranchId>,
    ) -> Result<Option<TreeScope>, ProviderError> {
        self.watch_cache
            .retain(|branch, _| eligible.contains(branch));
        let mut branches = vec![self.tree.root()];
        branches.extend(self.projection.rows.iter().filter_map(|row| {
            let branch = self.projection_state.branch(row.id)?;
            eligible.contains(&branch).then_some(branch)
        }));
        if branches.len() > TreeProjection::MAX_ROWS {
            return Err(ProviderError::ResourceLimited);
        }
        let mut targets = Vec::with_capacity(branches.len());
        let mut next_cache = HashMap::with_capacity(branches.len());
        let mut bytes = 0usize;
        for branch in branches {
            check_cancel(&self.lifetime)?;
            let (version, children) = self
                .tree
                .watch_listing(branch)
                .ok_or(ProviderError::StaleReference)?;
            let directory_identity = self.tree.loaded_native_identity(branch)?;
            let target = if let Some((cached_version, cached)) = self.watch_cache.get(&branch)
                && *cached_version == version
                && cached.directory_identity == directory_identity
            {
                cached.clone()
            } else {
                let root = branch == self.tree.root();
                let mounted = if root {
                    self.tree.root_target(&self.lifetime)?
                } else {
                    self.tree.resolve_entry_target(branch, &self.lifetime)?
                };
                let ProviderTarget::Native(path) = mounted.value()? else {
                    if root {
                        return Ok(None);
                    }
                    return Err(ProviderError::Unsupported);
                };
                let mounted_path_bytes = path.capacity();
                let path = path.clone();
                let path_bytes = mounted_path_bytes
                    .checked_add(path.capacity())
                    .ok_or(ProviderError::ResourceLimited)?;
                if !path.is_absolute() {
                    return Err(ProviderError::InvalidResponse(
                        "watch target is not an absolute native path",
                    ));
                }
                let mut signature = Signature::default();
                if root {
                    for entry in self.projection.root_actions.entries.iter() {
                        check_cancel(&self.lifetime)?;
                        if entry.path.parent() != Some(path.as_path())
                            || entry.path.file_name() != Some(entry.name.as_os_str())
                        {
                            return Err(ProviderError::InvalidResponse(
                                "invalid accepted root watch identity",
                            ));
                        }
                        signature
                            .add(entry)
                            .map_err(|_| ProviderError::ResourceLimited)?;
                    }
                } else {
                    for child in children {
                        let row = self
                            .tree
                            .state(*child)
                            .ok_or(ProviderError::StaleReference)?;
                        let child_target =
                            self.tree.resolve_entry_target(*child, &self.lifetime)?;
                        let ProviderTarget::Native(child_path) = child_target.value()? else {
                            return Err(ProviderError::Unsupported);
                        };
                        if child_path.parent() != Some(path.as_path()) {
                            return Err(ProviderError::InvalidResponse(
                                "watch child escaped its native parent",
                            ));
                        }
                        let mut entry = Entry::new(EntryId(0), child_path.clone(), row.entry.kind);
                        entry.metadata = row.entry.metadata.clone();
                        signature
                            .add(&entry)
                            .map_err(|_| ProviderError::ResourceLimited)?;
                    }
                }
                Arc::new(ScopeTarget {
                    branch,
                    path,
                    display: signature.digest(!root),
                    folders_only: !root,
                    directory_identity,
                    path_bytes,
                    _mounted: mounted,
                    _registration: self.registration.clone(),
                })
            };
            bytes = bytes
                .checked_add(target.path_bytes)
                .ok_or(ProviderError::ResourceLimited)?;
            if bytes > 32 * 1024 * 1024 {
                return Err(ProviderError::ResourceLimited);
            }
            next_cache.insert(branch, (version, target.clone()));
            targets.push(target);
        }
        self.watch_cache = next_cache;
        Ok(Some(TreeScope {
            stamp: self.projection.stamp,
            targets,
            eligible,
            complete: true,
        }))
    }
}
