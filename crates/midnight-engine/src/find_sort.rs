//! Backend-only bounded Find ordering. Discovery-order result IDs stay authoritative.
use midnight_core::*;
use std::{cmp::Ordering, collections::HashMap, ffi::OsStr, io, path::Path, sync::Arc};

const KEY_BUDGET: usize = 16 * 1024 * 1024;

struct Keys {
    primary: Option<Vec<u8>>,
    name: Option<Vec<u8>>,
}

/// Locale keys prepared for one search generation and column. The caller keeps
/// one cache per generation. Results are append-only within a generation and
/// keys depend only on (path, column), so a re-sort of a streaming search
/// derives keys only for new results; the key budget still covers all of them.
#[derive(Default)]
pub(crate) struct KeyCache {
    column: Option<FindSortColumn>,
    keys: Vec<Keys>,
    used: usize,
}

pub(crate) fn order(
    results: &[Arc<FindResult>],
    sort: FindSortSpec,
    revision: u64,
    locale: Option<&dyn LocaleService>,
    cancel: &Cancellation,
    cache: &mut KeyCache,
) -> Result<FindOrder, FsError> {
    if results.len() > FindLimits::default().max_results {
        return Err(find_error(
            Path::new(""),
            io::ErrorKind::InvalidData,
            "Find sort exceeds the result budget.",
        ));
    }
    if cache.column != Some(sort.column) || cache.keys.len() > results.len() {
        cache.column = Some(sort.column);
        cache.keys.clear();
        cache.used = 0;
    }
    for (index, result) in results.iter().enumerate() {
        check(cancel)?;
        if result.id.0 != index as u64 + 1 {
            return Err(find_error(
                &result.entry.path,
                io::ErrorKind::InvalidData,
                "Find discovery IDs are not contiguous.",
            ));
        }
        if index < cache.keys.len() {
            continue;
        }
        let path = &result.entry.path;
        let primary = prepare_key(
            locale,
            value(path, sort.column),
            path,
            &mut cache.used,
            cancel,
        )?;
        let name = if sort.column == FindSortColumn::Extension {
            prepare_key(locale, Some(filename(path)), path, &mut cache.used, cancel)?
        } else {
            None
        };
        cache.keys.push(Keys { primary, name });
    }
    let keys = &cache.keys[..results.len()];
    let mut indices: Vec<_> = (0..results.len()).collect();
    indices.sort_unstable_by(|&left, &right| {
        let a = &results[left].entry;
        let b = &results[right].entry;
        let order = match sort.column {
            FindSortColumn::Extension => {
                extension(a, &keys[left], b, &keys[right], sort.descending)
            }
            FindSortColumn::Size => numeric(a.metadata.size, b.metadata.size, sort.descending),
            FindSortColumn::Modified => {
                numeric(a.metadata.modified, b.metadata.modified, sort.descending)
            }
            _ => compare_collated_text(
                keys[left].primary.as_deref(),
                value(&a.path, sort.column).unwrap_or_default(),
                keys[right].primary.as_deref(),
                value(&b.path, sort.column).unwrap_or_default(),
                sort.descending,
            ),
        };
        order
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| results[left].id.0.cmp(&results[right].id.0))
    });
    if results
        .iter()
        .any(|result| result.duplicate_group.is_some())
    {
        // The first member in the selected column order determines its group's
        // position; members retain that order. Stable group IDs never change.
        let mut first_member = HashMap::new();
        let mut ranks = vec![(0, 0); results.len()];
        for (rank, &index) in indices.iter().enumerate() {
            check(cancel)?;
            let group_rank = results[index]
                .duplicate_group
                .map_or(rank, |group| *first_member.entry(group).or_insert(rank));
            ranks[index] = (group_rank, rank);
        }
        indices.sort_unstable_by_key(|&index| ranks[index]);
    }
    check(cancel)?;
    let mut positions = vec![0; results.len()];
    for (row, &index) in indices.iter().enumerate() {
        positions[index] = row;
    }
    Ok(FindOrder {
        revision,
        sort,
        ids: indices.into_iter().map(|index| results[index].id).collect(),
        positions: positions.into(),
    })
}

fn prepare_key(
    locale: Option<&dyn LocaleService>,
    value: Option<&OsStr>,
    path: &Path,
    used: &mut usize,
    cancel: &Cancellation,
) -> Result<Option<Vec<u8>>, FsError> {
    check(cancel)?;
    let key = match (locale, value) {
        (Some(locale), Some(value)) => locale.collation_key(value)?,
        _ => None,
    };
    check(cancel)?;
    if let Some(key) = &key {
        let total = used.saturating_add(key.len());
        if key.len() <= MAX_COLLATION_KEY_BYTES && total <= KEY_BUDGET {
            *used = total;
        } else {
            return Err(find_error(
                path,
                io::ErrorKind::InvalidData,
                "Find locale keys exceed the bounded sort budget; previous order retained.",
            ));
        }
    }
    Ok(key)
}

fn filename(path: &Path) -> &OsStr {
    path.file_name().unwrap_or(path.as_os_str())
}

fn extension(
    a: &FindEntry,
    a_keys: &Keys,
    b: &FindEntry,
    b_keys: &Keys,
    descending: bool,
) -> Ordering {
    let directory = |kind| matches!(kind, EntryKind::Directory | EntryKind::DirectoryLink);
    directory(b.kind)
        .cmp(&directory(a.kind))
        .then_with(|| match (a.path.extension(), b.path.extension()) {
            (Some(a), Some(b)) => compare_collated_text(
                a_keys.primary.as_deref(),
                a,
                b_keys.primary.as_deref(),
                b,
                descending,
            ),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
        .then_with(|| {
            compare_collated_text(
                a_keys.name.as_deref(),
                filename(&a.path),
                b_keys.name.as_deref(),
                filename(&b.path),
                false,
            )
        })
        .then_with(|| filename(&a.path).cmp(filename(&b.path)))
}

fn value(path: &Path, column: FindSortColumn) -> Option<&OsStr> {
    match column {
        FindSortColumn::Name => Some(path.file_name().unwrap_or(path.as_os_str())),
        FindSortColumn::Extension => path.extension(),
        FindSortColumn::Path => Some(path.parent().unwrap_or(path).as_os_str()),
        FindSortColumn::Size | FindSortColumn::Modified => None,
    }
}
fn numeric<T: Ord>(a: Option<T>, b: Option<T>, descending: bool) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            if descending {
                b.cmp(&a)
            } else {
                a.cmp(&b)
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}
fn check(cancel: &Cancellation) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(find_error(
            Path::new(""),
            io::ErrorKind::Interrupted,
            "Find sort cancelled.",
        ))
    } else {
        Ok(())
    }
}
