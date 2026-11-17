//! Explicit, opt-in measurement of 100,000-row listing work; it asserts nothing about time.
//! Run serially on an otherwise idle machine:
//! `cargo test -p midnight-core --release --test integration listing_performance -- --ignored --nocapture --test-threads=1`
use midnight_core::*;
use std::{
    collections::HashMap,
    ffi::OsStr,
    hint::black_box,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, UNIX_EPOCH},
};

const ROWS: usize = 100_000;
const ROUNDS: usize = 5;
const EXTENSIONS: [&str; 10] = [
    "txt", "rs", "PNG", "jpeg", "tar.gz", "pdf", "docx", "", "md", "zip",
];

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

/// One xorshift value per row, so every run measures the same listing.
fn row_states(seed: u64) -> impl Iterator<Item = u64> {
    let mut state = seed;
    std::iter::repeat_with(move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    })
    .take(ROWS)
}

fn file_name(prefix: &str, state: u64) -> String {
    match EXTENSIONS[(state >> 8) as usize % EXTENSIONS.len()] {
        "" => format!("{prefix} {state:016x}"),
        extension => format!("{prefix} {state:016x}.{extension}"),
    }
}

fn row_kind(state: u64) -> EntryKind {
    if state.is_multiple_of(10) {
        EntryKind::Directory
    } else {
        EntryKind::File
    }
}

fn rows() -> Vec<(PathBuf, EntryKind, Option<u64>, u64)> {
    let root = std::env::temp_dir().join("midnight-listing-measurement");
    row_states(0x2545_f491_4f6c_dd1d)
        .map(|state| {
            let size = (!state.is_multiple_of(7)).then_some(state >> 20);
            let path = root.join(file_name("Name", state));
            (path, row_kind(state), size, state >> 34)
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

/// Members as an archive provider lists them: provider basenames (about 1% in a
/// legacy encoding that is not UTF-8), optional sizes and UTC times.
fn archive_nodes() -> Vec<ArchiveNode> {
    row_states(0x9e37_79b9_7f4a_7c15)
        .zip(2_u64..)
        .map(|(state, entry)| {
            let display_name = file_name("Member", state);
            let mut raw_name = display_name.clone().into_bytes();
            if state.is_multiple_of(97) {
                raw_name[0] = 0xff;
            }
            let kind = row_kind(state);
            ArchiveNode {
                id: VirtualEntryId { mount: 7, entry },
                parent: 1,
                raw_name: raw_name.into(),
                display_name,
                kind,
                size: (kind == EntryKind::File && !state.is_multiple_of(7)).then_some(state >> 20),
                readable: true,
                modified: (!state.is_multiple_of(11)).then_some(ArchiveTimestamp::Utc {
                    seconds: (state >> 34) as i64,
                    nanoseconds: 0,
                }),
            }
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
    println!("{label:<48} {:>9.3} ms", elapsed.as_secs_f64() * 1000.0);
}

/// Calculated sizes as a backend size map supplies them to `sort_entries_with_sizes`.
fn size_map(entries: &[Entry]) -> HashMap<EntryId, u64> {
    entries
        .iter()
        .filter_map(|entry| Some((entry.id, entry.metadata.size?)))
        .collect()
}

/// Every column of the raw and the locale-keyed listing, then Size through `sizes`.
fn report_sorts(prefix: &str, plain: &[Entry], keyed: &[Entry]) {
    for (label, entries) in [("raw", plain), ("keyed", keyed)] {
        for column in [
            SortColumn::Name,
            SortColumn::Size,
            SortColumn::Modified,
            SortColumn::Type,
        ] {
            report(
                &format!("{prefix}sort {label} {column:?}"),
                best(
                    || entries.to_vec(),
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
    let sizes = size_map(plain);
    report(
        &format!("{prefix}sort raw Size via map lookup"),
        best(
            || plain.to_vec(),
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
}

fn keyed(entries: &[Entry]) -> Vec<Entry> {
    let mut keyed = entries.to_vec();
    prepare_sort_keys(&mut keyed, &LowercaseKeys, &Cancellation::default())
        .expect("measurement keys");
    keyed
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
    report("drop Vec<Entry>", best(|| plain.clone(), drop));
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
    report_sorts("", &plain, &keyed(&plain));
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

/// Archive rows box their provider node: one more allocation per build and per
/// clone than native rows, against a smaller row for every sort and copy.
#[test]
#[ignore = "explicit release-mode measurement of 100,000 archive member rows"]
fn archive_listing_operations_on_one_hundred_thousand_rows() {
    println!(
        "size_of::<Entry>() = {} bytes, size_of::<ArchiveNode>() = {} bytes",
        std::mem::size_of::<Entry>(),
        std::mem::size_of::<ArchiveNode>()
    );
    let nodes = archive_nodes();
    report(
        "archive Entry::from_archive x100k",
        best(
            || nodes.clone(),
            |nodes| {
                nodes
                    .into_iter()
                    .map(Entry::from_archive)
                    .collect::<Vec<_>>()
            },
        ),
    );
    let members: Vec<Entry> = nodes.iter().cloned().map(Entry::from_archive).collect();
    report(
        "archive clone Vec<Entry>",
        best(|| (), |()| members.clone()),
    );
    report("archive drop Vec<Entry>", best(|| members.clone(), drop));
    report(
        "archive DirectoryIndex::from_entries",
        best(|| (), |()| DirectoryIndex::from_entries(&members)),
    );
    report_sorts("archive ", &members, &keyed(&members));
    // The engine's archive navigation once `ArchiveMount::list` has returned the
    // nodes, minus the per-row timestamp text and collation keys that cost the same
    // for either row layout: build rows, sort them with the engine's call (Name, the
    // default column, never consults the size map), index them and share them.
    let sizes = size_map(&members);
    report(
        "archive listing: rows, Name sort, index, share",
        best(
            || nodes.clone(),
            |nodes| {
                let mut entries: Vec<_> = nodes.into_iter().map(Entry::from_archive).collect();
                sort_entries_with_sizes(&mut entries, SortSpec::default(), |entry| {
                    sizes.get(&entry.id).copied()
                });
                let index = DirectoryIndex::from_entries(&entries);
                (Arc::<[Entry]>::from(entries), index)
            },
        ),
    );
}
