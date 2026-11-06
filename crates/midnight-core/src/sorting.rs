//! Native keys are prepared on workers; comparators do no OS work or allocation.
use crate::{Cancellation, Entry, EntryKind, FsError, LocaleService, SortColumn, SortSpec};
use std::{
    cmp::Ordering,
    collections::HashMap,
    ffi::{OsStr, OsString},
    io,
    sync::Arc,
};

/// A generated-key resource bound, never a filename/path truncation threshold.
pub const MAX_COLLATION_KEY_BYTES: usize = 4 * 1024 * 1024;
const EXTENSION_CACHE_CAPACITY: usize = 256;

#[derive(Debug)]
pub(crate) struct EntryCollationKeys {
    name: Option<Vec<u8>>,
    extension: Option<Arc<[u8]>>,
}

/// Fill immutable per-entry keys once per captured listing. Cloned entries keep
/// them for subsequent re-sorts. Providers must retain the same session locale.
pub fn prepare_sort_keys(
    entries: &mut [Entry],
    locale: &dyn LocaleService,
    cancel: &Cancellation,
) -> Result<(), FsError> {
    let mut extensions: HashMap<OsString, Option<Arc<[u8]>>> = HashMap::new();
    for entry in entries {
        check_cancel(cancel, entry)?;
        if entry.collation.is_some() {
            continue;
        }
        let name = key(locale, &entry.name, entry, cancel)?;
        let extension = if let Some(extension) = entry.path.extension() {
            if let Some(cached) = extensions.get(extension) {
                cached.clone()
            } else {
                let value = key(locale, extension, entry, cancel)?.map(Arc::from);
                if extensions.len() < EXTENSION_CACHE_CAPACITY {
                    extensions.insert(extension.to_owned(), value.clone());
                }
                value
            }
        } else {
            None
        };
        check_cancel(cancel, entry)?;
        entry.collation = Some(Arc::new(EntryCollationKeys { name, extension }));
    }
    Ok(())
}

fn check_cancel(cancel: &Cancellation, entry: &Entry) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(FsError::from_io(
            "prepare locale sorting",
            &entry.path,
            io::Error::new(
                io::ErrorKind::Interrupted,
                "Locale sort preparation cancelled.",
            ),
        ))
    } else {
        Ok(())
    }
}

fn key(
    locale: &dyn LocaleService,
    value: &OsStr,
    entry: &Entry,
    cancel: &Cancellation,
) -> Result<Option<Vec<u8>>, FsError> {
    check_cancel(cancel, entry)?;
    let key = locale.collation_key(value).map_err(|mut error| {
        if error.path.as_os_str().is_empty() {
            error.path = entry.path.clone();
        }
        error
    })?;
    check_cancel(cancel, entry)?;
    if key
        .as_ref()
        .is_some_and(|key| key.len() > MAX_COLLATION_KEY_BYTES)
    {
        return Err(FsError::from_io(
            "prepare locale sorting",
            &entry.path,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Native collation key exceeds its 4 MiB resource budget.",
            ),
        ));
    }
    Ok(key)
}

fn direction(order: Ordering, descending: bool) -> Ordering {
    if descending { order.reverse() } else { order }
}

fn collated(
    left: Option<&[u8]>,
    left_raw: &OsStr,
    right: Option<&[u8]>,
    right_raw: &OsStr,
    descending: bool,
) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => direction(left.cmp(right), descending),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => direction(left_raw.cmp(right_raw), descending),
    }
}

fn name_order(left: &Entry, right: &Entry, descending: bool) -> Ordering {
    collated(
        left.collation
            .as_ref()
            .and_then(|keys| keys.name.as_deref()),
        &left.name,
        right
            .collation
            .as_ref()
            .and_then(|keys| keys.name.as_deref()),
        &right.name,
        descending,
    )
}

fn optional<T: Ord>(left: Option<T>, right: Option<T>, descending: bool) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => direction(left.cmp(&right), descending),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Sort only immutable keys. Native-equivalent names use raw names and stable IDs
/// as ascending deterministic ties. Missing metadata/extensions stay last.
pub fn sort_entries(entries: &mut [Entry], sort: SortSpec) {
    entries.sort_unstable_by(|left, right| {
        let is_folder = |kind| matches!(kind, EntryKind::Directory | EntryKind::DirectoryLink);
        let directories = is_folder(right.kind).cmp(&is_folder(left.kind));
        if directories != Ordering::Equal {
            return directories;
        }
        let primary = match sort.column {
            SortColumn::Name => name_order(left, right, sort.descending),
            SortColumn::Size => optional(left.metadata.size, right.metadata.size, sort.descending),
            SortColumn::Modified => optional(
                left.metadata.modified,
                right.metadata.modified,
                sort.descending,
            ),
            SortColumn::Type => match (left.path.extension(), right.path.extension()) {
                (Some(left_ext), Some(right_ext)) => collated(
                    left.collation
                        .as_ref()
                        .and_then(|keys| keys.extension.as_deref()),
                    left_ext,
                    right
                        .collation
                        .as_ref()
                        .and_then(|keys| keys.extension.as_deref()),
                    right_ext,
                    sort.descending,
                ),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
        };
        primary
            .then_with(|| name_order(left, right, false))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.0.cmp(&right.id.0))
    });
}
