//! Select and deselect by mask in loaded native and archive views.
use super::{tests::request, *};

fn entries() -> Vec<Entry> {
    [
        ("docs", EntryKind::Directory),
        ("notes.txt", EntryKind::File),
        ("README.MD", EntryKind::File),
        ("draft.txt", EntryKind::File),
        ("文档.txt", EntryKind::File),
        ("site.txt", EntryKind::DirectoryLink),
        ("image.png", EntryKind::File),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (path, kind))| Entry::new(EntryId(i as u64 + 100), path.into(), kind))
    .collect()
}

fn mask(pattern: &str, select: bool, folders: bool) -> SelectionAction {
    SelectionAction::Mask(Arc::new(MaskSelection {
        pattern: pattern.into(),
        select,
        folders,
    }))
}

fn bits(snapshot: &SelectionSnapshot) -> u64 {
    snapshot.bits()[0]
}

#[test]
fn masks_select_and_deselect_matching_rows_and_keep_every_other_mark() {
    let index = Arc::new(DirectoryIndex::from_entries(&entries()));
    let mut state = State::default();
    let apply = |state: &mut State, revision, action| {
        state
            .apply(&request(revision, Some(index.clone()), action))
            .unwrap()
            .unwrap()
            .0
    };
    // An earlier mark on a row the mask does not match survives.
    apply(
        &mut state,
        1,
        SelectionAction::Toggle(PathBuf::from("image.png")),
    );
    let files = apply(&mut state, 2, mask("*.txt;*.md|draft*", true, false));
    assert_eq!(
        bits(&files),
        0b101_0110,
        "files only, case ignored, exclusions kept"
    );
    let folders = apply(&mut state, 3, mask("*.txt", true, true));
    assert_eq!(
        bits(&folders),
        0b111_1110,
        "folders join only when chosen; a directory link counts as a folder"
    );
    let deselected = apply(&mut state, 4, mask("*.md;site*", false, false));
    assert_eq!(
        bits(&deselected),
        0b111_1010,
        "deselecting files leaves a matching folder marked"
    );
    let every = apply(&mut state, 5, mask("*", false, true));
    assert_eq!(bits(&every), 0);
}

#[test]
fn masks_need_a_complete_view_and_report_their_grammar_errors() {
    let index = Arc::new(DirectoryIndex::from_entries(&entries()));
    let mut state = State::default();
    assert_eq!(
        state
            .apply(&request(1, None, mask("*.txt", true, false)))
            .err(),
        Some("Wait until the folder has finished loading, then select by mask.")
    );
    let mut state = State::default();
    assert_eq!(
        state
            .apply(&request(1, Some(index), mask("*.a|*.b|*.c", true, false)))
            .err(),
        Some("Use at most one | before excluded masks.")
    );
}

#[test]
fn archive_masks_match_displayed_member_captions_only() {
    let rows = entries();
    // The view shows every member but the first file; it never changes.
    let shown: Vec<_> = rows
        .iter()
        .filter(|entry| entry.id != EntryId(101))
        .cloned()
        .collect();
    let view = DirectoryIndex::from_entries(&shown);
    let selected = std::collections::HashSet::from([EntryId(106)]);
    let result = crate::archive_selection::apply(
        &rows,
        &view,
        &selected,
        ArchiveSelectionAction::Mask(Arc::new(MaskSelection {
            pattern: "*.TXT".into(),
            select: true,
            folders: true,
        })),
    )
    .unwrap();
    assert_eq!(
        *result.selected,
        std::collections::HashSet::from([EntryId(103), EntryId(104), EntryId(105), EntryId(106)])
    );
    let result = crate::archive_selection::apply(
        &rows,
        &view,
        &result.selected,
        ArchiveSelectionAction::Mask(Arc::new(MaskSelection {
            pattern: "*.txt".into(),
            select: false,
            folders: false,
        })),
    )
    .unwrap();
    assert_eq!(
        *result.selected,
        std::collections::HashSet::from([EntryId(105), EntryId(106)])
    );
}
