//! Ordering capture contract, shared with the standalone RED/GREEN harness.
use super::reorder;
use midnight_core::*;
use std::sync::Arc;

fn commands() -> Arc<Vec<UserCommand>> {
    Arc::new(
        ["a", "b", "c", "d"]
            .into_iter()
            .map(|id| UserCommand {
                id: UserCommandId::new(id).unwrap(),
                name: format!("Command {id} 文档"),
                enabled: id != "b",
                toolbar_label: Some(format!("Label {id}")),
                icon: None,
                working_directory: "${active.path}".into(),
                launch: UserCommandLaunch::Single,
                invocation: UserCommandInvocation::Program {
                    executable: std::env::temp_dir().join("program 文档.exe"),
                    arguments: vec!["".into(), "${file.path}".into()],
                },
            })
            .collect(),
    )
}
fn ids(commands: &[UserCommand]) -> Vec<&str> {
    commands.iter().map(|command| command.id.as_str()).collect()
}

#[test]
fn insertion_boundaries_move_both_directions_without_changing_command_data_or_source() {
    let source = commands();
    let drag = reorder::Drag::new(&source, 7, "a").unwrap();
    let moved = drag.order(&source, 7, false, 3).unwrap().unwrap();
    assert_eq!(ids(&moved), ["b", "c", "a", "d"]);
    assert_eq!(moved[2], source[0]);
    assert_eq!(ids(&source), ["a", "b", "c", "d"]);
    let drag = reorder::Drag::new(&source, 7, "d").unwrap();
    assert_eq!(
        ids(&drag.order(&source, 7, false, 0).unwrap().unwrap()),
        ["d", "a", "b", "c"]
    );
    let drag = reorder::Drag::new(&source, 7, "a").unwrap();
    assert_eq!(
        ids(&drag.order(&source, 7, false, 4).unwrap().unwrap()),
        ["b", "c", "d", "a"]
    );
}

#[test]
fn same_position_is_no_op_and_dirty_or_invalid_boundaries_cannot_reorder() {
    let source = commands();
    let drag = reorder::Drag::new(&source, 3, "b").unwrap();
    assert!(drag.order(&source, 3, false, 1).unwrap().is_none());
    assert!(drag.order(&source, 3, false, 2).unwrap().is_none());
    assert!(
        drag.order(&source, 3, true, 4)
            .unwrap_err()
            .contains("draft")
    );
    assert!(drag.order(&source, 3, false, usize::MAX).is_err());
    assert_eq!(ids(&source), ["a", "b", "c", "d"]);
}

#[test]
fn replaced_configuration_revision_or_deleted_identity_rejects_the_captured_drag() {
    let source = commands();
    let drag = reorder::Drag::new(&source, 3, "b").unwrap();
    let replacement = Arc::new(source.as_ref().clone());
    assert!(drag.order(&replacement, 3, false, 4).is_err());
    assert!(drag.order(&source, 4, false, 4).is_err());
    let deleted = Arc::new(
        source
            .iter()
            .filter(|c| c.id.as_str() != "b")
            .cloned()
            .collect(),
    );
    assert!(drag.order(&deleted, 4, false, 0).is_err());
    assert!(reorder::Drag::new(&source, 3, "missing").is_err());
}

#[test]
fn capture_rejects_ambiguous_identity_and_more_than_the_configured_row_budget() {
    let source = commands();
    let duplicate = Arc::new(vec![source[0].clone(), source[0].clone()]);
    assert!(reorder::Drag::new(&duplicate, 1, "a").is_err());
    let oversized = Arc::new(vec![source[0].clone(); MAX_USER_COMMANDS + 1]);
    assert!(reorder::Drag::new(&oversized, 1, "a").is_err());
}
