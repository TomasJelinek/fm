//! Backend-only bounded Find ordering. Discovery-order result IDs stay authoritative.
use midnight_core::*;
use std::{
    cmp::Ordering,
    collections::HashMap,
    ffi::OsStr,
    io,
    path::{Component, Path},
    sync::Arc,
};

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
    let descending = sort.descending;
    let mut indices = match sort.column {
        FindSortColumn::Name => sorted(
            results,
            keys,
            |entry, keys| {
                (
                    keys.primary.as_deref(),
                    value(&entry.path, sort.column).unwrap_or_default(),
                )
            },
            |a, b| compare_collated_text(a.0, a.1, b.0, b.1, descending),
        ),
        // Results of one folder tie on it: their names decide (`TiePath`).
        FindSortColumn::Path => sorted(
            results,
            keys,
            |entry, keys| {
                (
                    keys.primary.as_deref(),
                    value(&entry.path, sort.column).unwrap_or_default(),
                    TiePath::new(&entry.path),
                )
            },
            |a, b| {
                compare_collated_text(a.0, a.1, b.0, b.1, descending).then_with(|| a.2.cmp(&b.2))
            },
        ),
        FindSortColumn::Extension => sorted(results, keys, ExtensionFacts::new, |a, b| {
            a.compare(b, descending)
        }),
        FindSortColumn::Size => sorted(
            results,
            keys,
            |entry, _| entry.metadata.size,
            |a, b| numeric(*a, *b, descending),
        ),
        FindSortColumn::Modified => sorted(
            results,
            keys,
            |entry, _| entry.metadata.modified,
            |a, b| numeric(*a, *b, descending),
        ),
    };
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

/// Return the positions of `results` in sorted order. Each result's facts are derived
/// once, so comparisons neither parse paths nor follow result pointers. Ties fall back
/// to the path and then discovery order (a position is its result's ID minus one).
fn sorted<'a, P>(
    results: &'a [Arc<FindResult>],
    keys: &'a [Keys],
    facts: impl Fn(&'a FindEntry, &'a Keys) -> P,
    compare: impl Fn(&P, &P) -> Ordering,
) -> Vec<usize> {
    let mut rows: Vec<_> = results
        .iter()
        .zip(keys)
        .enumerate()
        .map(|(position, (result, keys))| {
            (
                facts(&result.entry, keys),
                result.entry.path.as_path(),
                position,
            )
        })
        .collect();
    rows.sort_unstable_by(|left, right| {
        compare(&left.0, &right.0)
            .then_with(|| left.1.cmp(right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    rows.into_iter().map(|(_, _, position)| position).collect()
}

/// A path ordered exactly as `Path::cmp` orders it: component by component. A path
/// ending in a name keeps its parent's bytes and that name. Two paths with the same
/// parent bytes have the same leading components, so their names decide without
/// walking the parent's components again (on Windows `Path::cmp` has no byte shortcut
/// for a `C:` prefix). Splitting costs a parse per result, so only the Path column,
/// whose ties fall within one folder, uses it.
struct TiePath<'a> {
    path: &'a Path,
    parent_and_name: Option<(&'a OsStr, &'a OsStr)>,
}

impl<'a> TiePath<'a> {
    fn new(path: &'a Path) -> Self {
        let mut components = path.components();
        let parent_and_name = match components.next_back() {
            // What `Path::parent` returns, the components left before the name.
            Some(Component::Normal(name)) => Some((components.as_path().as_os_str(), name)),
            _ => None,
        };
        Self {
            path,
            parent_and_name,
        }
    }

    fn cmp(&self, other: &Self) -> Ordering {
        match (self.parent_and_name, other.parent_and_name) {
            (Some((parent, name)), Some((other_parent, other_name))) if parent == other_parent => {
                name.cmp(other_name)
            }
            _ => self.path.cmp(other.path),
        }
    }
}

fn filename(path: &Path) -> &OsStr {
    path.file_name().unwrap_or(path.as_os_str())
}

/// What the Extension column compares: folders first, then the extension (missing
/// last), then the file name.
struct ExtensionFacts<'a> {
    directory: bool,
    extension: Option<(&'a OsStr, Option<&'a [u8]>)>,
    name: &'a OsStr,
    name_key: Option<&'a [u8]>,
}

impl<'a> ExtensionFacts<'a> {
    fn new(entry: &'a FindEntry, keys: &'a Keys) -> Self {
        Self {
            directory: matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink),
            extension: entry
                .path
                .extension()
                .map(|extension| (extension, keys.primary.as_deref())),
            name: filename(&entry.path),
            name_key: keys.name.as_deref(),
        }
    }

    fn compare(&self, other: &Self, descending: bool) -> Ordering {
        other
            .directory
            .cmp(&self.directory)
            .then_with(|| match (self.extension, other.extension) {
                (Some((a, a_key)), Some((b, b_key))) => {
                    compare_collated_text(a_key, a, b_key, b, descending)
                }
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            })
            .then_with(|| {
                compare_collated_text(self.name_key, self.name, other.name_key, other.name, false)
            })
            .then_with(|| self.name.cmp(other.name))
    }
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
