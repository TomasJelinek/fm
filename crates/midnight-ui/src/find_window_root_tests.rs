use super::*;

fn root(path: &str, searched: bool) -> FindRoot {
    FindRoot {
        path: path.into(),
        searched,
    }
}

fn listed(model: &VecModel<FindRootRow>) -> Vec<(String, bool)> {
    model
        .iter()
        .map(|row| (row.label.to_string(), row.searched))
        .collect()
}

#[test]
fn ticks_update_their_rows_in_place_and_other_changes_replace_the_list() {
    let old = [root("/a", true), root("/b", true)];
    let model = VecModel::from(old.iter().map(root_row).collect::<Vec<_>>());
    // A mark on an untouched row: an in-place update keeps it, a new list would not.
    model.set_row_data(
        1,
        FindRootRow {
            label: "untouched".into(),
            searched: true,
        },
    );
    let ticked = [root("/a", false), root("/b", true)];
    update_roots(&model, &old, &ticked);
    assert_eq!(
        listed(&model),
        [("/a".into(), false), ("untouched".into(), true)]
    );
    let added = [root("/a", false), root("/b", true), root("/c", true)];
    update_roots(&model, &ticked, &added);
    assert_eq!(
        listed(&model),
        [
            ("/a".into(), false),
            ("/b".into(), true),
            ("/c".into(), true)
        ]
    );
    let removed = [root("/a", false), root("/c", true)];
    update_roots(&model, &added, &removed);
    assert_eq!(listed(&model), [("/a".into(), false), ("/c".into(), true)]);
}
