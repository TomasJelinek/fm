use super::*;
use fm_core::{Entry, EntryId, EntryKind, NavigationId, PaneId, SearchMatch, SearchRevision};
use std::{cell::Cell, path::PathBuf};

fn request(query: &str, names: &[&str]) -> SearchRequest {
    SearchRequest {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        revision: SearchRevision(1),
        location: "folder".into(),
        source: names
            .iter()
            .enumerate()
            .map(|(row, name)| {
                Entry::new(
                    EntryId(row as u64 + 1),
                    PathBuf::from("folder").join(name),
                    EntryKind::File,
                )
            })
            .collect::<Vec<_>>()
            .into(),
        query: query.into(),
    }
}

fn ready(request: &SearchRequest) -> SearchResult {
    match compute(request, &|| false) {
        Computation::Ready(result) => result,
        Computation::Cancelled => panic!("unexpected cancellation"),
        Computation::Failed(error) => panic!("unexpected search error: {error}"),
    }
}

fn rows(result: &SearchResult) -> Vec<(usize, usize)> {
    result
        .matches
        .iter()
        .map(|matched| (matched.source_row, matched.display_prefix_end))
        .collect()
}

#[test]
fn full_default_casefold_preserves_expansions_and_original_name_spans() {
    let names = [
        "Straße.txt",
        "STRASSE.data",
        "Σ.txt",
        "ςeta",
        "ﬃle",
        "Other",
    ];
    assert_eq!(rows(&ready(&request("strass", &names))), [(0, 6), (1, 6)]);
    assert_eq!(rows(&ready(&request("σ", &names))), [(2, 2), (3, 2)]);
    assert_eq!(rows(&ready(&request("ffi", &names))), [(4, 3)]);
    assert_eq!(rows(&ready(&request("f", &names))), [(4, 3)]);
    assert_eq!(rows(&ready(&request("s", &["ß"]))), [(0, 2)]);
}

#[test]
fn prefix_highlights_cover_whole_original_combining_and_emoji_graphemes() {
    assert_eq!(rows(&ready(&request("e", &["e\u{301}.txt"]))), [(0, 3)]);
    assert_eq!(
        rows(&ready(&request("👩", &["👩🏽‍💻notes"]))),
        [(0, "👩🏽‍💻".len())]
    );
    assert_eq!(
        rows(&ready(&request("1", &["1\u{fe0f}\u{20e3}name"]))),
        [(0, 7)]
    );
    assert_eq!(rows(&ready(&request("\r", &["\r\nname"]))), [(0, 2)]);
}

#[test]
fn matching_is_plain_prefix_without_locale_accent_or_canonical_normalization() {
    let names = ["I", "i", "İ", "ı"];
    assert_eq!(
        rows(&ready(&request("i", &names))),
        [(0, 1), (1, 1), (2, 2)]
    );
    assert_eq!(rows(&ready(&request("İ", &names))), [(2, 2)]);
    assert_eq!(
        rows(&ready(&request("é", &["éclair", "e\u{301}clair"]))),
        [(0, 2)]
    );
    assert_eq!(
        rows(&ready(&request("*", &["literal.txt", "*literal.txt"]))),
        [(1, 1)]
    );
    assert!(ready(&request("ral", &["literal.txt"])).matches.is_empty());
}

#[test]
fn projection_indexes_are_contiguous_and_keep_distinct_raw_casefold_equal_paths() {
    let request = request("ss", &["other", "ß.txt", "z", "SS.txt"]);
    let result = ready(&request);
    assert!(Arc::ptr_eq(&request.source, &result.source));
    assert_eq!(rows(&result), [(1, 2), (3, 2)]);
    assert_eq!(result.index.by_id.get(&EntryId(2)), Some(&0));
    assert_eq!(result.index.by_id.get(&EntryId(4)), Some(&1));
    assert_eq!(result.index.by_path.len(), 2);
    assert!(
        result
            .index
            .by_path
            .contains_key(&PathBuf::from("folder/ß.txt"))
    );
    assert!(
        result
            .index
            .by_path
            .contains_key(&PathBuf::from("folder/SS.txt"))
    );
}

#[test]
fn alternate_display_labels_cannot_invent_or_misplace_a_raw_name_match() {
    let mut request = request("stras", &["Straße.txt", "other"]);
    Arc::make_mut(&mut request.source)[0].display_name = "friendly label".into();
    Arc::make_mut(&mut request.source)[1].display_name = "Straße".into();
    assert_eq!(rows(&ready(&request)), [(0, 0)]);
}

#[test]
fn large_snapshot_is_cancellable_and_full_results_keep_valid_indices() {
    let names: Vec<_> = (0..50_000).map(|row| format!("row-{row}")).collect();
    let request = request("row", &names.iter().map(String::as_str).collect::<Vec<_>>());
    let polls = Cell::new(0);
    assert!(matches!(
        compute(&request, &|| {
            polls.set(polls.get() + 1);
            polls.get() >= 3
        }),
        Computation::Cancelled
    ));
    let result = ready(&request);
    assert_eq!(result.matches.len(), 50_000);
    assert_eq!(result.index.by_id.get(&EntryId(50_000)), Some(&49_999));
    assert_eq!(
        result.matches.last(),
        Some(&SearchMatch {
            source_row: 49_999,
            display_prefix_end: 3
        })
    );
}

#[test]
fn duplicate_raw_paths_or_ids_cannot_create_an_ambiguous_selection_projection() {
    for duplicate_id in [false, true] {
        let mut request = request("a", &["alpha", "another"]);
        let source = Arc::make_mut(&mut request.source);
        if duplicate_id {
            source[1].id = source[0].id;
        } else {
            source[1].path = source[0].path.clone();
        }
        assert!(matches!(
            compute(&request, &|| false),
            Computation::Failed(_)
        ));
    }
}

#[cfg(unix)]
#[path = "tests/unix.rs"]
mod unix;

#[cfg(windows)]
#[path = "tests/windows.rs"]
mod windows;
