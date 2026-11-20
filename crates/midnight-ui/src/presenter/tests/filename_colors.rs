use super::*;

#[test]
fn color_matching_waits_for_loaded_preferences_and_nonempty_entries() {
    let mut p = Presenter::new("left".into(), "right".into());
    let generation = begin(&mut p, PaneId::Left, "left");
    finish(&mut p, PaneId::Left, generation, vec![entry(1, "a.rs")]);
    p.flush_filename_colors(&|_| panic!("saved preferences have not arrived"));
    p.preferences_loaded(Ok(UserPreferences::default()));
    let matched = RefCell::new(Vec::new());
    p.flush_filename_colors(&|command| {
        let Command::MatchFilenameColors(request) = command else {
            panic!("expected matching for populated panel")
        };
        matched.borrow_mut().push(request.pane);
        true
    });
    assert_eq!(*matched.borrow(), [PaneId::Left]);
}

fn colored_preferences() -> UserPreferences {
    UserPreferences {
        filename_colors: vec![FilenameColorRule {
            style: FilenameStyle {
                foreground: Some(RgbColor(220, 30, 40)),
                background: None,
            },
            ..Default::default()
        }]
        .into(),
        ..Default::default()
    }
}

fn request(p: &mut Presenter) -> FilenameColorsRequest {
    let sent = RefCell::new(None);
    p.flush_filename_colors(&|command| {
        if let Command::MatchFilenameColors(request) = command
            && request.pane == PaneId::Left
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    sent.into_inner().unwrap()
}

fn deliver(p: &mut Presenter, request: FilenameColorsRequest) {
    let styles = if let Some(tree) = &request.tree {
        tree.rows
            .iter()
            .map(|row| (row.entry.id, request.rules[0].style))
            .collect()
    } else {
        request
            .source
            .iter()
            .map(|entry| (entry.id, request.rules[0].style))
            .collect()
    };
    let result = FilenameColorsResult {
        styles,
        request: request.clone(),
    };
    p.apply(Event::FilenameColorsReady {
        request,
        result: Ok(Arc::new(result)),
    });
}

fn tree_colors(root_id: u64) -> (Presenter, Arc<TreeProjection>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("_fixtures/tree-filename-colors");
    let mut p = Presenter::new(path.clone(), path.join("other"));
    p.preferences_loaded(Ok(colored_preferences()));
    let generation = begin(&mut p, PaneId::Left, path.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![
            Entry::new(EntryId(root_id), path.join("root.pdf"), EntryKind::File),
            Entry::new(
                EntryId(root_id + 1),
                path.join("nested"),
                EntryKind::Directory,
            ),
        ],
    );
    let pane = &p.panes[0];
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: generation,
        entries: pane.entries.clone(),
        index: pane.index.clone(),
    };
    let mut rows = Vec::new();
    for (position, (name, depth, parent, kind)) in [
        ("root.pdf", 0, None, EntryKind::File),
        ("nested", 0, None, EntryKind::Directory),
        ("nested/child.pdf", 1, Some(TreeRowId(2)), EntryKind::File),
        ("nested/deeper", 1, Some(TreeRowId(2)), EntryKind::Directory),
        (
            "nested/deeper/grandchild.pdf",
            2,
            Some(TreeRowId(4)),
            EntryKind::File,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut entry = Entry::new(EntryId(1001 + position as u64), path.join(name), kind);
        entry.display_name = format!("{}Visible label {position}", "  ".repeat(depth));
        rows.push(TreeDisplayRow {
            id: TreeRowId(position as u64 + 1),
            entry,
            parent,
            depth,
            disclosure: if kind == EntryKind::Directory {
                TreeDisclosure::Expanded
            } else {
                TreeDisclosure::None
            },
            expanded: kind == EntryKind::Directory,
            selectable: true,
            navigable: kind == EntryKind::Directory,
            context_only: false,
            error: None,
            error_details: None,
            original_root_entry: (depth == 0).then_some(EntryId(root_id + position as u64)),
            prefix_end: depth * 2,
            search_match: false,
        });
    }
    let view = Arc::new(
        TreeProjection::new(
            root,
            TreeViewStamp {
                pane: PaneId::Left,
                navigation: generation,
                session: TreeSessionId(1),
                revision: TreeRevision(1),
            },
            TreeLayoutRevision(1),
            rows,
        )
        .unwrap(),
    );
    install_tree_colors(&mut p, view.clone());
    (p, view)
}

fn install_tree_colors(p: &mut Presenter, view: Arc<TreeProjection>) {
    p.panes[0].tree.publication = Some(
        TreePublication::new(
            view.clone(),
            SelectionRevision(0),
            Arc::new(TreeSelectionSnapshot::from_bits(view, vec![0]).unwrap()),
        )
        .unwrap(),
    );
}

#[test]
fn tree_color_results_use_synthetic_entry_ids_for_root_child_and_grandchild() {
    let (mut p, view) = tree_colors(1);
    let matched = request(&mut p);
    assert!(Arc::ptr_eq(&matched.source, &view.root.entries));
    assert!(Arc::ptr_eq(matched.tree.as_ref().unwrap(), &view));
    deliver(&mut p, matched);
    for index in [0, 2, 4] {
        assert_eq!(
            p.filename_style(PaneId::Left, &view.rows[index].entry),
            Some(colored_preferences().filename_colors[0].style)
        );
    }
    p.flush_filename_colors(&|_| panic!("same source/rules/tree must not submit another job"));
    p.panes[0].cursor = Some(view.rows[4].entry.id);
    p.flush_filename_colors(&|_| panic!("cursor-only change must reuse the tree colors"));
}

#[test]
fn same_layout_metadata_tree_replacement_requires_matching_and_rejects_old_results() {
    let (mut p, view) = tree_colors(1);
    let old = request(&mut p);
    deliver(&mut p, old.clone());
    let mut rows = view.rows.to_vec();
    rows[4].entry.metadata.size = Some(123);
    let mut stamp = view.stamp;
    stamp.revision = TreeRevision(2);
    let next = Arc::new(
        TreeProjection::new(view.root.clone(), stamp, view.layout_revision, rows).unwrap(),
    );
    install_tree_colors(&mut p, next.clone());
    assert!(
        p.filename_style(PaneId::Left, &next.rows[4].entry)
            .is_none()
    );
    let current = request(&mut p);
    assert_eq!(next.layout_revision, view.layout_revision);
    assert!(Arc::ptr_eq(current.tree.as_ref().unwrap(), &next));
    deliver(&mut p, old);
    assert!(
        p.filename_style(PaneId::Left, &next.rows[4].entry)
            .is_none()
    );
    deliver(&mut p, current);
    assert!(
        p.filename_style(PaneId::Left, &next.rows[4].entry)
            .is_some()
    );
}

#[test]
fn identical_tree_stamp_on_a_new_allocation_does_not_reuse_color_work() {
    let (mut p, view) = tree_colors(1);
    let old = request(&mut p);
    deliver(&mut p, old.clone());
    let replacement = Arc::new((*view).clone());
    install_tree_colors(&mut p, replacement.clone());
    assert!(
        p.filename_style(PaneId::Left, &replacement.rows[4].entry)
            .is_none()
    );
    let current = request(&mut p);
    assert_eq!(current.tree.as_ref().unwrap().stamp, view.stamp);
    assert!(Arc::ptr_eq(current.tree.as_ref().unwrap(), &replacement));
    deliver(&mut p, old);
    assert!(
        p.filename_style(PaneId::Left, &replacement.rows[4].entry)
            .is_none()
    );
    deliver(&mut p, current);
    assert!(
        p.filename_style(PaneId::Left, &replacement.rows[4].entry)
            .is_some()
    );
}

#[test]
fn retained_color_result_must_describe_the_exact_admitted_request() {
    let (mut p, view) = tree_colors(1);
    let current = request(&mut p);
    for variant in 0..4 {
        let mut retained = current.clone();
        match variant {
            0 => retained.tree = Some(Arc::new((*view).clone())), // Same stamp/layout/rows; different projection allocation.
            1 => retained.rules = retained.rules.to_vec().into(),
            2 => retained.source = retained.source.to_vec().into(),
            3 => retained.id += 1,
            _ => unreachable!(),
        }
        let styles = [(view.rows[4].entry.id, current.rules[0].style)].into();
        p.apply(Event::FilenameColorsReady {
            request: current.clone(),
            result: Ok(Arc::new(FilenameColorsResult {
                request: retained,
                styles,
            })),
        });
        assert!(
            p.filename_style(PaneId::Left, &view.rows[4].entry)
                .is_none(),
            "retained snapshot variant {variant}"
        );
    }
    deliver(&mut p, current);
    assert!(
        p.filename_style(PaneId::Left, &view.rows[4].entry)
            .is_some()
    );
}

#[test]
fn tree_and_flat_color_results_do_not_cross_modes_when_ids_and_root_arc_are_reused() {
    let (mut p, view) = tree_colors(1001);
    p.panes[0].tree.publication = None;
    let flat = request(&mut p);
    assert!(flat.tree.is_none());
    deliver(&mut p, flat.clone());
    assert!(
        p.filename_style(PaneId::Left, &p.panes[0].entries[0])
            .is_some()
    );
    install_tree_colors(&mut p, view.clone());
    assert!(
        p.filename_style(PaneId::Left, &view.rows[0].entry)
            .is_none()
    );
    let tree = request(&mut p);
    deliver(&mut p, flat);
    assert!(
        p.filename_style(PaneId::Left, &view.rows[0].entry)
            .is_none()
    );
    deliver(&mut p, tree.clone());
    assert!(
        p.filename_style(PaneId::Left, &view.rows[0].entry)
            .is_some()
    );
    p.panes[0].tree.publication = None;
    assert!(
        p.filename_style(PaneId::Left, &p.panes[0].entries[0])
            .is_none()
    );
    let flat = request(&mut p);
    deliver(&mut p, tree);
    assert!(
        p.filename_style(PaneId::Left, &p.panes[0].entries[0])
            .is_none()
    );
    deliver(&mut p, flat);
    assert!(
        p.filename_style(PaneId::Left, &p.panes[0].entries[0])
            .is_some()
    );
}

#[test]
fn tree_color_rules_replacement_rejects_the_previous_rules_arc() {
    let (mut p, view) = tree_colors(1);
    let old = request(&mut p);
    deliver(&mut p, old.clone());
    let mut rules = p.preferences.values.filename_colors.to_vec();
    rules[0].style.foreground = Some(RgbColor(40, 90, 230));
    p.preferences.values.filename_colors = rules.into();
    let current = request(&mut p);
    deliver(&mut p, old);
    assert!(
        p.filename_style(PaneId::Left, &view.rows[4].entry)
            .is_none()
    );
    deliver(&mut p, current);
    assert_eq!(
        p.filename_style(PaneId::Left, &view.rows[4].entry)
            .unwrap()
            .foreground,
        Some(RgbColor(40, 90, 230))
    );
}

#[test]
fn tree_navigation_mismatch_clears_admitted_color_work() {
    let (mut p, view) = tree_colors(1);
    let current = request(&mut p);
    deliver(&mut p, current);
    p.panes[0].generation += 1;
    assert!(
        p.filename_style(PaneId::Left, &view.rows[4].entry)
            .is_none()
    );
    p.flush_filename_colors(&|command| {
        assert!(matches!(
            command,
            Command::ClearFilenameColors {
                pane: PaneId::Left,
                ..
            }
        ));
        true
    });
    p.flush_filename_colors(&|_| panic!("invalid navigation must not start another matching job"));
}

#[test]
fn stale_color_work_cannot_color_recycled_entry_ids_after_refresh() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.preferences_loaded(Ok(colored_preferences()));
    let generation = begin(&mut p, PaneId::Left, "left");
    finish(&mut p, PaneId::Left, generation, vec![entry(1, "old.rs")]);
    let old = request(&mut p);
    let generation = begin(&mut p, PaneId::Left, "left");
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![entry(1, "replacement.txt")],
    );
    let new = request(&mut p);
    deliver(&mut p, old);
    assert!(
        p.filename_style(PaneId::Left, &p.panes[0].entries[0])
            .is_none()
    );
    deliver(&mut p, new);
    assert_eq!(
        p.filename_style(PaneId::Left, &p.panes[0].entries[0])
            .unwrap()
            .foreground,
        Some(RgbColor(220, 30, 40))
    );
}

#[test]
fn selection_changes_reuse_styles_but_reordered_source_gets_fresh_matching() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.preferences_loaded(Ok(colored_preferences()));
    let generation = begin(&mut p, PaneId::Left, "left");
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![entry(1, "a.rs"), entry(2, "b.rs")],
    );
    let initial = request(&mut p);
    deliver(&mut p, initial);
    p.panes[0].cursor = Some(EntryId(2));
    p.panes[0].selected =
        Arc::new(SelectionSnapshot::from_bits(p.panes[0].index.clone(), vec![2]).unwrap());
    p.flush_filename_colors(&|_| panic!("caret-only change must reuse styles"));
    assert!(
        p.filename_style(PaneId::Left, &p.panes[0].entries[1])
            .is_some()
    );
    let reversed: Vec<_> = p.panes[0].entries.iter().rev().cloned().collect();
    p.panes[0].entries = reversed.into();
    assert!(
        p.filename_style(PaneId::Left, &p.panes[0].entries[0])
            .is_none()
    );
    let reordered = request(&mut p);
    assert_eq!(reordered.source[0].id, EntryId(2));
    deliver(&mut p, reordered);
    assert!(
        p.filename_style(PaneId::Left, &p.panes[0].entries[0])
            .is_some()
    );
}

#[test]
fn local_color_edits_survive_late_preferences_without_overwriting_other_saved_options() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.update_preferences(colored_preferences(), &|_| {
        panic!("save must await disk preferences")
    });
    p.preferences_loaded(Ok(UserPreferences {
        theme: PreferenceTheme::Nord,
        font_family: "Saved 文档".into(),
        ..Default::default()
    }));
    assert_eq!(
        p.preferences.values.filename_colors,
        colored_preferences().filename_colors
    );
    assert_eq!(p.preferences.values.theme, PreferenceTheme::Nord);
    assert_eq!(p.preferences.values.font_family, "Saved 文档");
}

#[test]
fn removing_last_rule_cancels_backend_work_and_releases_the_previous_snapshot() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.preferences_loaded(Ok(colored_preferences()));
    let generation = begin(&mut p, PaneId::Left, "left");
    finish(&mut p, PaneId::Left, generation, vec![entry(1, "a.rs")]);
    let current = request(&mut p);
    deliver(&mut p, current);
    Arc::make_mut(&mut p.preferences.values.filename_colors).clear();
    let cleared = RefCell::new(Vec::new());
    p.flush_filename_colors(&|command| {
        let Command::ClearFilenameColors { pane, .. } = command else {
            panic!("expected color cancellation")
        };
        cleared.borrow_mut().push(pane);
        true
    });
    assert_eq!(*cleared.borrow(), [PaneId::Left]);
    assert!(
        p.filename_style(PaneId::Left, &p.panes[0].entries[0])
            .is_none()
    );
    p.flush_filename_colors(&|_| panic!("cancel must not repeat after it is accepted"));
}

#[test]
fn colors_of_rows_a_background_reread_edits_in_place_stay_until_matched_again() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.preferences_loaded(Ok(colored_preferences()));
    let generation = begin(&mut p, PaneId::Left, "left");
    let mut grown = Entry::new(EntryId(2), "left/b.rs".into(), EntryKind::File);
    grown.metadata.size = Some(1);
    let shown = vec![
        Entry::new(EntryId(1), "left/a.rs".into(), EntryKind::File),
        grown.clone(),
    ];
    finish(&mut p, PaneId::Left, generation, shown.clone());
    let first = request(&mut p);
    deliver(&mut p, first);
    let style = |p: &Presenter, entry: &Entry| p.filename_style(PaneId::Left, entry);
    assert!(style(&p, &shown[0]).is_some());

    // A watch hint re-reads the folder in the background; one file grew.
    let sent = RefCell::new(None);
    assert!(p.refresh_if_changed(PaneId::Left, generation, &|command| {
        if let Command::Refresh { navigation, .. } = command {
            *sent.borrow_mut() = Some(navigation);
        }
        true
    }));
    let refresh = sent.into_inner().expect("a background Refresh");
    assert!(
        style(&p, &shown[0]).is_some(),
        "pending rows keep their colors"
    );
    grown.metadata.size = Some(2);
    let listed = vec![shown[0].clone(), grown];
    p.apply(Event::ListingChanges {
        pane: PaneId::Left,
        navigation: refresh,
        base: p.panes[0].entries.clone(),
        edits: Arc::from([RowEdit::Changed { row: 1 }]),
    });
    finish(&mut p, PaneId::Left, refresh, listed.clone());
    assert!(
        p.panes[0].row_edits().is_some(),
        "the rows were edited in place"
    );
    assert!(
        style(&p, &listed[0]).is_some(),
        "an in-place commit keeps the colors until the new rows are matched"
    );
    let second = request(&mut p);
    assert!(
        style(&p, &listed[0]).is_some(),
        "requesting the new match keeps the colors meanwhile"
    );
    deliver(&mut p, second);
    assert!(style(&p, &listed[1]).is_some());
}
