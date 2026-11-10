//! Reprojection failures must not alter the acknowledged view or occurrence state.
use super::{tests::fixture, *};
use midnight_core::{
    FilterMode, PanelFilter, SelectionOperation, SortColumn, SortSpec, TreeProjectionOptions,
    TreeReprojectRequest, TreeSelectionAction, TreeSelectionSnapshot,
};

fn expanded() -> super::tests::Fixture {
    let mut f = fixture();
    let view = f.session.projection().clone();
    f.session.commit(&view);
    f.session
        .branch(
            &view,
            TreeRequestId(1),
            view.rows[1].id,
            BranchAction::Expand,
        )
        .unwrap();
    let work = f.session.take_work().unwrap().unwrap();
    f.session
        .apply(work.run(), &Cancellation::default())
        .unwrap();
    let view = f.session.projection().clone();
    f.session.commit(&view);
    f
}

fn request(session: &mut Session, id: u64, expression: &str) -> TreeReprojectRequest {
    let p = session.publication().unwrap();
    TreeReprojectRequest {
        view: p.view,
        request: TreeRequestId(id),
        selection_revision: p.selection_revision,
        selection: p.selection,
        options: TreeProjectionOptions {
            search: Default::default(),
            sort: SortSpec::default(),
            filter: PanelFilter {
                enabled: !expression.is_empty(),
                mode: if expression.is_empty() {
                    FilterMode::Masks
                } else {
                    FilterMode::Expression
                },
                expression: expression.into(),
                case_sensitive: true,
            },
        },
    }
}

fn select(session: &mut Session, revision: u64, action: TreeSelectionAction) {
    session
        .select(&TreeSelectionRequest {
            view: session.projection().clone(),
            revision: SelectionRevision(revision),
            seed: None,
            seed_revision: SelectionRevision(0),
            action,
        })
        .unwrap();
}

#[test]
fn tree_filter_reprojects_exact_view_and_selection_atomically() {
    let mut f = expanded();
    let view = f.session.projection().clone();
    let child = view.rows.iter().find(|r| r.depth == 1).unwrap().id;
    select(
        &mut f.session,
        1,
        TreeSelectionAction::Bulk(SelectionOperation::All),
    );
    select(&mut f.session, 2, TreeSelectionAction::Toggle(child));
    let req = request(&mut f.session, 2, "name = \"child\"");
    let changed = f
        .session
        .reproject_request(&req, &Cancellation::default())
        .unwrap();
    assert_eq!(
        changed
            .view
            .rows
            .iter()
            .map(|r| r.entry.display_name.as_str())
            .collect::<Vec<_>>(),
        ["z-parent", "child"]
    );
    assert!(changed.view.root.same_snapshot(&view.root));
    assert!(changed.view.rows[0].context_only && !changed.view.rows[0].selectable);
    assert!(changed.view.rows[0].expanded);
    assert_eq!(changed.selection.len(), 1);
    assert!(changed.selection.contains_row(child));
    assert!(Arc::ptr_eq(&changed.view, changed.selection.basis()));
    assert!(
        f.session.check_view(&changed.view).is_err(),
        "new view needs its own Commit"
    );
    assert!(f.session.commit(&changed.view));
    assert!(
        f.session
            .reproject_request(&req, &Cancellation::default())
            .is_err()
    );
}

#[test]
fn tree_filter_rejects_invalid_stale_and_cancelled_requests_without_changing_state() {
    let mut f = expanded();
    let req = request(&mut f.session, 2, "name = \"child\"");
    for case in 0..8 {
        let mut bad = req.clone();
        let cancel = Cancellation::default();
        match case {
            0 => bad.options.filter.expression = "name =".into(),
            1 => bad.options.filter.expression = "x".repeat(4097),
            2 => bad.options.filter.expression = "\0".into(),
            3 => bad.view = Arc::new((*req.view).clone()),
            4 => bad.selection = Arc::new((*req.selection).clone()),
            5 => bad.selection_revision = SelectionRevision(99),
            6 => bad.request = TreeRequestId(1),
            7 => cancel.cancel(),
            _ => unreachable!(),
        }
        assert!(
            f.session.reproject_request(&bad, &cancel).is_err(),
            "case {case}"
        );
        let after = f.session.publication().unwrap();
        assert!(Arc::ptr_eq(&after.view, &req.view));
        assert!(Arc::ptr_eq(&after.selection, &req.selection));
        assert!(f.session.check_view(&req.view).is_ok());
        assert!(!f.session.options.filter.enabled);
    }
    let good = f
        .session
        .reproject_request(&req, &Cancellation::default())
        .unwrap();
    assert_eq!(
        good.view.rows[1].id,
        req.view.rows.iter().find(|r| r.depth == 1).unwrap().id
    );
}

#[test]
fn tree_filter_clear_and_sort_keep_expansion_and_top_level_select_all() {
    let mut f = expanded();
    let initial = f.session.projection().clone();
    let child = initial.rows.iter().find(|r| r.depth == 1).unwrap().id;
    select(&mut f.session, 1, TreeSelectionAction::Toggle(child));
    let req = request(&mut f.session, 2, "name = \"file.txt\"");
    let filtered = f
        .session
        .reproject_request(&req, &Cancellation::default())
        .unwrap();
    assert!(filtered.selection.is_empty());
    f.session.commit(&filtered.view);
    let mut clear = request(&mut f.session, 3, "name = \"file.txt\"");
    clear.options.filter.enabled = false;
    clear.options.sort = SortSpec {
        column: SortColumn::Name,
        descending: true,
    };
    let restored = f
        .session
        .reproject_request(&clear, &Cancellation::default())
        .unwrap();
    assert_eq!(
        restored
            .view
            .rows
            .iter()
            .map(|r| r.entry.display_name.as_str())
            .collect::<Vec<_>>(),
        ["z-parent", "child", "a-parent", "file.txt"]
    );
    assert!(restored.view.rows[0].expanded);
    assert!(restored.selection.is_empty());
    assert_eq!(f.session.options.filter.expression, "name = \"file.txt\"");
    f.session.commit(&restored.view);
    select(&mut f.session, 2, TreeSelectionAction::Toggle(child));
    select(
        &mut f.session,
        3,
        TreeSelectionAction::Bulk(SelectionOperation::All),
    );
    let selected = f.session.publication().unwrap().selection;
    assert_eq!(selected.len(), 3);
    assert!(!selected.contains_row(child));
    let foreign = Arc::new(TreeSelectionSnapshot::from_bits(initial, vec![0]).unwrap());
    let mut req = request(&mut f.session, 4, "");
    req.selection = foreign;
    assert!(
        f.session
            .reproject_request(&req, &Cancellation::default())
            .is_err()
    );
}

#[test]
fn tree_filter_cancellation_during_projection_preserves_records_for_retry() {
    struct CancelLocale(Cancellation);
    impl midnight_core::LocaleService for CancelLocale {
        fn number_locale(&self) -> Result<midnight_core::NumberLocale, midnight_core::FsError> {
            Ok(Default::default())
        }
        fn collation_key(
            &self,
            _: &std::ffi::OsStr,
        ) -> Result<Option<Vec<u8>>, midnight_core::FsError> {
            self.0.cancel();
            Ok(Some(vec![99]))
        }
    }
    let mut f = expanded();
    let cancel = Cancellation::default();
    f.session.options.locale = Some(Arc::new(CancelLocale(cancel.clone())));
    let req = request(&mut f.session, 2, "name = \"child\"");
    assert!(f.session.reproject_request(&req, &cancel).is_err());
    let after = f.session.publication().unwrap();
    assert!(Arc::ptr_eq(&after.view, &req.view));
    assert!(Arc::ptr_eq(&after.selection, &req.selection));
    assert_eq!(f.session.last_request, TreeRequestId(1));
    f.session.options.locale = None;
    let changed = f
        .session
        .reproject_request(&req, &Cancellation::default())
        .unwrap();
    assert_eq!(
        changed.view.rows[1].id,
        req.view.rows.iter().find(|r| r.depth == 1).unwrap().id
    );
    assert_eq!(changed.view.rows[0].entry.display_name, "z-parent");
}

#[test]
fn tree_search_jump_preserves_marks_and_filter_keeps_only_match_and_context() {
    let mut f = expanded();
    let initial = f.session.projection().clone();
    let child = initial.rows.iter().find(|r| r.depth == 1).unwrap().id;
    select(
        &mut f.session,
        1,
        TreeSelectionAction::Bulk(SelectionOperation::All),
    );
    select(&mut f.session, 2, TreeSelectionAction::Toggle(child));
    let mut req = request(&mut f.session, 2, "");
    req.options.filter.enabled = false;
    req.options.search.query = "CH".into();
    let jumped = f
        .session
        .reproject_request(&req, &Cancellation::default())
        .unwrap();
    assert_eq!(jumped.view.rows.len(), 4);
    assert_eq!(jumped.selection.len(), 4);
    assert_eq!(&*jumped.view.index.search_rows, &[2]);
    assert_eq!(jumped.view.rows[2].prefix_end, 2);
    assert_eq!(jumped.view.rows[2].id, child);
    f.session.commit(&jumped.view);
    let mut req = request(&mut f.session, 3, "");
    req.options.search.query = "ch".into();
    req.options.search.filter_matches = true;
    let filtered = f
        .session
        .reproject_request(&req, &Cancellation::default())
        .unwrap();
    assert_eq!(filtered.view.rows.len(), 2);
    assert_eq!(&*filtered.view.index.search_rows, &[1]);
    assert!(filtered.view.rows[0].context_only);
    assert!(!filtered.view.rows[0].search_match);
    assert!(!filtered.view.rows[0].selectable);
    assert_eq!(filtered.selection.len(), 1);
    assert!(filtered.selection.contains_row(child));
    assert!(Arc::ptr_eq(&filtered.view, filtered.selection.basis()));
    f.session.commit(&filtered.view);
    let mut req = request(&mut f.session, 4, "");
    req.options.search.query = "missing".into();
    req.options.search.filter_matches = true;
    let empty = f
        .session
        .reproject_request(&req, &Cancellation::default())
        .unwrap();
    assert!(empty.view.rows.is_empty());
    assert!(empty.view.index.search_rows.is_empty());
    assert!(empty.selection.is_empty());
    f.session.commit(&empty.view);
    let req = request(&mut f.session, 5, "");
    let restored = f
        .session
        .reproject_request(&req, &Cancellation::default())
        .unwrap();
    assert_eq!(restored.view.rows.len(), 4);
    assert!(restored.view.index.search_rows.is_empty());
    assert!(restored.selection.is_empty());
}

#[test]
fn tree_search_invalid_query_preserves_exact_published_pair() {
    let mut f = expanded();
    for query in ["x".repeat(4097), "bad\0query".into()] {
        let mut req = request(&mut f.session, 2, "");
        req.options.search.query = query;
        assert!(
            f.session
                .reproject_request(&req, &Cancellation::default())
                .is_err()
        );
        let after = f.session.publication().unwrap();
        assert!(Arc::ptr_eq(&after.view, &req.view));
        assert!(Arc::ptr_eq(&after.selection, &req.selection));
    }
}

#[test]
fn tree_search_persistent_filter_context_never_becomes_a_prefix_match() {
    let mut f = expanded();
    let mut req = request(&mut f.session, 2, "name = \"child\"");
    req.options.search.query = "Z".into();
    let jumped = f
        .session
        .reproject_request(&req, &Cancellation::default())
        .unwrap();
    assert_eq!(jumped.view.rows.len(), 2);
    assert_eq!(jumped.view.rows[0].entry.display_name, "z-parent");
    assert!(jumped.view.rows[0].context_only);
    assert!(jumped.view.index.search_rows.is_empty());
    assert_eq!(jumped.view.rows[0].prefix_end, 0);
    f.session.commit(&jumped.view);
    let mut req = request(&mut f.session, 3, "name = \"child\"");
    req.options.search.query = "Z".into();
    req.options.search.filter_matches = true;
    let filtered = f
        .session
        .reproject_request(&req, &Cancellation::default())
        .unwrap();
    assert!(filtered.view.rows.is_empty());
}

#[test]
fn tree_search_branch_completion_recomputes_the_retained_query() {
    let mut f = fixture();
    let initial = f.session.projection().clone();
    f.session.commit(&initial);
    let mut req = request(&mut f.session, 1, "");
    req.options.search.query = "CH".into();
    let searched = f
        .session
        .reproject_request(&req, &Cancellation::default())
        .unwrap();
    assert!(searched.view.index.search_rows.is_empty());
    f.session.commit(&searched.view);
    let parent = searched
        .view
        .rows
        .iter()
        .find(|r| r.entry.display_name == "z-parent")
        .unwrap()
        .id;
    f.session
        .branch(
            &searched.view,
            TreeRequestId(2),
            parent,
            BranchAction::Expand,
        )
        .unwrap();
    let work = f.session.take_work().unwrap().unwrap();
    f.session
        .apply(work.run(), &Cancellation::default())
        .unwrap();
    let publication = f.session.publication().unwrap();
    assert_eq!(&*publication.view.index.search_rows, &[2]);
    assert_eq!(publication.view.rows[2].entry.display_name, "child");
    assert_eq!(publication.view.rows[2].prefix_end, 2);
    assert!(Arc::ptr_eq(
        &publication.view,
        publication.selection.basis()
    ));
}

#[test]
fn tree_search_query_changes_reuse_prepared_locale_keys() {
    #[derive(Default)]
    struct CountLocale(std::sync::atomic::AtomicUsize);
    impl midnight_core::LocaleService for CountLocale {
        fn number_locale(&self) -> Result<midnight_core::NumberLocale, midnight_core::FsError> {
            Ok(Default::default())
        }
        fn collation_key(
            &self,
            value: &std::ffi::OsStr,
        ) -> Result<Option<Vec<u8>>, midnight_core::FsError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(value.to_str().map(|name| name.as_bytes().to_vec()))
        }
    }
    let mut f = expanded();
    let locale = Arc::new(CountLocale::default());
    f.session.options.locale = Some(locale.clone());
    let mut req = request(&mut f.session, 2, "");
    req.options.search.query = "CH".into();
    let searched = f
        .session
        .reproject_request(&req, &Cancellation::default())
        .unwrap();
    let calls = locale.0.load(Ordering::Relaxed);
    assert!(calls > 0);
    f.session.commit(&searched.view);
    let mut req = request(&mut f.session, 3, "");
    req.options.search.query = "CHILD".into();
    let searched = f
        .session
        .reproject_request(&req, &Cancellation::default())
        .unwrap();
    assert_eq!(&*searched.view.index.search_rows, &[2]);
    assert_eq!(locale.0.load(Ordering::Relaxed), calls);
}
