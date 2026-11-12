//! Worker-owned native argument captures; selection and focus keep distinct command semantics.
use super::{CapturedSelection, CapturedTarget, Session};
use crate::{
    providers::{ProviderError, ProviderTarget},
    retirement::WorkerOwned,
};
use midnight_core::*;
use std::{
    collections::HashMap,
    io,
    path::{Component, Path},
    sync::Arc,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

struct Payload {
    selected: Option<CapturedSelection>,
    selected_positions: HashMap<TreeRowId, usize>,
    focus: Option<(TreeRowId, CapturedTarget)>,
    root: CapturedTarget,
    view: Arc<TreeProjection>,
    request: TreeRequestId,
    cancel: Cancellation,
    _permit: OwnedSemaphorePermit,
}
struct Sources {
    owner: WorkerOwned<Payload>,
}
impl std::ops::Deref for Sources {
    type Target = Payload;
    fn deref(&self) -> &Self::Target {
        &self.owner
    }
}
impl std::fmt::Debug for Sources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeUserCommandSources")
            .field("stamp", &self.view.stamp)
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}

impl Session {
    pub(crate) fn capture_user_command(
        &mut self,
        request: &TreeUserCommandRequest,
        capacity: &Arc<Semaphore>,
        retirement: &crate::tree_rename::Retirement,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn CapturedUserCommandSources>, ProviderError> {
        let linked = cancel.linked(&request.cancel);
        self.check_selection(
            &request.view,
            request.selection_revision,
            &request.selection,
            &linked,
        )?;
        if request.request.0 == 0 || request.request.0 <= self.last_request.0 {
            return Err(ProviderError::StaleReference);
        }
        let permit = capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| ProviderError::ResourceLimited)?;
        // Reserve before any provider-bearing capture. Final Arc release may happen
        // in a rejected command, a stale reply, or after Tokio has stopped.
        let ownership = retirement
            .domain
            .try_reserve()
            .map_err(|_| ProviderError::ResourceLimited)?;
        let sources = (|| {
            let selected = if request.selection.is_empty() {
                None
            } else {
                Some(self.capture_selection(
                    &request.view,
                    request.selection_revision,
                    &request.selection,
                    None,
                    &linked,
                )?)
            };
            let focus = request
                .focus
                .map(|row| {
                    self.capture_target(&request.view, row, &linked)
                        .map(|captured| (row, captured))
                })
                .transpose()?;
            let root = CapturedTarget {
                target: self.tree.root_target(&linked)?,
                _registration: self.registration.clone(),
            };
            let selected_positions = selected.as_ref().map_or_else(HashMap::new, |selected| {
                selected
                    .targets
                    .iter()
                    .enumerate()
                    .map(|(index, occurrence)| (occurrence.row, index))
                    .collect()
            });
            let sources = Payload {
                selected_positions,
                selected,
                focus,
                root,
                view: request.view.clone(),
                request: request.request,
                cancel: request.cancel.clone(),
                _permit: permit,
            };
            // Validate the whole batch before publishing any native argument values.
            sources.check(&linked)?;
            Ok(sources)
        })()
        .map_err(|error| {
            if matches!(error, ProviderError::ResourceLimited) {
                ProviderError::Failed(
                    concat!(
                        "Command target capture exceeded a provider or resource budget. ",
                        "Select fewer targets or retry explicitly."
                    )
                    .into(),
                )
            } else {
                error
            }
        })?;
        self.last_request = request.request;
        Ok(Arc::new(Sources {
            owner: ownership.own(sources),
        }))
    }
}

fn native(target: &CapturedTarget) -> Result<&Path, ProviderError> {
    let ProviderTarget::Native(path) = target.target.value()? else {
        return Err(ProviderError::Unsupported);
    };
    if !path.is_absolute()
        || path.as_os_str().as_encoded_bytes().contains(&0)
        || path.components().next_back() == Some(Component::ParentDir)
    {
        return Err(ProviderError::InvalidResponse(
            "Command arguments require native paths, not parent navigation.",
        ));
    }
    Ok(path)
}

impl Payload {
    fn check_entry(&self, row: TreeRowId, target: &CapturedTarget) -> Result<(), ProviderError> {
        let position = self
            .view
            .index
            .row_positions
            .get(&row)
            .ok_or(ProviderError::StaleReference)?;
        let row = &self.view.rows[*position];
        if !row.selectable
            || row.context_only
            || !matches!(
                row.entry.kind,
                EntryKind::File
                    | EntryKind::Directory
                    | EntryKind::Symlink
                    | EntryKind::DirectoryLink
            )
        {
            return Err(ProviderError::Unsupported);
        }
        native(target)?;
        Ok(())
    }
    fn check_root(&self, cancel: &Cancellation) -> Result<(), ProviderError> {
        if cancel.is_cancelled() || self.cancel.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        native(&self.root)?;
        Ok(())
    }
    fn check(&self, cancel: &Cancellation) -> Result<(), ProviderError> {
        let cancelled = || cancel.is_cancelled() || self.cancel.is_cancelled();
        if cancelled() {
            return Err(ProviderError::Cancelled);
        }
        native(&self.root)?;
        if let Some(selected) = &self.selected {
            for occurrence in &selected.targets {
                if cancelled() {
                    return Err(ProviderError::Cancelled);
                }
                self.check_entry(occurrence.row, &occurrence.captured)?;
            }
        }
        if let Some((row, target)) = &self.focus {
            self.check_entry(*row, target)?;
        }
        if cancelled() {
            return Err(ProviderError::Cancelled);
        }
        Ok(())
    }
    fn entry(&self, row: TreeRowId, target: &CapturedTarget) -> Result<Entry, ProviderError> {
        self.check_entry(row, target)?;
        let original = &self.view.rows[self.view.index.row_positions[&row]].entry;
        // Rebuild native names from the retained provider path, not display text.
        let mut entry = Entry::new(EntryId(row.0), native(target)?.to_owned(), original.kind);
        entry.metadata = original.metadata.clone();
        Ok(entry)
    }
}

impl CapturedUserCommandSources for Sources {
    fn stamp(&self) -> TreeViewStamp {
        self.view.stamp
    }
    fn request(&self) -> TreeRequestId {
        self.request
    }
    fn resolve(&self, cancel: &Cancellation) -> Result<ResolvedUserCommandSources, FsError> {
        self.check(cancel).map_err(failure)?;
        let selected = self
            .selected
            .as_ref()
            .map_or_else(
                || Ok(Vec::new()),
                |selected| {
                    selected
                        .targets
                        .iter()
                        .map(|value| self.entry(value.row, &value.captured))
                        .collect()
                },
            )
            .map_err(failure)?;
        let focus = self
            .focus
            .as_ref()
            .map(|(row, target)| self.entry(*row, target))
            .transpose()
            .map_err(failure)?;
        let directory = native(&self.root).map_err(failure)?.to_owned();
        self.check(cancel).map_err(failure)?;
        Ok(ResolvedUserCommandSources {
            directory,
            selected: selected.into(),
            focus,
        })
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.check(cancel).map_err(failure)
    }
    fn revalidate_for(&self, entries: &[EntryId], cancel: &Cancellation) -> Result<(), FsError> {
        self.check_root(cancel).map_err(failure)?;
        for id in entries {
            self.check_root(cancel).map_err(failure)?;
            let row = TreeRowId(id.0);
            let target = if let Some((focus, target)) = &self.focus
                && *focus == row
            {
                target
            } else {
                let position = self
                    .selected_positions
                    .get(&row)
                    .ok_or_else(|| failure(ProviderError::StaleReference))?;
                &self.selected.as_ref().expect("indexed selection").targets[*position].captured
            };
            self.check_entry(row, target).map_err(failure)?;
        }
        self.check_root(cancel).map_err(failure)
    }
}

fn failure(error: ProviderError) -> FsError {
    let kind = match error {
        ProviderError::Cancelled => io::ErrorKind::Interrupted,
        ProviderError::Unsupported => io::ErrorKind::Unsupported,
        _ => io::ErrorKind::InvalidData,
    };
    FsError::from_io(
        "Custom Command capture",
        Path::new(""),
        io::Error::new(kind, error),
    )
}
