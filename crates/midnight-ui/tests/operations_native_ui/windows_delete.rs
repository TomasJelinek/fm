//! A real Windows sharing refusal pauses the permanent Delete in its decision prompt,
//! with no notice repeating it. Windows offers the whole item: Retry while the file is
//! still open asks again, and Retry once it is closed deletes the folder.
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
    // Without delete sharing, Windows refuses to delete the open file.
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(SHARE_READ_WRITE)
        .open(&file)
        .unwrap();

    let id = confirm_permanent_delete(&mut h);
    let first = prompt(&mut h, id, "");
    assert!(h.ui.window.get_copy_is_failure());
    assert!(h.ui.window.get_copy_can_retry() && h.ui.window.get_copy_can_skip());
    let body = h.ui.window.get_copy_body();
    for part in [
        "Deleting permanently",
        "(os error 32)",
        ".claude",
        "fixture.txt",
    ] {
        assert!(body.contains(part), "the prompt names {part:?}: {body}");
    }
    // The Delete waits: nothing is deleted or counted as an error yet. Its progress
    // says why once the next snapshot arrives.
    h.until(|h| !latest(h, id).message.is_empty());
    let waiting = latest(&h, id);
    assert_eq!(waiting.state, OperationState::Running, "{waiting:?}");
    assert_eq!(waiting.kind, MutationKind::Delete(DeleteMode::Permanent));
    assert_eq!(
        (waiting.completed, waiting.failed, waiting.processed_entries),
        (0, 0, 0)
    );
    assert!(waiting.errors.is_empty(), "{waiting:?}");
    assert!(waiting.message.contains("(os error 32)"), "{waiting:?}");
    assert!(target.is_dir());
    assert_eq!(fs::read(&file).unwrap(), b"locked owned fixture");
    // The Delete's own window says why it waits; no notice repeats it.
    let row =
        h.ui.window
            .get_operations()
            .iter()
            .find(|row| row.identity == id.0.to_string())
            .expect("the waiting Delete is listed");
    assert!(row.active);
    assert!(
        row.detail.contains(waiting.message.as_str()),
        "{}",
        row.detail
    );
    assert!(
        h.ui.window
            .global::<NoticePresentation>()
            .get_token()
            .is_empty(),
        "no notice repeats what the prompt shows"
    );

    // Retry repeats the whole item, which is still open: the Delete asks again.
    h.click_decision("Retry");
    prompt(&mut h, id, &first);
    assert!(target.is_dir());
    assert_eq!(latest(&h, id).state, OperationState::Running);

    drop(locked);
    h.click_decision("Retry");
    h.until(|h| latest(h, id).state.is_terminal());
    let success = latest(&h, id);
    assert_eq!(success.state, OperationState::Completed, "{success:?}");
    // A retried failure is not an error of the job.
    assert_eq!((success.completed, success.failed), (1, 0), "{success:?}");
    assert!(success.errors.is_empty(), "{success:?}");
    assert!(!target.exists());
    h.until(|h| !h.rows(PaneId::Left).iter().any(|row| row.name == ".claude"));
    assert!(
        h.ui.window
            .global::<NoticePresentation>()
            .get_token()
            .is_empty()
    );
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

/// Waits for the shared prompt to ask about a failure of the Delete `id` other than the
/// one `shown` names; returns its token.
fn prompt(h: &mut native::Harness, id: OperationId, shown: &str) -> String {
    let prefix = format!("delete-error:{}:", id.0);
    h.until(|h| {
        let token = h.ui.window.get_copy_token();
        token.starts_with(&prefix) && token != shown
    });
    h.ui.window.get_copy_token().into()
}

/// The newest progress of the job `id`.
fn latest(h: &native::Harness, id: OperationId) -> std::sync::Arc<OperationProgress> {
    h.operations
        .iter()
        .rev()
        .find(|p| p.id == id)
        .expect("the Delete reported progress")
        .clone()
}
