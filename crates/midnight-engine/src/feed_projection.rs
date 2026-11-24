//! No filesystem access: project immutable native Find rows on a bounded worker.
use midnight_core::*;
use midnight_platform_common::filter::{CompiledFilter, CompiledPanelFilter};
use std::{
    collections::{HashMap, HashSet},
    io,
    path::Path,
    sync::Arc,
};

fn invalid(message: &str) -> FsError {
    FsError::from_io(
        "project search results",
        Path::new(""),
        io::Error::new(io::ErrorKind::InvalidInput, message),
    )
}

fn check(request: &FindFeedProjectionRequest) -> Result<(), FsError> {
    if request.cancel.is_cancelled() {
        return Err(FsError::from_io(
            "project search results",
            Path::new(""),
            io::Error::new(
                io::ErrorKind::Interrupted,
                "Search result projection cancelled.",
            ),
        ));
    }
    Ok(())
}

pub(crate) fn project(
    request: &FindFeedProjectionRequest,
    locale: Option<&dyn LocaleService>,
) -> Result<FindFeedProjection, FsError> {
    check(request)?;
    let limits = FindLimits::default();
    if request.entries.len() > limits.max_results
        || request.visible.len() > request.entries.len()
        || request.visible.len() != request.selected.len()
        || request.removed.len() > 64
    {
        return Err(invalid(
            "Search result projection exceeds its row or selection bounds.",
        ));
    }
    let mut bytes = 0usize;
    let mut source = HashMap::with_capacity(request.entries.len());
    let mut paths = HashSet::with_capacity(request.entries.len());
    for entry in request.entries.iter() {
        check(request)?;
        bytes = bytes
            .saturating_add(std::mem::size_of::<Entry>())
            .saturating_add(entry.path.as_os_str().as_encoded_bytes().len())
            .saturating_add(entry.name.as_encoded_bytes().len())
            .saturating_add(entry.display_name.len());
        if bytes > limits.max_retained_bytes
            || entry.native_path().is_none()
            || !matches!(entry.kind, EntryKind::File | EntryKind::Directory)
            || entry.id.0 == 0
            || source.insert(entry.id, entry).is_some()
            || !paths.insert(&entry.path)
        {
            return Err(invalid(
                "Search results exceed the native identity or retained byte budget.",
            ));
        }
    }
    let mut selected = HashMap::with_capacity(request.visible.len());
    for (entry, mark) in request.visible.iter().zip(request.selected.iter()) {
        check(request)?;
        if !source
            .get(&entry.id)
            .is_some_and(|source| source.path == entry.path)
            || selected.insert(entry.id, *mark).is_some()
        {
            return Err(invalid(
                "The visible search result snapshot no longer matches its source.",
            ));
        }
    }
    let filter = CompiledPanelFilter::new(&request.options.filter)
        .map_err(|diagnostic| invalid(&diagnostic.message))?;
    let mask = request
        .mask
        .as_ref()
        .map(|mask| CompiledFilter::new(&mask.filter()).map_err(|message| invalid(&message)))
        .transpose()?;
    let mut removed = HashSet::new();
    let mut removed_count = 0usize;
    let mut removed_bytes = 0usize;
    for batch in request.removed.iter() {
        removed_count = removed_count.saturating_add(batch.len());
        if removed_count > limits.max_results {
            return Err(invalid("Too many removed search result paths."));
        }
        for path in batch.iter() {
            check(request)?;
            removed_bytes = removed_bytes.saturating_add(path.as_os_str().as_encoded_bytes().len());
            if removed_bytes > limits.max_retained_bytes {
                return Err(invalid("Removed paths exceed the retained byte budget."));
            }
            removed.insert(path.as_path());
        }
    }
    let all_entries: Arc<[Entry]> = if removed.is_empty() {
        request.entries.clone()
    } else {
        let mut retained = Vec::with_capacity(request.entries.len());
        for entry in request.entries.iter() {
            check(request)?;
            if !entry.path.ancestors().any(|path| removed.contains(path)) {
                retained.push(entry.clone());
            }
        }
        retained.into()
    };
    // Reuse Find's locale ordering and aggregate 16 MiB key budget. These temporary
    // identities are positions for ordering only; output keeps each original EntryId.
    let mut matched = Vec::new();
    let mut ordering = Vec::new();
    for entry in all_entries.iter() {
        check(request)?;
        if (!request.options.show_hidden && crate::hidden_entries::is_hidden(entry))
            || !filter.matches_entry(entry)
        {
            continue;
        }
        matched.push(entry);
        ordering.push(Arc::new(FindResult {
            id: FindResultId(ordering.len() as u64 + 1),
            entry: FindEntry {
                path: entry.path.clone(),
                kind: entry.kind,
                metadata: entry.metadata.clone(),
                identity: TransferIdentity {
                    device: 0,
                    object: [0; 16],
                },
                revision: FindRevision([0; 6]),
            },
            duplicate_group: None,
            duplicate_group_size: 0,
            archive: None,
        }));
    }
    let order = crate::find_sort::order(
        &ordering,
        request.options.sort,
        request.id,
        locale,
        &request.cancel,
        &mut crate::find_sort::KeyCache::default(),
    )?;
    let mut entries = Vec::with_capacity(matched.len());
    let mut marks = Vec::with_capacity(matched.len());
    let mut selected_count = 0usize;
    let mut selected_bytes = 0u64;
    for id in order.ids.iter() {
        check(request)?;
        let entry = matched[id.0 as usize - 1];
        let mut mark = selected.get(&entry.id).copied().unwrap_or(false);
        if selected.contains_key(&entry.id)
            && let (Some(mask), Some(matcher)) = (&request.mask, &mask)
            && (entry.kind != EntryKind::Directory || mask.folders)
            && matcher.matches(&entry.name)
        {
            mark = mask.select;
        }
        if mark {
            selected_count += 1;
            if entry.kind == EntryKind::File {
                selected_bytes = selected_bytes.saturating_add(entry.metadata.size.unwrap_or(0));
            }
        }
        entries.push(entry.clone());
        marks.push(mark);
    }
    check(request)?;
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    let cursor = request
        .cursor
        .and_then(|id| index.by_id.get(&id).copied())
        .unwrap_or(0);
    Ok(FindFeedProjection {
        all_entries,
        entries: entries.into(),
        index,
        selected: Arc::new(marks),
        selected_count,
        selected_bytes,
        cursor,
    })
}

#[cfg(test)]
#[path = "feed_projection_tests.rs"]
mod tests;
