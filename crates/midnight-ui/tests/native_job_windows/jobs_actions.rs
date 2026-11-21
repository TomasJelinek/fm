//! U-D14 of the UI polish gate, reported as "to confirm": in Jobs, Open and Copy details
//! seemed to do nothing for a finished Permanently delete job. Both act on it: Open
//! shows the finished job in a window of its own, and Copy details copies its details.
use super::outcome_notices::{deletion, shows};
use super::*;

fn apply_deletion(h: &Harness, progress: OperationProgress) {
    h.apply(Event::Operation(OperationEvent::Progress(Arc::new(
        progress,
    ))));
}

/// The text of each clipboard copy sent since the last clear.
fn copied(h: &Harness) -> Vec<String> {
    h.commands
        .borrow()
        .iter()
        .filter_map(|command| match command {
            Command::CopyText { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn jobs_open_and_copy_details_act_on_a_finished_permanent_delete() {
    let h = Harness::new();
    // A finished copy and a Permanently delete that finished on its own: its window
    // closed itself, as after the Delete dialog's Permanently delete.
    h.start(1);
    h.complete(1);
    h.children(0);
    let mut running = deletion(7, OperationState::Running, "");
    running.current_path = Some("source/report 文档.txt".into());
    apply_deletion(&h, running);
    h.children(1);
    let mut completed = deletion(7, OperationState::Completed, "Deleted 2 items.");
    completed.completed = 2;
    apply_deletion(&h, completed);
    h.children(0);
    let identity = "7";
    let job = h
        .main
        .get_operations()
        .iter()
        .find(|row| row.identity == identity)
        .expect("Jobs lists the finished delete");
    assert!(job.completed && !job.active, "the delete has finished");

    h.main.invoke_show_operations();
    let jobs = h.children(1).remove(0);
    input::click(&jobs, &job_row(&jobs, identity));
    h.children(1);
    // Open: the finished delete's own window, which says how it ended.
    modeless::click(&jobs, "Open");
    let children = h.children(2);
    assert!(Rc::ptr_eq(&jobs, &children[0]), "Jobs stays open");
    let progress = children[1].clone();
    assert!(shows(&progress, "Completed"), "Open shows the finished job");
    assert!(
        shows(&progress, &job.title),
        "the window is the delete's own"
    );
    // Copy details: the job's whole details go to the clipboard.
    h.commands.borrow_mut().clear();
    modeless::click(&jobs, "Copy details");
    assert_eq!(copied(&h), [job.detail.to_string()], "Copy details copies");
    assert!(!job.detail.is_empty(), "the delete has details to copy");
    // Open again only brings the open window forward; it opens no second one.
    modeless::click(&jobs, "Open");
    let children = h.children(2);
    assert!(Rc::ptr_eq(&progress, &children[1]));
    // Closed, it opens again.
    input::key(&progress, Key::Escape);
    h.children(1);
    modeless::click(&jobs, "Open");
    assert!(shows(&h.children(2)[1], "Completed"));
}
