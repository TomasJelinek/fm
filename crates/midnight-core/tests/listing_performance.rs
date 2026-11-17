//! Explicit, opt-in measurement of 100,000-row listing work; it asserts nothing about time.
//! Run serially on an otherwise idle machine:
//! `cargo test -p midnight-core --release --test integration listing_performance -- --ignored --nocapture --test-threads=1`
use midnight_core::*;
use std::{
    collections::HashMap,
    ffi::OsStr,
    hint::black_box,
    path::PathBuf,
    time::{Duration, Instant, UNIX_EPOCH},
};

const ROWS: usize = 100_000;
const ROUNDS: usize = 5;

/// Deterministic stand-in for native collation keys; no OS call is made.
struct LowercaseKeys;
impl LocaleService for LowercaseKeys {
    fn number_locale(&self) -> Result<NumberLocale, FsError> {
        Ok(Default::default())
    }
    fn collation_key(&self, value: &OsStr) -> Result<Option<Vec<u8>>, FsError> {
        Ok(value.to_str().map(|text| text.to_lowercase().into_bytes()))
    }
}

fn rows() -> Vec<(PathBuf, EntryKind, Option<u64>, u64)> {
    let root = std::env::temp_dir().join("midnight-listing-measurement");
    let extensions = [
        "txt", "rs", "PNG", "jpeg", "tar.gz", "pdf", "docx", "", "md", "zip",
    ];
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    (0..ROWS)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let kind = if state.is_multiple_of(10) {
                EntryKind::Directory
            } else {
                EntryKind::File
            };
            let extension = extensions[(state >> 8) as usize % extensions.len()];
            let name = if extension.is_empty() {
                format!("Name {state:016x}")
            } else {
                format!("Name {state:016x}.{extension}")
            };
            let size = (!state.is_multiple_of(7)).then_some(state >> 20);
            (root.join(name), kind, size, state >> 34)
        })
        .collect()
}

fn listing(rows: &[(PathBuf, EntryKind, Option<u64>, u64)]) -> Vec<Entry> {
    rows.iter()
        .enumerate()
        .map(|(row, (path, kind, size, seconds))| {
            let mut entry = Entry::new(EntryId(row as u64 + 1), path.clone(), *kind);
            entry.metadata.size = *size;
            entry.metadata.modified = UNIX_EPOCH.checked_add(Duration::from_secs(*seconds));
            entry
        })
        .collect()
}

/// Best of several rounds; input preparation and result destruction are not timed.
fn best<I, T>(mut input: impl FnMut() -> I, mut run: impl FnMut(I) -> T) -> Duration {
    (0..ROUNDS)
        .map(|_| {
            let input = input();
            let start = Instant::now();
            let output = run(input);
            let elapsed = start.elapsed();
            drop(black_box(output));
            elapsed
        })
        .min()
        .unwrap_or_default()
}

fn report(label: &str, elapsed: Duration) {
    println!("{label:<44} {:>9.3} ms", elapsed.as_secs_f64() * 1000.0);
}

#[test]
#[ignore = "explicit release-mode measurement of 100,000-row listing work"]
fn listing_operations_on_one_hundred_thousand_rows() {
    println!(
        "size_of::<Entry>() = {} bytes",
        std::mem::size_of::<Entry>()
    );
    let rows = rows();
    report(
        "Entry::new x100k",
        best(
            || {
                rows.iter()
                    .map(|(path, kind, ..)| (path.clone(), *kind))
                    .collect::<Vec<_>>()
            },
            |paths| {
                paths
                    .into_iter()
                    .enumerate()
                    .map(|(row, (path, kind))| Entry::new(EntryId(row as u64), path, kind))
                    .collect::<Vec<_>>()
            },
        ),
    );
    let plain = listing(&rows);
    report("clone Vec<Entry>", best(|| (), |()| plain.clone()));
    report(
        "DirectoryIndex::from_entries",
        best(|| (), |()| DirectoryIndex::from_entries(&plain)),
    );
    let mut localized = plain.clone();
    for (row, entry) in localized.iter_mut().enumerate() {
        entry.metadata.localized_modified = Some(LocalizedDateTime {
            date: format!("{}. 11. 2026", row % 28 + 1),
            time: format!("{}:{:02}", row % 24, row % 60),
        });
    }
    report(
        "DirectoryIndex::from_entries localized",
        best(|| (), |()| DirectoryIndex::from_entries(&localized)),
    );
    drop(localized);
    let mut keyed = plain.clone();
    prepare_sort_keys(&mut keyed, &LowercaseKeys, &Cancellation::default())
        .expect("measurement keys");
    for (label, entries) in [("raw", &plain), ("keyed", &keyed)] {
        for column in [
            SortColumn::Name,
            SortColumn::Size,
            SortColumn::Modified,
            SortColumn::Type,
        ] {
            report(
                &format!("sort {label} {column:?}"),
                best(
                    || entries.clone(),
                    |mut entries| {
                        sort_entries(
                            &mut entries,
                            SortSpec {
                                column,
                                descending: false,
                            },
                        );
                        entries
                    },
                ),
            );
        }
    }
    let sizes: HashMap<EntryId, u64> = plain
        .iter()
        .filter_map(|entry| Some((entry.id, entry.metadata.size?)))
        .collect();
    report(
        "sort raw Size via map lookup",
        best(
            || plain.clone(),
            |mut entries| {
                sort_entries_with_sizes(
                    &mut entries,
                    SortSpec {
                        column: SortColumn::Size,
                        descending: false,
                    },
                    |entry| sizes.get(&entry.id).copied(),
                );
                entries
            },
        ),
    );
    let digests: Vec<ChecksumDigest> = (0..16_384_u32)
        .flat_map(|row| {
            [
                (ChecksumAlgorithm::Crc32, 4),
                (ChecksumAlgorithm::Md5, 16),
                (ChecksumAlgorithm::Sha1, 20),
                (ChecksumAlgorithm::Sha256, 32),
                (ChecksumAlgorithm::Sha512, 64),
            ]
            .map(|(algorithm, length)| ChecksumDigest {
                algorithm,
                bytes: (0..length)
                    .map(|byte| (row as u8).wrapping_add(byte))
                    .collect(),
            })
        })
        .collect();
    report(
        "ChecksumDigest::hex x81,920",
        best(
            || (),
            |()| digests.iter().map(ChecksumDigest::hex).collect::<Vec<_>>(),
        ),
    );
    let preferences = UserPreferences::default();
    report(
        "UserPreferences::validate (defaults)",
        best(|| (), |()| preferences.validate().is_ok()),
    );
}
