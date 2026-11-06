use midnight_core::*;
use std::{
    ffi::OsStr,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, UNIX_EPOCH},
};

#[cfg(unix)]
#[path = "support/locale_unix.rs"]
mod native_fixture;
#[cfg(windows)]
#[path = "support/locale_windows.rs"]
mod native_fixture;

struct Weights {
    calls: AtomicUsize,
}
impl Weights {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}
impl LocaleService for Weights {
    fn number_locale(&self) -> Result<NumberLocale, FsError> {
        Ok(Default::default())
    }
    fn collation_key(&self, value: &OsStr) -> Result<Option<Vec<u8>>, FsError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(value.to_str().map(|name| match name {
            "h" => vec![1],
            "ch" => vec![2],
            "i" => vec![3],
            "é" | "e\u{301}" => vec![4],
            _ => name.as_bytes().to_vec(),
        }))
    }
}
fn entry(id: u64, name: &str, kind: EntryKind) -> Entry {
    Entry::new(EntryId(id), PathBuf::from("root").join(name), kind)
}
fn names(entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| entry.name.to_str().unwrap().to_owned())
        .collect()
}

#[test]
fn precomputed_keys_order_names_extensions_and_equal_metadata_without_lossy_labels() {
    let locale = Weights::new();
    let mut entries = vec![
        entry(1, "ch", EntryKind::File),
        entry(2, "i", EntryKind::File),
        entry(3, "h", EntryKind::File),
    ];
    for item in &mut entries {
        item.display_name = "unrelated display label".into();
        item.metadata.size = Some(17);
        item.metadata.modified = Some(UNIX_EPOCH + Duration::from_secs(8));
    }
    prepare_sort_keys(&mut entries, &locale, &Cancellation::default()).unwrap();
    for column in [SortColumn::Name, SortColumn::Size, SortColumn::Modified] {
        sort_entries(
            &mut entries,
            SortSpec {
                column,
                descending: false,
            },
        );
        assert_eq!(names(&entries), ["h", "ch", "i"]);
    }
    sort_entries(
        &mut entries,
        SortSpec {
            column: SortColumn::Name,
            descending: true,
        },
    );
    assert_eq!(names(&entries), ["i", "ch", "h"]);
    let mut extensions = vec![
        entry(1, "one.ch", EntryKind::File),
        entry(2, "two.h", EntryKind::File),
        entry(3, "three.i", EntryKind::File),
        entry(4, "missing", EntryKind::File),
    ];
    prepare_sort_keys(&mut extensions, &locale, &Cancellation::default()).unwrap();
    sort_entries(
        &mut extensions,
        SortSpec {
            column: SortColumn::Type,
            descending: false,
        },
    );
    assert_eq!(
        names(&extensions),
        ["two.h", "one.ch", "three.i", "missing"]
    );
    sort_entries(
        &mut extensions,
        SortSpec {
            column: SortColumn::Type,
            descending: true,
        },
    );
    assert_eq!(
        names(&extensions),
        ["three.i", "one.ch", "two.h", "missing"]
    );
}

#[test]
fn cached_keys_survive_clones_and_resorts_with_directories_first_and_unknown_sizes_last() {
    let locale = Weights::new();
    let mut entries = vec![
        entry(1, "ch", EntryKind::File),
        entry(2, "i", EntryKind::File),
        entry(3, "h", EntryKind::DirectoryLink),
    ];
    entries[0].metadata.size = Some(9);
    prepare_sort_keys(&mut entries, &locale, &Cancellation::default()).unwrap();
    let calls = locale.calls.load(Ordering::Relaxed);
    for descending in [false, true] {
        let mut cloned = entries.clone();
        prepare_sort_keys(&mut cloned, &locale, &Cancellation::default()).unwrap();
        sort_entries(
            &mut cloned,
            SortSpec {
                column: SortColumn::Size,
                descending,
            },
        );
        assert_eq!(names(&cloned), ["h", "ch", "i"]);
    }
    assert_eq!(
        locale.calls.load(Ordering::Relaxed),
        calls,
        "re-sorts must reuse captured keys"
    );
}

#[test]
fn equal_native_weights_use_raw_names_and_ids_as_deterministic_ties() {
    let locale = Weights::new();
    for descending in [false, true] {
        let mut entries = vec![
            entry(5, "é", EntryKind::File),
            entry(2, "e\u{301}", EntryKind::File),
            entry(1, "e\u{301}", EntryKind::File),
        ];
        prepare_sort_keys(&mut entries, &locale, &Cancellation::default()).unwrap();
        sort_entries(
            &mut entries,
            SortSpec {
                column: SortColumn::Name,
                descending,
            },
        );
        assert_eq!(
            entries.iter().map(|entry| entry.id.0).collect::<Vec<_>>(),
            [1, 2, 5]
        );
    }
}

struct Stops {
    cancel: Cancellation,
    calls: AtomicUsize,
}
impl LocaleService for Stops {
    fn number_locale(&self) -> Result<NumberLocale, FsError> {
        Ok(Default::default())
    }
    fn collation_key(&self, _: &OsStr) -> Result<Option<Vec<u8>>, FsError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.cancel.cancel();
        Ok(Some(vec![1]))
    }
}
#[test]
fn cancellation_stops_preparation_between_native_calls_and_does_not_commit_the_cancelled_key() {
    let cancel = Cancellation::default();
    let stopping = Stops {
        cancel: cancel.clone(),
        calls: AtomicUsize::new(0),
    };
    let mut entries = vec![
        entry(1, "h", EntryKind::File),
        entry(2, "i", EntryKind::File),
    ];
    assert_eq!(
        prepare_sort_keys(&mut entries, &stopping, &cancel)
            .unwrap_err()
            .kind,
        std::io::ErrorKind::Interrupted
    );
    assert_eq!(stopping.calls.load(Ordering::Relaxed), 1);
    let locale = Weights::new();
    prepare_sort_keys(&mut entries, &locale, &Cancellation::default()).unwrap();
    assert_eq!(locale.calls.load(Ordering::Relaxed), 2);
}

#[test]
fn invalid_native_names_remain_distinct_and_follow_collatable_names_in_either_direction() {
    let invalid = native_fixture::invalid_name();
    for descending in [false, true] {
        let mut entries = vec![
            Entry::new(
                EntryId(1),
                PathBuf::from("root").join(&invalid),
                EntryKind::File,
            ),
            entry(2, "h", EntryKind::File),
        ];
        prepare_sort_keys(&mut entries, &Weights::new(), &Cancellation::default()).unwrap();
        sort_entries(
            &mut entries,
            SortSpec {
                column: SortColumn::Name,
                descending,
            },
        );
        assert_eq!(entries[0].name, OsStr::new("h"));
        assert_eq!(entries[1].name, invalid);
    }
}

struct Oversized;
impl LocaleService for Oversized {
    fn number_locale(&self) -> Result<NumberLocale, FsError> {
        Ok(Default::default())
    }
    fn collation_key(&self, _: &OsStr) -> Result<Option<Vec<u8>>, FsError> {
        Ok(Some(vec![1; MAX_COLLATION_KEY_BYTES + 1]))
    }
}
#[test]
fn generated_key_resource_limit_reports_the_exact_entry_without_truncation() {
    let mut entries = vec![entry(1, "literal 文档", EntryKind::File)];
    let original = entries[0].path.clone();
    let error = prepare_sort_keys(&mut entries, &Oversized, &Cancellation::default()).unwrap_err();
    assert_eq!(error.kind, std::io::ErrorKind::InvalidData);
    assert_eq!(error.path, original);
    assert_eq!(entries[0].path, original);
}
