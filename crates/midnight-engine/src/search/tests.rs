use super::*;
use midnight_core::{Entry, EntryId, EntryKind, NavigationId, PaneId, SearchMatch, SearchRevision};
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
        filter: Default::default(),
    }
}

fn ready(request: &SearchRequest) -> SearchResult {
    match compute(request, None, &|| false, &|| false).0 {
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
fn duplicate_archive_names_filter_by_provider_identity_without_native_paths() {
    let mut request = request("same", &[]);
    request.source = (2..=3)
        .map(|id| {
            Entry::from_archive(midnight_core::ArchiveNode {
                modified: None,
                id: midnight_core::VirtualEntryId {
                    mount: 9,
                    entry: id,
                },
                parent: 1,
                raw_name: b"same.txt".as_slice().into(),
                display_name: "same.txt".into(),
                kind: EntryKind::File,
                size: Some(id),
                readable: true,
                access: Default::default(),
            })
        })
        .collect::<Vec<_>>()
        .into();
    let result = ready(&request);
    assert_eq!(result.matches.len(), 2);
    assert_eq!(result.index.by_id.len(), 2);
    assert!(result.index.by_path.is_empty());
}

#[test]
fn network_listing_quick_search_and_filter_use_entry_identities_without_native_paths() {
    let network = |id: u64, display: &str| {
        Entry::from_network(
            EntryId(id),
            midnight_core::NetworkNode {
                address: midnight_core::NetworkAddress {
                    provider: "test.network".into(),
                    opaque: Arc::from(id.to_le_bytes()),
                    display: display.into(),
                },
                display: display.into(),
            },
        )
    };
    let mut request = request("s", &[]);
    request.source = [
        network(1, "server-a"),
        network(2, "server-b"),
        network(3, "printer"),
    ]
    .into_iter()
    .collect();
    request.filter = midnight_core::PanelFilter {
        enabled: true,
        expression: "s*".into(),
        ..Default::default()
    };
    // Network rows share the empty native path; it must not make them ambiguous.
    // Masks keep folder-like network rows, so the filter projects all three.
    let result = ready(&request);
    assert_eq!(rows(&result), [(0, 1), (1, 1)]);
    assert_eq!(result.filtered.len(), 3);
    assert_eq!(result.filtered_index.by_id.len(), 3);
    assert!(result.filtered_index.by_path.is_empty());
    assert!(result.index.by_path.is_empty());
    request.filter = Default::default();
    request.query.clear();
    let unfiltered = ready(&request);
    assert_eq!(unfiltered.matches.len(), 3);
    assert_eq!(unfiltered.index.by_id.get(&EntryId(3)), Some(&2));
}

#[test]
fn cached_filter_projection_serves_only_the_same_source_and_filter() {
    let mut request = request("a", &["alpha", "beta", "another"]);
    request.filter = midnight_core::PanelFilter {
        enabled: true,
        expression: "a*".into(),
        ..Default::default()
    };
    let (first, projection) = compute(&request, None, &|| false, &|| false);
    let Computation::Ready(first) = first else {
        panic!("first search failed")
    };
    let projection = projection.expect("validated projection is kept");
    assert!(projection.serves(&request));
    request.query = "an".into();
    let (second, kept) = compute(&request, Some(projection), &|| false, &|| false);
    let Computation::Ready(second) = second else {
        panic!("cached search failed")
    };
    assert!(Arc::ptr_eq(&first.filtered_index, &second.filtered_index));
    assert_eq!(rows(&second), [(2, 2)]);
    let kept = kept.unwrap();
    request.query.clear();
    let (empty, kept) = compute(&request, Some(kept), &|| false, &|| false);
    let Computation::Ready(empty) = empty else {
        panic!("empty query failed")
    };
    assert!(Arc::ptr_eq(&empty.index, &empty.filtered_index));
    assert_eq!(empty.matches, empty.filtered);
    let kept = kept.unwrap();
    let mut other_filter = request.clone();
    other_filter.filter.expression = "b*".into();
    assert!(!kept.serves(&other_filter));
    let mut other_source = request.clone();
    other_source.source = request.source.to_vec().into();
    assert!(!kept.serves(&other_source));
}

fn intent(serial: u64, request: Option<&SearchRequest>) -> Intent {
    Intent {
        serial,
        revision: request.map(|request| request.revision),
        request: request.cloned().map(Arc::new),
    }
}

#[test]
fn a_newer_query_during_the_projection_build_keeps_the_projection_for_reuse() {
    let names: Vec<_> = (0..2_000).map(|row| format!("row-{row}")).collect();
    let names: Vec<_> = names.iter().map(String::as_str).collect();
    let mut first = request("row-1", &names);
    first.filter = midnight_core::PanelFilter {
        enabled: true,
        expression: "row*".into(),
        ..Default::default()
    };
    let mut next = first.clone();
    next.revision = SearchRevision(2);
    next.query = "row-12".into();
    // The search actor's predicates over its intent channel. The next keystroke,
    // for the same listing and filter, arrives at the build's first check.
    let (input, observer) = watch::channel(intent(1, Some(&first)));
    let typed = Cell::new(false);
    let (computation, kept) = compute(
        &first,
        None,
        &|| observer.has_changed().unwrap_or(true),
        &|| {
            if !typed.replace(true) {
                input.send(intent(2, Some(&next))).unwrap();
            }
            projection_obsolete(&observer, &first)
        },
    );
    assert!(
        typed.get(),
        "the keystroke arrived during the projection build"
    );
    assert!(
        matches!(computation, Computation::Cancelled),
        "the superseded query is not answered"
    );
    let kept = kept.expect("a query-only change keeps the finished projection");
    assert!(kept.serves(&next));
    assert_eq!(kept.filtered.len(), 2_000);
    let index = kept.index.clone();
    let rebuilt = Cell::new(false);
    let (computation, _) = compute(&next, Some(kept), &|| false, &|| {
        rebuilt.set(true);
        false
    });
    let Computation::Ready(result) = computation else {
        panic!("search with the kept projection failed")
    };
    assert!(!rebuilt.get(), "the next keystroke rebuilt the projection");
    assert!(Arc::ptr_eq(&result.filtered_index, &index));
    // "row-12", "row-120".."row-129" and "row-1200".."row-1299".
    assert_eq!(result.matches.len(), 111);
}

#[test]
fn only_another_listing_filter_or_cancellation_makes_the_projection_obsolete() {
    let current = request("a", &["alpha", "beta"]);
    let (input, observer) = watch::channel(intent(1, Some(&current)));
    assert!(!projection_obsolete(&observer, &current), "no newer intent");
    let mut typed = current.clone();
    typed.revision = SearchRevision(2);
    typed.query = "al".into();
    input.send(intent(2, Some(&typed))).unwrap();
    assert!(
        !projection_obsolete(&observer, &current),
        "a newer query reuses the projection"
    );
    let mut filtered = typed.clone();
    filtered.filter.enabled = true;
    filtered.filter.expression = "a*".into();
    input.send(intent(3, Some(&filtered))).unwrap();
    assert!(projection_obsolete(&observer, &current), "another filter");
    let mut refreshed = typed.clone();
    refreshed.source = current.source.to_vec().into();
    input.send(intent(4, Some(&refreshed))).unwrap();
    assert!(
        projection_obsolete(&observer, &current),
        "a refreshed listing"
    );
    input.send(intent(5, None)).unwrap();
    assert!(
        projection_obsolete(&observer, &current),
        "cancellation or navigation"
    );
    input.send(intent(6, Some(&typed))).unwrap();
    drop(input);
    assert!(
        projection_obsolete(&observer, &current),
        "a stopped search actor"
    );
}

#[test]
fn archive_prefix_uses_valid_raw_name_instead_of_an_opaque_or_alternate_caption() {
    let mut request = request("raw", &[]);
    request.source = [
        (1, &b"raw-name.txt"[..], "friendly caption"),
        (2, &b"opaque-\xff"[..], "raw-name.txt"),
        (3, &b"other.txt"[..], "raw-name.txt"),
    ]
    .into_iter()
    .map(|(id, raw_name, caption)| {
        Entry::from_archive(midnight_core::ArchiveNode {
            modified: None,
            id: midnight_core::VirtualEntryId {
                mount: 9,
                entry: id,
            },
            parent: 1,
            raw_name: raw_name.into(),
            display_name: caption.into(),
            kind: EntryKind::File,
            size: None,
            readable: true,
            access: Default::default(),
        })
    })
    .collect();
    assert_eq!(rows(&ready(&request)), [(0, 0)]);
}

#[test]
fn persistent_masks_compose_with_prefix_and_keep_directories() {
    let mut request = request("a", &["alpha.txt", "alpha.bin", "folder", "beta.txt"]);
    request.filter = midnight_core::PanelFilter {
        enabled: true,
        expression: "*.txt".into(),
        ..Default::default()
    };
    Arc::make_mut(&mut request.source)[2].kind = EntryKind::DirectoryLink;
    Arc::make_mut(&mut request.source)[0].metadata.size = Some(42);
    let result = ready(&request);
    assert_eq!(rows(&result), [(0, 1)]);
    assert_eq!(
        result
            .filtered
            .iter()
            .map(|row| row.source_row)
            .collect::<Vec<_>>(),
        [0, 2, 3]
    );
    assert_eq!(result.filtered_index.file_sizes, [Some(42), None, None]);
    assert!(result.filtered_index.is_directory(EntryId(3)));
}

#[test]
fn metadata_expression_projection_composes_prefix_and_keeps_only_authoritative_targets() {
    let mut request = request("a", &["alpha.txt", "alpha-small.txt", "folder", "beta.txt"]);
    let source = Arc::make_mut(&mut request.source);
    for entry in source.iter_mut() {
        entry.metadata.size = Some(50);
    }
    source[1].metadata.size = Some(1);
    source[2].kind = EntryKind::Directory;
    request.filter = midnight_core::PanelFilter {
        enabled: true,
        mode: midnight_core::FilterMode::Expression,
        expression: "kind = file and size >= 10".into(),
        case_sensitive: false,
    };
    let result = ready(&request);
    assert!(Arc::ptr_eq(&result.source, &request.source));
    assert_eq!(rows(&result), [(0, 1)]);
    assert_eq!(
        result
            .filtered
            .iter()
            .map(|row| row.source_row)
            .collect::<Vec<_>>(),
        [0, 3]
    );
    assert_eq!(result.filtered_index.file_sizes, [Some(50), Some(50)]);
    let selected = midnight_core::SelectionSnapshot::from_bits(
        Arc::new(DirectoryIndex::from_entries(&request.source)),
        vec![0b1111],
    )
    .unwrap();
    let targets = crate::file_targets::resolve(
        &selected,
        &result.filtered_index,
        None,
        &midnight_core::Cancellation::default(),
    )
    .unwrap();
    assert_eq!(
        targets,
        [
            request.source[0].path.clone(),
            request.source[3].path.clone()
        ]
    );
}

#[test]
fn metadata_expression_refresh_uses_new_snapshot_without_reusing_old_metadata_or_indices() {
    let mut request = request("", &["first", "second"]);
    Arc::make_mut(&mut request.source)[0].metadata.size = Some(100);
    Arc::make_mut(&mut request.source)[1].metadata.size = Some(1);
    request.filter = midnight_core::PanelFilter {
        enabled: true,
        mode: midnight_core::FilterMode::Expression,
        expression: "size >= 50".into(),
        case_sensitive: false,
    };
    let old = ready(&request);
    let source = Arc::make_mut(&mut request.source);
    source.swap(0, 1);
    source[0].metadata.size = Some(70);
    source[1].metadata.size = None;
    request.revision = SearchRevision(2);
    let refreshed = ready(&request);
    assert_eq!(refreshed.filtered_index.by_path.len(), 1);
    assert!(
        refreshed
            .filtered_index
            .by_path
            .contains_key(&request.source[0].path)
    );
    assert!(
        !refreshed
            .filtered_index
            .by_path
            .contains_key(&old.source[0].path)
    );
    assert!(!Arc::ptr_eq(&old.source, &refreshed.source));
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
fn projected_file_sizes_keep_filtered_positions_and_selection_totals_valid() {
    let mut request = request(
        "match",
        &[
            "omitted",
            "match-file",
            "match-unknown",
            "match-link",
            "match-folder",
        ],
    );
    let source = Arc::make_mut(&mut request.source);
    source[0].metadata.size = Some(900);
    source[1].metadata.size = Some(37);
    source[3].kind = EntryKind::Symlink;
    source[3].metadata.size = Some(700);
    source[4].kind = EntryKind::Directory;
    source[4].metadata.size = Some(800);
    let result = ready(&request);
    assert_eq!(result.index.file_sizes, [Some(37), None, None, None]);
    let selected =
        midnight_core::SelectionSnapshot::from_bits(result.index.clone(), vec![1]).unwrap();
    assert_eq!(selected.file_bytes(), Some(37));
    for unknown in [0b10, 0b100] {
        let selected =
            midnight_core::SelectionSnapshot::from_bits(result.index.clone(), vec![unknown])
                .unwrap();
        assert_eq!(selected.file_bytes(), None);
    }
    let folders =
        midnight_core::SelectionSnapshot::from_bits(result.index.clone(), vec![0b1000]).unwrap();
    assert_eq!(folders.file_bytes(), Some(0), "folder totals are separate");
    assert_eq!(
        folders.with_bits(vec![0b1001]).unwrap().file_bytes(),
        Some(37)
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
    let mut request = request("row", &names.iter().map(String::as_str).collect::<Vec<_>>());
    request.filter = midnight_core::PanelFilter {
        enabled: true,
        mode: midnight_core::FilterMode::Expression,
        expression: "kind = file and (size >= 10 or missing(size))".into(),
        case_sensitive: true,
    };
    let polls = Cell::new(0);
    let interrupted = || {
        polls.set(polls.get() + 1);
        polls.get() >= 3
    };
    // A newer intent for another listing or filter abandons the projection build.
    let (computation, kept) = compute(&request, None, &interrupted, &interrupted);
    assert!(matches!(computation, Computation::Cancelled));
    assert!(kept.is_none(), "an abandoned build keeps no projection");
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
            compute(&request, None, &|| false, &|| false).0,
            Computation::Failed(_)
        ));
    }
}

/// `cargo test -p midnight-engine --lib measure_quick_search -- --ignored --nocapture`.
/// Uncached revisions repeat the per-keystroke work of the former single pass.
#[test]
#[ignore = "timing harness"]
fn measure_quick_search_keystrokes_with_and_without_the_cached_projection() {
    let names: Vec<_> = (0..200_000)
        .map(|row| format!("row-{row:06}.txt"))
        .collect();
    let names: Vec<_> = names.iter().map(String::as_str).collect();
    let mut request = request("", &names);
    let queries = ["r", "ro", "row", "row-", "row-1", "row-12", "row-123"];
    let started = std::time::Instant::now();
    for query in queries {
        request.query = query.into();
        assert!(matches!(
            compute(&request, None, &|| false, &|| false).0,
            Computation::Ready(_)
        ));
    }
    let uncached = started.elapsed();
    request.query.clear();
    let mut projection = compute(&request, None, &|| false, &|| false).1;
    let started = std::time::Instant::now();
    for query in queries {
        request.query = query.into();
        let (computation, kept) = compute(&request, projection.take(), &|| false, &|| false);
        assert!(matches!(computation, Computation::Ready(_)));
        projection = kept;
    }
    let cached = started.elapsed();
    eprintln!(
        "200k rows, {} keystrokes: uncached {:?} ({:?}/key), cached {:?} ({:?}/key)",
        queries.len(),
        uncached,
        uncached / queries.len() as u32,
        cached,
        cached / queries.len() as u32
    );
}

#[cfg(unix)]
#[path = "tests/unix.rs"]
mod unix;

#[cfg(windows)]
#[path = "tests/windows.rs"]
mod windows;
