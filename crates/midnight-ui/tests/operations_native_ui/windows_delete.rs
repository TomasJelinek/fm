//! A real Windows sharing refusal must reach the embedded file-operation notice.
use super::*;
use midnight_ui::NoticePresentation;
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
            && !h
                .ui
                .window
                .global::<NoticePresentation>()
                .get_token()
                .is_empty()
    });
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

    let notice = h.ui.window.global::<NoticePresentation>();
    assert_eq!(notice.get_title().as_str(), "File operation failed");
    assert!(notice.get_error());
    let details = format!(
        "{}\n{}\n{}",
        notice.get_body(),
        notice.get_folder_path(),
        notice.get_details(),
    );
    assert!(details.contains(".claude"), "{details}");
    assert!(details.contains(&failure.errors[0].message), "{details}");

    drop(locked);
    notice.invoke_dismiss(notice.get_token());
    h.until(|h| {
        h.ui.window
            .global::<NoticePresentation>()
            .get_token()
            .is_empty()
            && !h.ui.window.get_left_loading()
    });
    // Closing the notice leaves the failed Delete's modeless progress available.
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
