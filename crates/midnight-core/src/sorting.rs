//! Native keys are prepared on workers. Sorting derives each row's facts once;
//! comparisons do no OS work, allocation, path parsing or size lookups.
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

impl EntryCollationKeys {
    pub(crate) fn retained_bytes(&self) -> usize {
        self.name
            .as_ref()
            .map_or(0, Vec::capacity)
            .saturating_add(self.extension.as_ref().map_or(0, |key| key.len()))
    }
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
        let extension = if let Some(extension) = entry.filename_extension() {
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

/// Compare captured native text keys without OS calls or allocations. Text for
/// which the locale supplied no key stays after keyed text in either direction;
/// two unavailable keys use their lossless native text as the fallback.
pub fn compare_collated_text(
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
    sort_entries_with_sizes(entries, sort, |entry| entry.metadata.size);
}

/// Use immutable calculated sizes without replacing provider metadata. The lookup
/// is called only when sorting by [`SortColumn::Size`], and then at most once per
/// entry, never per comparison; it must be stable for this call and do no I/O. All
/// other ordering rules are shared.
pub fn sort_entries_with_sizes(
    entries: &mut [Entry],
    sort: SortSpec,
    size: impl Fn(&Entry) -> Option<u64>,
) {
    let descending = sort.descending;
    let order = match sort.column {
        SortColumn::Name => sorted_order(
            entries,
            |_| (),
            |left, right| {
                compare_collated_text(
                    left.name_key,
                    left.name,
                    right.name_key,
                    right.name,
                    descending,
                )
            },
        ),
        SortColumn::Size => sorted_order(entries, size, |left, right| {
            optional(left.primary, right.primary, descending)
        }),
        SortColumn::Modified => sorted_order(
            entries,
            |entry| entry.metadata.modified,
            |left, right| optional(left.primary, right.primary, descending),
        ),
        SortColumn::Type => sorted_order(
            entries,
            |entry| {
                let key = entry
                    .collation
                    .as_ref()
                    .and_then(|keys| keys.extension.as_deref());
                entry.filename_extension().map(|extension| (extension, key))
            },
            |left, right| match (left.primary, right.primary) {
                (Some((left_ext, left_key)), Some((right_ext, right_key))) => {
                    compare_collated_text(left_key, left_ext, right_key, right_ext, descending)
                }
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
        ),
    };
    apply_order(entries, order);
}

/// Facts derived once per row, so comparisons never re-parse names or paths and
/// never call the size lookup, and the sort moves these small rows, not entries.
struct SortRow<'a, P> {
    primary: P,
    name_key: Option<&'a [u8]>,
    name: &'a OsStr,
    id: u64,
    position: usize,
    folder: bool,
}

impl<'a, P> SortRow<'a, P> {
    fn new(entry: &'a Entry, position: usize, primary: P) -> Self {
        Self {
            primary,
            name_key: entry
                .collation
                .as_ref()
                .and_then(|keys| keys.name.as_deref()),
            name: &entry.name,
            id: entry.id.0,
            position,
            folder: matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink),
        }
    }
}

/// Folders lead; `compare` orders the column; ties fall back to ascending names and
/// then stable IDs.
fn compare_rows<'a, P>(
    left: &SortRow<'a, P>,
    right: &SortRow<'a, P>,
    compare: &impl Fn(&SortRow<'a, P>, &SortRow<'a, P>) -> Ordering,
) -> Ordering {
    right
        .folder
        .cmp(&left.folder)
        .then_with(|| compare(left, right))
        .then_with(|| {
            compare_collated_text(left.name_key, left.name, right.name_key, right.name, false)
        })
        .then_with(|| left.name.cmp(right.name))
        .then_with(|| left.id.cmp(&right.id))
}

/// Return `order` where `order[target]` is the current position of the entry that
/// belongs at `target` (see [`compare_rows`]).
fn sorted_order<'a, P>(
    entries: &'a [Entry],
    primary: impl Fn(&'a Entry) -> P,
    compare: impl Fn(&SortRow<'a, P>, &SortRow<'a, P>) -> Ordering,
) -> Vec<usize> {
    let mut rows: Vec<_> = entries
        .iter()
        .enumerate()
        .map(|(position, entry)| SortRow::new(entry, position, primary(entry)))
        .collect();
    rows.sort_unstable_by(|left, right| compare_rows(left, right, &compare));
    rows.into_iter().map(|row| row.position).collect()
}

/// Whether `entries` already stand in the order that [`sort_entries_with_sizes`]
/// gives a [`SortColumn::Size`] sort with the same `size` lookup. One pass over
/// adjacent rows, without allocation, calling `size` once per entry: a panel uses it
/// to re-sort only when sizes calculated since its last sort change the order. Any
/// other column is reported as sorted (calculated sizes cannot change its order).
pub fn is_sorted_with_sizes(
    entries: &[Entry],
    sort: SortSpec,
    size: impl Fn(&Entry) -> Option<u64>,
) -> bool {
    if sort.column != SortColumn::Size {
        return true;
    }
    let descending = sort.descending;
    let compare = |left: &SortRow<'_, Option<u64>>, right: &SortRow<'_, Option<u64>>| {
        optional(left.primary, right.primary, descending)
    };
    let mut rows = entries
        .iter()
        .enumerate()
        .map(|(position, entry)| SortRow::new(entry, position, size(entry)));
    let Some(mut previous) = rows.next() else {
        return true;
    };
    for row in rows {
        if compare_rows(&previous, &row, &compare) == Ordering::Greater {
            return false;
        }
        previous = row;
    }
    true
}

/// Rearrange `entries` so that position `target` receives the entry at `order[target]`,
/// following each permutation cycle once: at most one swap per entry.
fn apply_order(entries: &mut [Entry], mut order: Vec<usize>) {
    for start in 0..order.len() {
        let mut current = start;
        loop {
            let source = order[current];
            order[current] = current;
            if source == start {
                break;
            }
            entries.swap(current, source);
            current = source;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EntryId;
    use std::{cell::Cell, collections::HashMap, path::PathBuf};

    #[test]
    fn size_lookup_runs_once_per_entry_and_long_cycles_reach_their_ranks() {
        // 7919 is coprime to 1000, so the sizes permute 0..1000 into long cycles.
        let mut entries: Vec<_> = (0..1000_u64)
            .map(|id| {
                let mut entry = Entry::new(
                    EntryId(id),
                    PathBuf::from(format!("{id:04}.txt")),
                    EntryKind::File,
                );
                entry.metadata.size = Some(id * 7919 % 1000);
                entry
            })
            .collect();
        let calls = Cell::new(0);
        let sort = SortSpec {
            column: SortColumn::Size,
            descending: false,
        };
        sort_entries_with_sizes(&mut entries, sort, |entry| {
            calls.set(calls.get() + 1);
            entry.metadata.size
        });
        assert_eq!(calls.get(), 1000);
        assert!(
            entries
                .iter()
                .enumerate()
                .all(|(rank, entry)| entry.metadata.size == Some(rank as u64))
        );
    }

    fn folder(id: u64, name: &str) -> Entry {
        Entry::new(EntryId(id), PathBuf::from(name), EntryKind::Directory)
    }

    fn file(id: u64, name: &str, size: u64) -> Entry {
        let mut entry = Entry::new(EntryId(id), PathBuf::from(name), EntryKind::File);
        entry.metadata.size = Some(size);
        entry
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries
            .iter()
            .map(|entry| entry.name.to_str().unwrap())
            .collect()
    }

    /// A Size sort keeps folders first. Folders with a calculated size are ordered by
    /// it (ties by name); folders without one follow them in either direction, by
    /// name; files follow the folders by their own lengths.
    #[test]
    fn size_sort_orders_measured_folders_first_then_unmeasured_ones_by_name() {
        let sizes: HashMap<&str, u64> = [("b-big", 900), ("a-small", 10), ("c-tie", 10)]
            .into_iter()
            .collect();
        let lookup = |entry: &Entry| {
            if entry.kind == EntryKind::Directory {
                sizes.get(entry.name.to_str().unwrap()).copied()
            } else {
                entry.metadata.size
            }
        };
        let rows = || {
            vec![
                file(1, "f-large", 5000),
                folder(2, "z-unknown"),
                folder(3, "c-tie"),
                folder(4, "b-big"),
                file(5, "e-small", 1),
                folder(6, "d-unknown"),
                folder(7, "a-small"),
            ]
        };
        for (descending, expected) in [
            (
                false,
                [
                    "a-small",
                    "c-tie",
                    "b-big",
                    "d-unknown",
                    "z-unknown",
                    "e-small",
                    "f-large",
                ],
            ),
            (
                true,
                [
                    "b-big",
                    "a-small",
                    "c-tie",
                    "d-unknown",
                    "z-unknown",
                    "f-large",
                    "e-small",
                ],
            ),
        ] {
            let sort = SortSpec {
                column: SortColumn::Size,
                descending,
            };
            let mut entries = rows();
            assert!(!is_sorted_with_sizes(&entries, sort, lookup));
            sort_entries_with_sizes(&mut entries, sort, lookup);
            assert_eq!(names(&entries), expected, "descending {descending}");
            assert!(is_sorted_with_sizes(&entries, sort, lookup));
        }
    }

    #[test]
    fn a_size_arriving_later_makes_the_order_stale_only_when_it_moves_a_row() {
        let sort = SortSpec {
            column: SortColumn::Size,
            descending: true,
        };
        let mut sizes: HashMap<u64, u64> = HashMap::from([(1, 50)]);
        let mut entries = vec![folder(1, "a"), folder(2, "b"), folder(3, "c")];
        sort_entries_with_sizes(&mut entries, sort, |entry| sizes.get(&entry.id.0).copied());
        assert_eq!(names(&entries), ["a", "b", "c"]);
        // "b" measured smaller than "a": still in order.
        sizes.insert(2, 20);
        assert!(is_sorted_with_sizes(&entries, sort, |entry| {
            sizes.get(&entry.id.0).copied()
        }));
        // "c" measured largest: it moves to the top.
        sizes.insert(3, 70);
        let lookup = |entry: &Entry| sizes.get(&entry.id.0).copied();
        assert!(!is_sorted_with_sizes(&entries, sort, lookup));
        sort_entries_with_sizes(&mut entries, sort, lookup);
        assert_eq!(names(&entries), ["c", "a", "b"]);
        // Other columns never depend on calculated sizes.
        let by_name = SortSpec {
            column: SortColumn::Name,
            descending: false,
        };
        assert!(is_sorted_with_sizes(&entries, by_name, |_| unreachable!()));
    }

    #[test]
    fn size_lookup_is_not_called_for_other_columns() {
        let mut entries: Vec<_> = (0..64_u64)
            .map(|id| {
                let extension = if id.is_multiple_of(2) { "txt" } else { "rs" };
                let mut entry = Entry::new(
                    EntryId(id),
                    PathBuf::from(format!("{:02}.{extension}", 63 - id)),
                    EntryKind::File,
                );
                entry.metadata.size = Some(id);
                entry
            })
            .collect();
        let calls = Cell::new(0);
        for column in [SortColumn::Name, SortColumn::Modified, SortColumn::Type] {
            for descending in [false, true] {
                sort_entries_with_sizes(&mut entries, SortSpec { column, descending }, |entry| {
                    calls.set(calls.get() + 1);
                    entry.metadata.size
                });
            }
        }
        assert_eq!(calls.get(), 0, "only a Size sort reads calculated sizes");
    }
}
