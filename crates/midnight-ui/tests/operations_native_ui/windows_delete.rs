//! A real Windows sharing refusal pauses the permanent Delete in its decision prompt,
//! with no notice repeating it. Windows asks about the open file itself, as Linux asks
//! about each entry it cannot remove: Retry while the file is still open asks again,
//! Retry once it is closed deletes the folder, and Skip keeps only that file with the
//! folders that contain it and deletes everything else.
use super::*;
use midnight_ui::NoticePresentation;
use slint::Model;
use std::{os::windows::fs::OpenOptionsExt, path::Path};

/// Share modes of a program that does not let its open file be deleted.
const SHARE_READ_WRITE: u32 = 0x1 | 0x2;

#[test]
fn locked_directory_delete_reports_an_error_and_succeeds_after_release() {
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

    // Retry repeats the step that failed; the file is still open, so the Delete asks
    // again.
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

#[test]
fn skip_keeps_only_the_open_file_with_its_folders_and_deletes_the_rest() {
    let mut h = native::Harness::new();
    let target = h.left.join(".claude");
    let nested = target.join("nested");
    fs::create_dir_all(&nested).unwrap();
    let file = nested.join("fixture.txt");
    fs::write(&file, b"locked owned fixture").unwrap();
    fs::write(nested.join("gone.txt"), b"deleted").unwrap();
    fs::write(target.join("other.txt"), b"deleted").unwrap();
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

    let id = confirm_permanent_delete(&mut h);
    prompt(&mut h, id, "");
    assert!(h.ui.window.get_copy_can_retry() && h.ui.window.get_copy_can_skip());
    let body = h.ui.window.get_copy_body();
    for part in ["(os error 32)", "nested", "fixture.txt"] {
        assert!(body.contains(part), "the prompt names {part:?}: {body}");
    }
    h.click_decision("Skip");
    h.until(|h| latest(h, id).state.is_terminal());
    drop(locked);
    let done = latest(&h, id);
    // The skipped file is the job's one error; the folder that keeps it is not deleted
    // and says so once, and everything else in it is gone.
    assert_eq!(done.state, OperationState::CompletedWithErrors, "{done:?}");
    assert_eq!((done.completed, done.failed), (0, 1), "{done:?}");
    assert_eq!(done.errors.len(), 1, "{done:?}");
    assert!(
        done.errors[0]
            .path
            .ends_with(Path::new(".claude").join("nested").join("fixture.txt")),
        "{done:?}"
    );
    let kept = "was not deleted because it still contains skipped items.";
    assert_eq!(done.message.matches(kept).count(), 1, "{}", done.message);
    assert_eq!(fs::read(&file).unwrap(), b"locked owned fixture");
    assert!(!nested.join("gone.txt").exists());
    assert!(!target.join("other.txt").exists());
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
