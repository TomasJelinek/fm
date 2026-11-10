#[path = "../src/find_sort.rs"]
mod find_sort;

use midnight_core::*;
use std::{ffi::OsStr, path::PathBuf, sync::Arc};

struct Weights;
impl LocaleService for Weights {
    fn number_locale(&self) -> Result<NumberLocale, FsError> {
        Ok(NumberLocale::default())
    }
    fn collation_key(&self, value: &OsStr) -> Result<Option<Vec<u8>>, FsError> {
        Ok(value.to_str().and_then(|value| match value {
            "raw" => None,
            "h" => Some(vec![1]),
            "ch" => Some(vec![2]),
            "i" => Some(vec![3]),
            _ => Some(value.to_lowercase().into_bytes()),
        }))
    }
}

fn fixture() -> Vec<Arc<FindResult>> {
    [
        ("root/no-extension", EntryKind::File),
        ("root/A.ch", EntryKind::File),
        ("root/z.h", EntryKind::File),
        ("root/b.h", EntryKind::File),
        ("root/fallback.raw", EntryKind::File),
        ("root/first.i", EntryKind::Directory),
        ("root/directory-link", EntryKind::DirectoryLink),
        ("root/.hidden", EntryKind::File),
        ("root/final.", EntryKind::File),
        ("root/日本語.h", EntryKind::File),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (path, kind))| {
        Arc::new(FindResult {
            id: FindResultId(index as u64 + 1),
            duplicate_group: None,
            duplicate_group_size: 0,
            entry: FindEntry {
                path: PathBuf::from(path),
                kind,
                metadata: EntryMetadata::default(),
                identity: TransferIdentity {
                    device: 1,
                    object: [index as u8; 16],
                },
                revision: FindRevision([0; 6]),
            },
        })
    })
    .collect()
}

fn assert_panel_parity(locale: &dyn LocaleService) {
    let results = fixture();
    let mut panel: Vec<_> = results
        .iter()
        .map(|result| {
            Entry::new(
                EntryId(result.id.0),
                result.entry.path.clone(),
                result.entry.kind,
            )
        })
        .collect();
    prepare_sort_keys(&mut panel, locale, &Cancellation::default()).unwrap();
    for descending in [false, true] {
        sort_entries(
            &mut panel,
            SortSpec {
                column: SortColumn::Type,
                descending,
            },
        );
        let order = find_sort::order(
            &results,
            FindSortSpec {
                column: FindSortColumn::Extension,
                descending,
            },
            8,
            Some(locale),
            &Cancellation::default(),
        )
        .unwrap();
        assert_eq!(
            order.ids.iter().map(|id| id.0).collect::<Vec<_>>(),
            panel.iter().map(|entry| entry.id.0).collect::<Vec<_>>(),
            "Find extension order must match the panel, descending={descending}"
        );
        for (row, id) in order.ids.iter().enumerate() {
            assert_eq!(order.positions[id.0 as usize - 1], row);
        }
    }
}

#[test]
fn extension_sort_matches_panel_for_locale_ties_fallbacks_and_missing_extensions() {
    assert_panel_parity(&Weights);
}

#[test]
fn extension_sort_matches_panel_with_captured_native_locale() {
    let locale = midnight_platform::locale_service();
    assert_panel_parity(locale.as_ref());
}

#[test]
fn extension_secondary_name_keys_share_the_aggregate_resource_bound() {
    struct LargeKeys;
    impl LocaleService for LargeKeys {
        fn number_locale(&self) -> Result<NumberLocale, FsError> {
            Ok(NumberLocale::default())
        }
        fn collation_key(&self, value: &OsStr) -> Result<Option<Vec<u8>>, FsError> {
            Ok(Some(vec![
                0;
                if value.as_encoded_bytes().contains(&b'.') {
                    MAX_COLLATION_KEY_BYTES
                } else {
                    1
                }
            ]))
        }
    }
    let error = find_sort::order(
        &fixture(),
        FindSortSpec {
            column: FindSortColumn::Extension,
            descending: false,
        },
        1,
        Some(&LargeKeys),
        &Cancellation::default(),
    )
    .expect_err("both key sets must be bounded");
    assert_eq!(error.kind, std::io::ErrorKind::InvalidData);
}
