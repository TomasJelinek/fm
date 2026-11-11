//! Directory size admission keeps exact occurrence and selection authority.
use super::*;
use midnight_core::{TreeSizeRequest, TreeSizeScope};

fn request(session: &mut Session, scope: TreeSizeScope) -> TreeSizeRequest {
    let publication = session.publication().unwrap();
    assert!(session.commit(&publication.view));
    TreeSizeRequest {
        view: publication.view,
        request: TreeRequestId(session.last_request.0 + 1),
        selection_revision: publication.selection_revision,
        selection: publication.selection,
        scope,
    }
}

#[test]
fn collapsed_tree_measures_ordinary_directory_rows() {
    let mut fixture = tests::fixture();
    let request = request(&mut fixture.session, TreeSizeScope::SelectionOrVisible);
    let captured = fixture
        .session
        .capture_sizes(&request, &Cancellation::default())
        .unwrap();
    assert_eq!(captured.targets.len(), 2);
    for target in captured.targets {
        let position = request.view.index.row_positions[&target.row];
        assert_eq!(request.view.rows[position].depth, 0);
        assert_eq!(
            request.view.rows[position].entry.kind,
            midnight_core::EntryKind::Directory
        );
    }
}

#[test]
fn native_directory_link_is_captured_for_bulk_and_exact_single_size_scope() {
    let mut fixture =
        tests::fixture_with_directory_kind(0, midnight_core::EntryKind::DirectoryLink);
    let bulk = request(&mut fixture.session, TreeSizeScope::SelectionOrVisible);
    let link = bulk
        .view
        .rows
        .iter()
        .find(|row| row.entry.kind == midnight_core::EntryKind::DirectoryLink)
        .unwrap()
        .id;
    let captured = fixture
        .session
        .capture_sizes(&bulk, &Cancellation::default())
        .unwrap();
    assert_eq!(captured.targets.len(), 2);
    assert!(captured.targets.iter().any(|target| target.row == link));
    let single = request(&mut fixture.session, TreeSizeScope::Single(link));
    let captured = fixture
        .session
        .capture_sizes(&single, &Cancellation::default())
        .unwrap();
    assert_eq!(captured.targets.len(), 1);
    assert_eq!(captured.targets[0].row, link);
    assert!(matches!(
        captured.targets[0].captured.target.value().unwrap(),
        crate::providers::ProviderTarget::Native(_)
    ));
}

#[test]
fn empty_selection_includes_visible_expanded_descendants() {
    let mut fixture = tests::fixture();
    let parent = fixture.session.projection().rows[1].id;
    let expanded = action_capture_tests::expand(&mut fixture.session, parent);
    let child = expanded
        .view
        .rows
        .iter()
        .find(|row| row.depth == 1)
        .unwrap()
        .id;
    let request = request(&mut fixture.session, TreeSizeScope::SelectionOrVisible);
    let captured = fixture
        .session
        .capture_sizes(&request, &Cancellation::default())
        .unwrap();
    assert_eq!(captured.targets.len(), 3);
    assert!(captured.targets.iter().any(|target| target.row == child));
    assert!(captured.targets.iter().any(|target| target.row == parent));
}

#[test]
fn empty_selection_excludes_filtered_context_ancestors() {
    let mut fixture = tests::fixture();
    let parent = fixture.session.projection().rows[1].id;
    action_capture_tests::expand(&mut fixture.session, parent);
    fixture
        .session
        .reproject(
            ProjectionOptions {
                filter: midnight_core::PanelFilter {
                    enabled: true,
                    mode: midnight_core::FilterMode::Expression,
                    expression: "name = \"child\"".into(),
                    case_sensitive: true,
                },
                ..Default::default()
            },
            &Cancellation::default(),
        )
        .unwrap();
    let request = request(&mut fixture.session, TreeSizeScope::SelectionOrVisible);
    assert_eq!(request.view.rows.len(), 2);
    assert!(request.view.rows[0].context_only);
    let captured = fixture
        .session
        .capture_sizes(&request, &Cancellation::default())
        .unwrap();
    assert_eq!(captured.targets.len(), 1);
    assert_eq!(captured.targets[0].row, request.view.rows[1].id);
}

#[test]
fn size_capture_rejects_equal_but_foreign_selection_allocation() {
    let mut fixture = tests::fixture();
    let mut request = request(&mut fixture.session, TreeSizeScope::SelectionOrVisible);
    request.selection = Arc::new((*request.selection).clone());
    assert!(matches!(
        fixture
            .session
            .capture_sizes(&request, &Cancellation::default()),
        Err(ProviderError::StaleReference)
    ));
}

#[test]
fn single_size_scope_rejects_a_file_without_scanning_fallback_directories() {
    let mut fixture = tests::fixture();
    let file = fixture
        .session
        .projection()
        .rows
        .iter()
        .find(|row| row.entry.kind == midnight_core::EntryKind::File)
        .unwrap()
        .id;
    let request = request(&mut fixture.session, TreeSizeScope::Single(file));
    assert!(matches!(
        fixture
            .session
            .capture_sizes(&request, &Cancellation::default()),
        Err(ProviderError::Unsupported)
    ));
}

#[test]
fn files_only_selection_never_falls_back_to_all_directories() {
    let mut fixture = tests::fixture();
    let publication = fixture.session.publication().unwrap();
    fixture.session.commit(&publication.view);
    let file = publication
        .view
        .rows
        .iter()
        .find(|row| row.entry.kind == midnight_core::EntryKind::File)
        .unwrap()
        .id;
    fixture
        .session
        .select(&TreeSelectionRequest {
            view: publication.view,
            revision: SelectionRevision(1),
            seed: None,
            seed_revision: SelectionRevision(0),
            action: midnight_core::TreeSelectionAction::Toggle(file),
        })
        .unwrap();
    let request = request(&mut fixture.session, TreeSizeScope::SelectionOrVisible);
    assert!(
        fixture
            .session
            .capture_sizes(&request, &Cancellation::default())
            .unwrap()
            .targets
            .is_empty()
    );
}

#[test]
fn single_occurrence_ignores_other_marked_directories() {
    let mut fixture = tests::fixture();
    let publication = fixture.session.publication().unwrap();
    fixture.session.commit(&publication.view);
    let first = publication.view.rows[0].id;
    let second = publication.view.rows[1].id;
    fixture
        .session
        .select(&TreeSelectionRequest {
            view: publication.view,
            revision: SelectionRevision(1),
            seed: None,
            seed_revision: SelectionRevision(0),
            action: midnight_core::TreeSelectionAction::Toggle(first),
        })
        .unwrap();
    let request = request(&mut fixture.session, TreeSizeScope::Single(second));
    let captured = fixture
        .session
        .capture_sizes(&request, &Cancellation::default())
        .unwrap();
    assert_eq!(captured.targets.len(), 1);
    assert_eq!(captured.targets[0].row, second);
}
