//! Explicit, opt-in measurement of ordering the Find budget of 100,000 results; it
//! asserts nothing about time. Run serially on an otherwise idle machine:
//! `cargo test -p midnight-engine --release --lib find_sort_bench -- --ignored --nocapture --test-threads=1`
use crate::find_sort::{KeyCache, order};
use midnight_core::*;
use std::{
    ffi::OsStr,
    hint::black_box,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, UNIX_EPOCH},
};

const RESULTS: usize = 100_000;
const ROUNDS: usize = 7;
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

/// A recursive search's results: 1,000 folders five levels below the root, about
/// 100 results in each, in discovery order (folder by folder).
fn results() -> Vec<Arc<FindResult>> {
    let root = std::env::temp_dir().join("midnight-find-measurement");
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    (0..RESULTS)
        .map(|index| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let folder = index / 100;
            let name = match EXTENSIONS[(state >> 8) as usize % EXTENSIONS.len()] {
                "" => format!("Name {state:016x}"),
                extension => format!("Name {state:016x}.{extension}"),
            };
            let path: PathBuf = root
                .join("Projects")
                .join(format!("Area {:02}", folder / 100))
                .join(format!("Folder {:03}", folder % 100))
                .join("Sources")
                .join(name);
            Arc::new(FindResult {
                id: FindResultId(index as u64 + 1),
                duplicate_group: None,
                duplicate_group_size: 0,
                archive: None,
                entry: FindEntry {
                    path,
                    kind: if state.is_multiple_of(10) {
                        EntryKind::Directory
                    } else {
                        EntryKind::File
                    },
                    identity: TransferIdentity {
                        device: 1,
                        object: [0; 16],
                    },
                    revision: FindRevision([0; 6]),
                    metadata: EntryMetadata {
                        size: (!state.is_multiple_of(7)).then_some(state >> 20),
                        modified: UNIX_EPOCH.checked_add(Duration::from_secs(state >> 34)),
                        ..Default::default()
                    },
                },
            })
        })
        .collect()
}

/// Best and median of several rounds; input preparation and destruction are not timed.
fn rounds<I, T>(mut input: impl FnMut() -> I, mut run: impl FnMut(I) -> T) -> (f64, f64) {
    let mut times: Vec<f64> = (0..ROUNDS)
        .map(|_| {
            let input = input();
            let start = Instant::now();
            let output = run(input);
            let elapsed = start.elapsed();
            drop(black_box(output));
            elapsed.as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(f64::total_cmp);
    (times[0], times[ROUNDS / 2])
}

fn report(label: &str, (best, median): (f64, f64)) {
    println!("{label:<44} best {best:>9.3} ms  median {median:>9.3} ms");
}

#[test]
#[ignore = "explicit release-mode measurement of ordering 100,000 Find results"]
fn find_sort_bench_one_hundred_thousand_results() {
    let results = results();
    let cancel = Cancellation::default();
    let locale: &dyn LocaleService = &LowercaseKeys;
    for column in [
        FindSortColumn::Name,
        FindSortColumn::Extension,
        FindSortColumn::Path,
        FindSortColumn::Size,
        FindSortColumn::Modified,
    ] {
        let sort = FindSortSpec {
            column,
            descending: false,
        };
        report(
            &format!("{column:?} first sort (keys prepared)"),
            rounds(KeyCache::default, |mut cache| {
                order(&results, sort, 1, Some(locale), &cancel, &mut cache).unwrap()
            }),
        );
        let mut warm = KeyCache::default();
        order(&results, sort, 1, Some(locale), &cancel, &mut warm).unwrap();
        report(
            &format!("{column:?} re-sort (keys cached)"),
            rounds(
                || (),
                |()| order(&results, sort, 1, Some(locale), &cancel, &mut warm).unwrap(),
            ),
        );
        report(
            &format!("{column:?} sort without locale keys"),
            rounds(KeyCache::default, |mut cache| {
                order(&results, sort, 1, None, &cancel, &mut cache).unwrap()
            }),
        );
    }
    report(
        "snapshot Vec<Arc<FindResult>> (clone)",
        rounds(|| (), |()| results.clone()),
    );
    report("release snapshot (drop)", rounds(|| results.clone(), drop));
}
