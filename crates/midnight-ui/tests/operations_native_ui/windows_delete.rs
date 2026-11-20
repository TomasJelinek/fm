//! A real Windows sharing refusal must reach the failed Delete's own progress window,
//! with no notice repeating it.
use super::*;
use midnight_ui::NoticePresentation;
use slint::Model;
use std::os::windows::fs::OpenOptionsExt;

#[test]
fn locked_directory_delete_reports_an_error_and_succeeds_after_release() {
    const SHARE_READ_WRITE: u32 = 0x1 | 0x2;
    let mut h = native::Harness::new();
    let target = h.left.join(".claude");
    fs::create_dir(&target).unwrap();
    let file = target.join("fixture.txt");
    fs::write(&file, b"locked owned fixture").unwrap();
    ui::chord(&h.ui.window, &[Key::Control], Key::F9);
    h.until(|h| {
        !h.ui.window.get_left_loading()
            && h.rows(PaneId::Left).iter().any(|row| row.name == ".claude")
    });
    h.focus(PaneId::Left, ".claude", false);
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(SHARE_READ_WRITE)
        .open(&file)
        .unwrap();

    let first = confirm_permanent_delete(&mut h);
    h.until(|h| {
        h.operations
            .iter()
            .any(|p| p.id == first && p.state.is_terminal())
    });
    h.settle();
    let failure = h
        .operations
        .iter()
        .find(|p| p.id == first && p.state.is_terminal())
        .unwrap();
    assert_eq!(failure.state, OperationState::Failed, "{failure:?}");
    assert_eq!(failure.kind, MutationKind::Delete(DeleteMode::Permanent));
    assert_eq!(
        (failure.completed, failure.failed, failure.processed_entries),
        (0, 1, 0)
    );
    assert_eq!(failure.errors.len(), 1, "{failure:?}");
    assert!(failure.errors[0].path.starts_with(&target), "{failure:?}");
    assert!(target.is_dir());
    assert_eq!(fs::read(&file).unwrap(), b"locked owned fixture");

    // The Delete's own progress window lists the error with its path and message.
    assert!(
        h.ui.window
            .global::<NoticePresentation>()
            .get_token()
            .is_empty(),
        "no notice repeats what the progress window shows"
    );
    let row =
        h.ui.window
            .get_operations()
            .iter()
            .find(|row| row.identity == first.0.to_string())
            .expect("the failed Delete is listed");
    assert_eq!(row.issue_summary, "1 error");
    let issue = row.issues.row_data(0).expect("its error is listed");
    assert!(issue.path.contains(".claude"), "{}", issue.path);
    assert!(
        issue.detail.contains(failure.errors[0].message.trim()),
        "{}",
        issue.detail
    );

    drop(locked);
    h.until(|h| !h.ui.window.get_left_loading());
    // Close the failed Delete's progress window.
    h.click_decision("Close");
    h.focus(PaneId::Left, ".claude", false);
    let second = confirm_permanent_delete(&mut h);
    h.until(|h| {
        h.operations
            .iter()
            .any(|p| p.id == second && p.state.is_terminal())
    });
    let success = h
        .operations
        .iter()
        .find(|p| p.id == second && p.state.is_terminal())
        .unwrap();
    assert_eq!(success.state, OperationState::Completed, "{success:?}");
    assert!(!target.exists());
    h.assert_sentinel();
}

fn confirm_permanent_delete(h: &mut native::Harness) -> OperationId {
    ui::chord(&h.ui.window, &[Key::Shift], Key::F8);
    h.until(|h| !h.ui.window.get_operation_token().is_empty());
    assert!(h.ui.window.get_operation_dangerous());
    assert!(h.ui.window.get_operation_body().contains(".claude"));
    let id = OperationId(h.ui.window.get_operation_token().parse().unwrap());
    input::key(&h.dialog(), Key::Return);
    id
}
