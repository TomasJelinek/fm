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

fn archive_entry(id: u64, raw_name: &[u8], caption: &str, kind: EntryKind) -> Entry {
    Entry::from_archive(ArchiveNode {
        id: VirtualEntryId {
            mount: 77,
            entry: id,
        },
        parent: 1,
        raw_name: raw_name.into(),
        display_name: caption.into(),
        kind,
        size: None,
        modified: None,
        readable: kind == EntryKind::File,
    })
}

fn ids(entries: &[Entry]) -> Vec<u64> {
    entries.iter().map(|entry| entry.id.0).collect()
}

#[test]
fn archive_extension_sort_uses_raw_unicode_names_even_with_duplicate_captions_and_empty_paths() {
    for prepared in [false, true] {
        let locale = Weights::new();
        let mut entries = vec![
            archive_entry(10, b"z.h", "same.caption", EntryKind::File),
            archive_entry(20, "東京.ch".as_bytes(), "same.caption", EntryKind::File),
            archive_entry(30, b"a.i", "same.caption", EntryKind::File),
            archive_entry(40, b"opaque-\xff.h", "same.caption", EntryKind::File),
            archive_entry(50, b"missing", "same.caption", EntryKind::File),
            archive_entry(60, b"folder.i", "same.caption", EntryKind::Directory),
            archive_entry(70, "m.é".as_bytes(), "same.caption", EntryKind::File),
        ];
        if prepared {
            prepare_sort_keys(&mut entries, &locale, &Cancellation::default()).unwrap();
        }
        let calls = locale.calls.load(Ordering::Relaxed);
        for descending in [false, true] {
            sort_entries(
                &mut entries,
                SortSpec {
                    column: SortColumn::Type,
                    descending,
                },
            );
            let expected = match (prepared, descending) {
                (false, false) => [60, 20, 10, 30, 70, 40, 50],
                (false, true) => [60, 70, 30, 10, 20, 40, 50],
                (true, false) => [60, 10, 20, 30, 70, 40, 50],
                (true, true) => [60, 70, 30, 20, 10, 40, 50],
            };
            assert_eq!(
                ids(&entries),
                expected,
                "prepared={prepared}, descending={descending}"
            );
            assert!(
                entries
                    .iter()
                    .all(|entry| entry.path.as_os_str().is_empty())
            );
            let unicode = entries
                .iter()
                .find(|entry| entry.id == EntryId(20))
                .unwrap();
            assert_eq!(
                &*unicode.virtual_node.as_ref().unwrap().raw_name,
                "東京.ch".as_bytes()
            );
        }
        assert_eq!(
            locale.calls.load(Ordering::Relaxed),
            calls,
            "comparators use captured keys only"
        );
    }
}

#[test]
fn archive_dotfiles_and_trailing_dots_keep_native_extension_ordering_rules() {
    for descending in [false, true] {
        let mut entries = [
            (1, ".hidden"),
            (2, "noext"),
            (3, "trailing."),
            (4, ".hidden.txt"),
            (5, "many.parts.ch"),
            (6, "."),
            (7, ".."),
            (8, "dot..name"),
            (9, "東京.é"),
            (10, "..."),
        ]
        .into_iter()
        .map(|(id, name)| archive_entry(id, name.as_bytes(), name, EntryKind::File))
        .collect::<Vec<_>>();
        sort_entries(
            &mut entries,
            SortSpec {
                column: SortColumn::Type,
                descending,
            },
        );
        assert_eq!(
            ids(&entries),
            if descending {
                [9, 4, 8, 5, 10, 3, 6, 7, 1, 2]
            } else {
                [10, 3, 5, 8, 4, 9, 6, 7, 1, 2]
            }
        );
    }
}

#[test]
fn archive_equal_extensions_use_captured_name_weights_then_stable_identity_ties() {
    let locale = Weights::new();
    for descending in [false, true] {
        let mut entries = vec![
            archive_entry(5, b"raw-z.h", "i", EntryKind::File),
            archive_entry(3, b"raw-a.h", "ch", EntryKind::File),
            archive_entry(2, b"raw-b.h", "h", EntryKind::File),
            archive_entry(1, b"raw-c.h", "ch", EntryKind::File),
        ];
        prepare_sort_keys(&mut entries, &locale, &Cancellation::default()).unwrap();
        sort_entries(
            &mut entries,
            SortSpec {
                column: SortColumn::Type,
                descending,
            },
        );
        assert_eq!(ids(&entries), [2, 1, 3, 5]);
    }
}

#[test]
fn native_raw_extensions_keep_unavailable_locale_keys_last_in_both_directions() {
    let mut invalid_extension = std::ffi::OsString::from("entry.");
    invalid_extension.push(native_fixture::invalid_name());
    let mut invalid_basename = native_fixture::invalid_name();
    invalid_basename.push(".ch");
    for descending in [false, true] {
        let mut entries = vec![
            Entry::new(
                EntryId(1),
                PathBuf::from("root").join(&invalid_extension),
                EntryKind::File,
            ),
            entry(2, "visible.h", EntryKind::File),
            Entry::new(
                EntryId(3),
                PathBuf::from("root").join(&invalid_basename),
                EntryKind::File,
            ),
        ];
        for entry in &mut entries {
            entry.display_name = "caption.i".into();
        }
        prepare_sort_keys(&mut entries, &Weights::new(), &Cancellation::default()).unwrap();
        sort_entries(
            &mut entries,
            SortSpec {
                column: SortColumn::Type,
                descending,
            },
        );
        assert_eq!(
            ids(&entries),
            if descending { [3, 2, 1] } else { [2, 3, 1] }
        );
        assert_eq!(entries[2].name, invalid_extension);
    }
}
