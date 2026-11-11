#[path = "../src/find_sort.rs"]
mod find_sort;
use midnight_core::*;
use std::{
    ffi::OsStr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

struct ReverseLocale;
impl LocaleService for ReverseLocale {
    fn number_locale(&self) -> Result<NumberLocale, FsError> {
        Ok(NumberLocale::default())
    }
    fn collation_key(&self, value: &OsStr) -> Result<Option<Vec<u8>>, FsError> {
        Ok(value.to_str().map(|value| {
            value
                .bytes()
                .map(|byte| 255 - byte.to_ascii_lowercase())
                .collect()
        }))
    }
}
fn results() -> Vec<Arc<FindResult>> {
    [
        ("A/a.toml", Some(100), Some(10)),
        ("z/z.rs", Some(2), Some(20)),
        ("B/b.rs", None, None),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (path, size, seconds))| {
        Arc::new(FindResult {
            id: FindResultId(index as u64 + 1),
            duplicate_group: None,
            duplicate_group_size: 0,
            entry: FindEntry {
                path: PathBuf::from(path),
                kind: EntryKind::File,
                identity: TransferIdentity {
                    device: 1,
                    object: [index as u8; 16],
                },
                revision: FindRevision([0; 6]),
                metadata: EntryMetadata {
                    size,
                    modified: seconds.map(|seconds| UNIX_EPOCH + Duration::from_secs(seconds)),
                    ..Default::default()
                },
            },
        })
    })
    .collect()
}
fn sorted(column: FindSortColumn, descending: bool) -> FindOrder {
    find_sort::order(
        &results(),
        FindSortSpec { column, descending },
        7,
        Some(&ReverseLocale),
        &Cancellation::default(),
    )
    .unwrap()
}
fn ids(order: &FindOrder) -> Vec<u64> {
    order.ids.iter().map(|id| id.0).collect()
}

#[test]
fn every_duplicate_sort_keeps_group_members_adjacent() {
    let template = results().remove(0);
    let rows: Vec<_> = [("a.txt", 1), ("z.txt", 1), ("b.txt", 2), ("c.txt", 2)]
        .into_iter()
        .enumerate()
        .map(|(index, (name, group))| {
            let mut row = (*template).clone();
            row.id = FindResultId(index as u64 + 1);
            row.entry.path = PathBuf::from(name);
            row.duplicate_group = Some(group);
            row.duplicate_group_size = 2;
            Arc::new(row)
        })
        .collect();
    for descending in [false, true] {
        let ordered = find_sort::order(
            &rows,
            FindSortSpec {
                column: FindSortColumn::Name,
                descending,
            },
            1,
            None,
            &Cancellation::default(),
        )
        .unwrap();
        assert_eq!(
            ids(&ordered),
            if descending {
                vec![2, 1, 4, 3]
            } else {
                vec![1, 2, 3, 4]
            }
        );
        for pair in ordered.ids.as_chunks::<2>().0 {
            assert_eq!(
                rows[pair[0].0 as usize - 1].duplicate_group,
                rows[pair[1].0 as usize - 1].duplicate_group
            );
        }
    }
}

#[test]
fn missing_locale_keys_stay_last_in_both_name_and_path_directions() {
    struct MissingKey;
    impl LocaleService for MissingKey {
        fn number_locale(&self) -> Result<NumberLocale, FsError> {
            Ok(NumberLocale::default())
        }
        fn collation_key(&self, value: &OsStr) -> Result<Option<Vec<u8>>, FsError> {
            Ok(value
                .to_str()
                .filter(|text| !text.starts_with('z'))
                .map(|text| text.as_bytes().to_vec()))
        }
    }
    for column in [FindSortColumn::Name, FindSortColumn::Path] {
        for descending in [false, true] {
            let order = find_sort::order(
                &results(),
                FindSortSpec { column, descending },
                1,
                Some(&MissingKey),
                &Cancellation::default(),
            )
            .unwrap();
            assert_eq!(
                order.ids.last(),
                Some(&FindResultId(2)),
                "Missing keys must remain after known keys for {column:?}, descending={descending}"
            );
        }
    }
}

#[test]
fn locale_name_path_and_extension_order_do_not_follow_raw_discovery_order() {
    assert_eq!(ids(&sorted(FindSortColumn::Name, false)), [2, 3, 1]);
    assert_eq!(ids(&sorted(FindSortColumn::Path, false)), [2, 3, 1]);
    assert_eq!(ids(&sorted(FindSortColumn::Extension, false)), [1, 2, 3]);
    let order = sorted(FindSortColumn::Name, false);
    for (row, id) in order.ids.iter().enumerate() {
        assert_eq!(order.positions[id.0 as usize - 1], row);
    }
    assert_eq!(
        results().iter().map(|r| r.id.0).collect::<Vec<_>>(),
        [1, 2, 3]
    );
}
#[test]
fn numeric_size_and_exact_time_sort_both_directions_with_unknowns_last() {
    assert_eq!(ids(&sorted(FindSortColumn::Size, false)), [2, 1, 3]);
    assert_eq!(ids(&sorted(FindSortColumn::Size, true)), [1, 2, 3]);
    assert_eq!(ids(&sorted(FindSortColumn::Modified, false)), [1, 2, 3]);
    assert_eq!(ids(&sorted(FindSortColumn::Modified, true)), [2, 1, 3]);
}
#[test]
fn cancelled_sort_never_publishes_an_order() {
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(
        find_sort::order(
            &results(),
            FindSortSpec::default(),
            1,
            Some(&ReverseLocale),
            &cancel
        )
        .is_err()
    );
}
